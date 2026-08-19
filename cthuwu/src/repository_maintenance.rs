use crate::operator::ToolReceipt;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::{Instant, timeout},
};

const MANIFEST: &str = include_str!("../../repository-maintenance.json");
const MAX_COMMAND_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_RECEIPT_BYTES: usize = 12 * 1024;
const MAX_DIRTY_ENTRIES: usize = 128;
const MAX_REMOTE_COUNT: usize = 32;
const MAX_PATHS: usize = 64;
const MAX_DIVERGENT_COMMITS: usize = 40;
const MAX_BRANCH_CHARS: usize = 200;
const MAX_COMMIT_MESSAGE_CHARS: usize = 200;
const MAX_PR_TITLE_CHARS: usize = 256;
const MAX_PR_BODY_BYTES: usize = 8 * 1024;
// Cold locked installs and Rust tests can legitimately exceed an ordinary model-tool phase. The
// operation still shares the authenticated request deadline, and every child is killed with its
// process group at this per-command bound.
const COMMAND_LIMIT: Duration = Duration::from_secs(90);

#[derive(Clone, Copy)]
enum CommandCapability {
    Local,
    GitNetwork,
    GithubCli,
    Validation,
}

#[derive(Clone, Copy)]
enum ExecutableTrust {
    Authentication,
    Validation,
}

#[derive(Clone, Debug)]
pub struct RepositoryMaintenance {
    workspace_root: PathBuf,
    maximum_timeout: Duration,
    git_executable: Option<PathBuf>,
    gh_executable: Option<PathBuf>,
    authentication_path: OsString,
    validation_path: OsString,
    policy: RepositoryPolicy,
}

#[derive(Clone, Debug)]
struct RepositoryPolicy {
    canonical_owner: String,
    canonical_repository: String,
    canonical_url: String,
    default_base_branch: String,
    update_validation: Vec<ValidationId>,
    pr_validation: Vec<ValidationId>,
    restart: String,
    #[cfg(test)]
    canonical_local_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryManifest {
    schema: u32,
    canonical_owner: String,
    canonical_repository: String,
    canonical_url: String,
    default_branch: String,
    update_validation: Vec<ValidationId>,
    pr_validation: Vec<ValidationId>,
    source_update_does_not_restart: bool,
    restart: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum ValidationId {
    Submodules,
    AgentInstall,
    AgentTypecheck,
    AgentTest,
    AgentBuild,
    LauncherSmoke,
    LauncherTest,
    InstallTest,
    RustFmt,
    RustTest,
    RustClippy,
    RustBuild,
    WebInstall,
    WebTypecheck,
    WebTest,
    WebBuild,
    ForgeFmt,
    ForgeLint,
    ForgeBuild,
    ForgeTest,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum RepositoryMaintenanceRequest {
    Status,
    Fetch,
    Update,
    Merge {
        remote: String,
        branch: String,
    },
    Test {
        #[serde(default)]
        profile: TestProfile,
    },
    Build {
        #[serde(default)]
        profile: BuildProfile,
    },
    Commit {
        message: String,
        paths: Vec<String>,
        #[serde(default)]
        topic_branch: Option<String>,
    },
    Push {
        remote: String,
        branch: String,
    },
    Pr {
        branch: String,
        title: String,
        body: String,
        commit_message: String,
        paths: Vec<String>,
        #[serde(default)]
        base: Option<String>,
    },
}

impl RepositoryMaintenanceRequest {
    pub fn operation_name(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Fetch => "fetch",
            Self::Update => "update",
            Self::Merge { .. } => "merge",
            Self::Test { .. } => "test",
            Self::Build { .. } => "build",
            Self::Commit { .. } => "commit",
            Self::Push { .. } => "push",
            Self::Pr { .. } => "pr",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestProfile {
    Focused,
    #[default]
    Required,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildProfile {
    Runtime,
    #[default]
    Required,
}

#[derive(Clone, Debug, Serialize)]
struct RepositoryStatus {
    repository_root: String,
    head: String,
    branch: Option<String>,
    tracked_ref: Option<String>,
    ahead: Option<u64>,
    behind: Option<u64>,
    dirty: bool,
    dirty_entries: Vec<String>,
    dirty_entries_truncated: bool,
    topology: RepositoryTopology,
    canonical_remote: Option<String>,
    remotes: Vec<RemoteStatus>,
    git: CapabilityStatus,
    gh: GhStatus,
    source_update_note: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RepositoryTopology {
    Canonical,
    Fork,
    ForkMissingUpstream,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
struct RemoteStatus {
    name: String,
    fetch_url: String,
    push_url: String,
    fetch_repository: Option<String>,
    push_repository: Option<String>,
    identities_match: bool,
    canonical: bool,
    fetch_refspec_safe: bool,
    safe_for_network_maintenance: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RemoteInfo {
    name: String,
    fetch_url: String,
    push_url: String,
    fetch_identity: Option<RepositoryIdentity>,
    push_identity: Option<RepositoryIdentity>,
    fetch_refspec_safe: bool,
    safe_for_network_maintenance: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RepositoryIdentity {
    owner: String,
    repository: String,
}

impl RemoteInfo {
    fn verified_identity(&self) -> Option<&RepositoryIdentity> {
        (self.fetch_identity == self.push_identity)
            .then_some(self.fetch_identity.as_ref())
            .flatten()
    }
}

#[derive(Clone, Debug, Serialize)]
struct DivergentCommitSummary {
    local_hashes: Vec<String>,
    upstream_hashes: Vec<String>,
    local_count: u64,
    upstream_count: u64,
    truncated: bool,
}

#[derive(Default)]
struct SubmoduleConfig {
    path: Option<String>,
    url: Option<String>,
    update: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct CapabilityStatus {
    installed: bool,
    version: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct GhStatus {
    installed: bool,
    version: Option<String>,
    authenticated_for_github_com: bool,
}

#[derive(Clone, Debug)]
struct RepositorySnapshot {
    root: PathBuf,
    head: String,
    branch: Option<String>,
    tracked_ref: Option<String>,
    ahead: Option<u64>,
    behind: Option<u64>,
    dirty_entries: Vec<String>,
    dirty_entries_truncated: bool,
    topology: RepositoryTopology,
    canonical_remote: Option<String>,
    remotes: Vec<RemoteInfo>,
    git_version: String,
}

#[derive(Clone, Debug, Serialize)]
struct StepReceipt {
    step: String,
    ok: bool,
    summary: String,
}

#[derive(Debug)]
struct CommandResult {
    success: bool,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
    truncated: bool,
}

impl RepositoryPolicy {
    fn load() -> Result<Self> {
        let manifest: RepositoryManifest = serde_json::from_str(MANIFEST)
            .context("parsing embedded repository maintenance manifest")?;
        ensure!(
            manifest.schema == 1,
            "unsupported repository maintenance manifest schema"
        );
        ensure!(
            manifest.canonical_owner == "pierce403"
                && manifest.canonical_repository == "cthuwu"
                && manifest.canonical_url == "https://github.com/pierce403/cthuwu.git"
                && manifest.default_branch == "main",
            "embedded repository maintenance manifest does not match this Cthuwu release"
        );
        ensure!(
            manifest.source_update_does_not_restart,
            "maintenance manifest must distinguish source and running process"
        );
        validate_branch_name(&manifest.default_branch)?;
        ensure!(
            !manifest.update_validation.is_empty(),
            "update validation may not be empty"
        );
        ensure!(
            !manifest.pr_validation.is_empty(),
            "PR validation may not be empty"
        );
        let prescribed = [
            ValidationId::Submodules,
            ValidationId::AgentInstall,
            ValidationId::AgentTypecheck,
            ValidationId::AgentTest,
            ValidationId::AgentBuild,
            ValidationId::LauncherSmoke,
            ValidationId::LauncherTest,
            ValidationId::InstallTest,
            ValidationId::RustFmt,
            ValidationId::RustTest,
            ValidationId::RustClippy,
            ValidationId::RustBuild,
            ValidationId::WebInstall,
            ValidationId::WebTypecheck,
            ValidationId::WebTest,
            ValidationId::WebBuild,
            ValidationId::ForgeFmt,
            ValidationId::ForgeLint,
            ValidationId::ForgeBuild,
            ValidationId::ForgeTest,
        ];
        for (label, ids) in [
            ("update", &manifest.update_validation),
            ("pull-request", &manifest.pr_validation),
        ] {
            ensure!(
                ids.iter()
                    .enumerate()
                    .all(|(index, id)| !ids[..index].contains(id)),
                "{label} validation contains duplicate IDs"
            );
            ensure!(
                prescribed.iter().all(|required| ids.contains(required)),
                "{label} validation omits a repository-prescribed command ID"
            );
        }
        ensure!(
            manifest.restart == "stop the running process cleanly, then relaunch ./uwu.sh",
            "restart policy is not an allowlisted Cthuwu mechanism"
        );
        Ok(Self {
            canonical_owner: manifest.canonical_owner,
            canonical_repository: manifest.canonical_repository,
            canonical_url: manifest.canonical_url,
            default_base_branch: manifest.default_branch,
            update_validation: manifest.update_validation,
            pr_validation: manifest.pr_validation,
            restart: manifest.restart,
            #[cfg(test)]
            canonical_local_path: None,
        })
    }
}

impl RepositoryMaintenance {
    pub fn new(workspace_root: &Path, maximum_timeout_seconds: u64) -> Result<Self> {
        let workspace_root = fs::canonicalize(workspace_root).with_context(|| {
            format!(
                "resolving repository-maintenance workspace {}",
                workspace_root.display()
            )
        })?;
        ensure!(
            workspace_root.is_dir(),
            "operator workspace must be a directory"
        );
        ensure!(
            (1..=300).contains(&maximum_timeout_seconds),
            "repository-maintenance timeout must be between 1 and 300 seconds"
        );
        let executable_path = std::env::var_os("PATH").unwrap_or_default();
        let git_executable = resolve_executable_in_path(
            "git",
            &workspace_root,
            &executable_path,
            ExecutableTrust::Authentication,
        )?;
        let gh_executable = resolve_executable_in_path(
            "gh",
            &workspace_root,
            &executable_path,
            ExecutableTrust::Authentication,
        )?;
        let pinned_authentication_executables = git_executable
            .iter()
            .chain(gh_executable.iter())
            .cloned()
            .collect::<Vec<_>>();
        let authentication_path = sanitized_command_path(
            &workspace_root,
            &executable_path,
            ExecutableTrust::Authentication,
            &pinned_authentication_executables,
        )?;
        let validation_path = sanitized_command_path(
            &workspace_root,
            &executable_path,
            ExecutableTrust::Validation,
            &[],
        )?;
        Ok(Self {
            workspace_root,
            maximum_timeout: Duration::from_secs(maximum_timeout_seconds),
            git_executable,
            gh_executable,
            authentication_path,
            validation_path,
            policy: RepositoryPolicy::load()?,
        })
    }

    pub async fn execute(&self, request: RepositoryMaintenanceRequest) -> ToolReceipt {
        let operation = request.operation_name();
        let deadline = Instant::now() + self.maximum_timeout;
        let result = match request {
            RepositoryMaintenanceRequest::Status => self.status_receipt(deadline).await,
            RepositoryMaintenanceRequest::Fetch => self.fetch_receipt(deadline).await,
            RepositoryMaintenanceRequest::Update => self.update_receipt(deadline, true).await,
            RepositoryMaintenanceRequest::Merge { remote, branch } => {
                self.merge_receipt(deadline, &remote, &branch).await
            }
            RepositoryMaintenanceRequest::Test { profile } => {
                self.validation_receipt(deadline, ValidationKind::Test(profile))
                    .await
            }
            RepositoryMaintenanceRequest::Build { profile } => {
                self.validation_receipt(deadline, ValidationKind::Build(profile))
                    .await
            }
            RepositoryMaintenanceRequest::Commit {
                message,
                paths,
                topic_branch,
            } => {
                self.commit_receipt(deadline, &message, &paths, topic_branch.as_deref())
                    .await
            }
            RepositoryMaintenanceRequest::Push { remote, branch } => {
                self.push_receipt(deadline, &remote, &branch).await
            }
            RepositoryMaintenanceRequest::Pr {
                branch,
                title,
                body,
                commit_message,
                paths,
                base,
            } => {
                self.pr_receipt(
                    deadline,
                    &branch,
                    &title,
                    &body,
                    &commit_message,
                    &paths,
                    base.as_deref(),
                )
                .await
            }
        };
        match result {
            Ok(receipt) => receipt,
            Err(error) => ToolReceipt {
                tool: "repository_maintenance".to_owned(),
                ok: false,
                summary: sanitize_text(&format!("{operation} refused or failed: {error:#}"), 1024),
                output: String::new(),
                exit_code: None,
                timed_out: Instant::now() >= deadline,
                truncated: false,
            },
        }
    }

    async fn status_receipt(&self, deadline: Instant) -> Result<ToolReceipt> {
        let snapshot = self.snapshot(deadline).await?;
        let gh = self.gh_status(&snapshot.root, deadline).await;
        let status = snapshot.to_status(gh, &self.policy);
        Ok(json_receipt(
            true,
            "inspected the bounded Git workspace and sanitized GitHub capability state",
            &status,
        ))
    }

    async fn fetch_receipt(&self, deadline: Instant) -> Result<ToolReceipt> {
        let mut snapshot = self.snapshot(deadline).await?;
        ensure_clean(&snapshot)?;
        let canonical_remote = self
            .ensure_canonical_remote(&mut snapshot, deadline)
            .await?;
        let mut names = BTreeSet::new();
        if snapshot
            .remotes
            .iter()
            .any(|remote| remote.name == "origin")
        {
            names.insert("origin".to_owned());
        }
        names.insert(canonical_remote);
        let mut steps = Vec::new();
        for remote in &names {
            self.ensure_safe_remote(&snapshot, remote)?;
        }
        for remote in names {
            let result = self
                .git_network(
                    &snapshot.root,
                    ["fetch", "--prune", "--", remote.as_str()],
                    deadline,
                )
                .await?;
            let summary = command_summary(&result);
            steps.push(StepReceipt {
                step: format!("fetch {remote}"),
                ok: result.success,
                summary: summary.clone(),
            });
            ensure!(result.success, "fetching {remote} failed: {summary}");
        }
        json_receipt_value(
            true,
            "fetched configured fork and canonical remotes without changing the checked-out branch",
            json!({"steps": steps}),
        )
    }

    async fn update_receipt(&self, deadline: Instant, run_validation: bool) -> Result<ToolReceipt> {
        let initial = self.snapshot(deadline).await?;
        ensure_clean(&initial)?;
        let branch = initial
            .branch
            .clone()
            .context("detached HEAD cannot be updated automatically")?;
        validate_branch_name(&branch)?;
        let old_head = initial.head.clone();
        let mut snapshot = initial;
        let canonical_remote = self
            .ensure_canonical_remote(&mut snapshot, deadline)
            .await?;

        let mut fetch_remotes = BTreeSet::new();
        fetch_remotes.insert(canonical_remote.clone());
        if snapshot.topology != RepositoryTopology::Canonical
            && snapshot
                .remotes
                .iter()
                .any(|remote| remote.name == "origin")
        {
            fetch_remotes.insert("origin".to_owned());
        }
        let mut steps = Vec::new();
        for remote in &fetch_remotes {
            self.ensure_safe_remote(&snapshot, remote)?;
        }
        for remote in fetch_remotes {
            let result = self
                .git_network(
                    &snapshot.root,
                    ["fetch", "--prune", "--", remote.as_str()],
                    deadline,
                )
                .await?;
            steps.push(StepReceipt {
                step: format!("fetch {remote}"),
                ok: result.success,
                summary: command_summary(&result),
            });
            ensure!(result.success, "fetching {remote} failed");
        }

        snapshot = self.snapshot(deadline).await?;
        ensure_clean(&snapshot)?;
        let target_branch = self
            .corresponding_upstream_branch(&snapshot.root, &canonical_remote, &branch, deadline)
            .await?;
        let target = format!("{canonical_remote}/{target_branch}");
        validate_remote_ref(&canonical_remote, &target_branch)?;
        let (ahead, behind) = self
            .divergence(&snapshot.root, "HEAD", &target, deadline)
            .await?;
        let merge_base = self
            .git_stdout(
                &snapshot.root,
                ["merge-base", "HEAD", target.as_str()],
                deadline,
            )
            .await?
            .trim()
            .to_owned();
        let divergent_commits = self
            .divergent_commits(&snapshot.root, &target, ahead, behind, deadline)
            .await?;
        let topology = snapshot.topology;
        let mut changed = false;

        if behind > 0 {
            let result = if topology == RepositoryTopology::Canonical {
                if ahead > 0 {
                    return json_receipt_value(
                        false,
                        "canonical checkout diverged; intentional local commits were preserved and no merge or history rewrite was attempted",
                        json!({
                            "oldCommit": old_head,
                            "target": target,
                            "mergeBase": merge_base,
                            "aheadBefore": ahead,
                            "behindBefore": behind,
                            "divergentCommits": divergent_commits,
                            "runningProcessUpdated": false,
                            "nextAction": "Choose an explicit safe merge/rebase policy for the local commits, then invoke the corresponding typed maintenance operation."
                        }),
                    );
                }
                self.git(
                    &snapshot.root,
                    ["merge", "--ff-only", "--", target.as_str()],
                    deadline,
                )
                .await?
            } else {
                self.git(
                    &snapshot.root,
                    ["merge", "--no-edit", "--", target.as_str()],
                    deadline,
                )
                .await?
            };
            steps.push(StepReceipt {
                step: if topology == RepositoryTopology::Canonical {
                    format!("fast-forward {target}")
                } else {
                    format!("merge {target}")
                },
                ok: result.success,
                summary: command_summary(&result),
            });
            if !result.success {
                let (conflicts, conflicts_truncated) =
                    self.conflicted_paths(&snapshot.root, deadline).await?;
                return json_receipt_value(
                    false,
                    "upstream integration stopped on conflicts; no conflict was auto-resolved or discarded",
                    json!({
                        "oldCommit": old_head,
                        "target": target,
                        "mergeBase": merge_base,
                        "aheadBefore": ahead,
                        "behindBefore": behind,
                        "divergentCommits": divergent_commits,
                        "conflictedFiles": conflicts,
                        "conflictedFilesTruncated": conflicts_truncated,
                        "steps": steps,
                        "nextAction": "Resolve each conflict deliberately, stage only resolved files, run repository validation, and continue the merge commit. The maintenance workflow did not abort or replace either side."
                    }),
                );
            }
            changed = true;
        }

        let integrated = self.snapshot(deadline).await?;
        let mut current = integrated.clone();
        let mut validated = !changed;
        if changed && run_validation {
            let validation = self
                .run_validation_ids(deadline, self.policy.update_validation.clone(), true)
                .await?;
            validated = validation.iter().all(|step| step.ok);
            steps.extend(validation);
            current = self.snapshot(deadline).await?;
            if validated {
                let state = ensure_post_validation_state(&integrated, &current);
                let state_summary = match &state {
                    Ok(()) => {
                        "HEAD, branch, clean tree, remotes, and topology reverified".to_owned()
                    }
                    Err(error) => error.to_string(),
                };
                steps.push(StepReceipt {
                    step: "post-validation repository state".to_owned(),
                    ok: state.is_ok(),
                    summary: state_summary,
                });
                validated = state.is_ok();
            }
        }

        let mut pushed = false;
        if changed && topology != RepositoryTopology::Canonical && validated {
            self.ensure_operator_fork_remote(&current, "origin")?;
            let push_ref = format!("HEAD:refs/heads/{branch}");
            let push = self
                .git_network(
                    &current.root,
                    ["push", "--porcelain", "--", "origin", push_ref.as_str()],
                    deadline,
                )
                .await?;
            steps.push(StepReceipt {
                step: format!("push fork branch origin/{branch}"),
                ok: push.success,
                summary: command_summary(&push),
            });
            pushed = push.success;
        }

        let ok = !changed || (validated && (topology == RepositoryTopology::Canonical || pushed));
        let summary = if !changed && ahead > 0 {
            "no canonical commits were missing; intentional local commits were preserved without publication or history rewriting"
        } else if !changed {
            "repository was already current; no source or running process changed"
        } else if ok {
            "updated and validated source; the currently running process remains the old executable until a deliberate restart"
        } else {
            "source integration completed, but validation or fork publication failed; the running process remains unchanged"
        };
        json_receipt_value(
            ok,
            summary,
            json!({
                "topology": topology,
                "branch": branch,
                "target": target,
                "mergeBase": merge_base,
                "aheadBefore": ahead,
                "behindBefore": behind,
                "divergentCommits": divergent_commits,
                "oldCommit": old_head,
                "newCommit": current.head,
                "sourceChanged": changed,
                "validationPassed": validated,
                "forkPushed": pushed,
                "steps": steps,
                "runningProcessUpdated": false,
                "restart": if changed {
                    format!("Source/build state is newer, but this process is still the pre-update binary. {}", self.policy.restart)
                } else {
                    "No restart is required because the source commit did not change.".to_owned()
                }
            }),
        )
    }

    async fn merge_receipt(
        &self,
        deadline: Instant,
        remote: &str,
        branch: &str,
    ) -> Result<ToolReceipt> {
        validate_remote_name(remote)?;
        validate_branch_name(branch)?;
        let initial = self.snapshot(deadline).await?;
        ensure_clean(&initial)?;
        self.ensure_canonical_upstream_remote(&initial, remote)?;
        let fetch = self
            .git_network(&initial.root, ["fetch", "--prune", "--", remote], deadline)
            .await?;
        ensure!(
            fetch.success,
            "fetching the verified canonical merge remote failed"
        );
        let snapshot = self.snapshot(deadline).await?;
        ensure_clean(&snapshot)?;
        ensure!(
            snapshot.head == initial.head && snapshot.branch == initial.branch,
            "canonical fetch changed HEAD or the checked-out branch; merge refused"
        );
        ensure!(
            snapshot.remotes == initial.remotes
                && snapshot.topology == initial.topology
                && snapshot.canonical_remote == initial.canonical_remote,
            "repository remote identity or topology changed during canonical fetch; merge refused"
        );
        self.ensure_canonical_upstream_remote(&snapshot, remote)?;
        let target = format!("{remote}/{branch}");
        let target_ref = format!("refs/remotes/{remote}/{branch}");
        let target_exists = self
            .git(
                &snapshot.root,
                ["show-ref", "--verify", "--quiet", target_ref.as_str()],
                deadline,
            )
            .await?;
        ensure!(
            target_exists.success,
            "verified canonical remote does not expose the requested branch"
        );
        let (ahead, behind) = self
            .divergence(&snapshot.root, "HEAD", &target, deadline)
            .await?;
        let merge_base = self
            .git_stdout(
                &snapshot.root,
                ["merge-base", "HEAD", target.as_str()],
                deadline,
            )
            .await?;
        let divergent_commits = self
            .divergent_commits(&snapshot.root, &target, ahead, behind, deadline)
            .await?;
        let result = self
            .git(
                &snapshot.root,
                ["merge", "--no-edit", "--", target.as_str()],
                deadline,
            )
            .await?;
        let (conflicts, conflicts_truncated) = if result.success {
            (Vec::new(), false)
        } else {
            self.conflicted_paths(&snapshot.root, deadline).await?
        };
        json_receipt_value(
            result.success,
            if result.success {
                "merged the selected remote branch without rewriting history"
            } else {
                "merge stopped with conflicts; both sides and the in-progress merge were preserved"
            },
            json!({
                "target": target,
                "mergeBase": merge_base.trim(),
                "aheadBefore": ahead,
                "behindBefore": behind,
                "divergentCommits": divergent_commits,
                "conflictedFiles": conflicts,
                "conflictedFilesTruncated": conflicts_truncated,
                "fetch": command_summary(&fetch),
                "command": command_summary(&result),
            }),
        )
    }

    async fn validation_receipt(
        &self,
        deadline: Instant,
        kind: ValidationKind,
    ) -> Result<ToolReceipt> {
        let snapshot = self.snapshot(deadline).await?;
        let ids = validation_ids(kind);
        // Focused validation is expected after an operator-authorized source edit. It must not
        // require a clean tree; the commands are compiled and do not discard local work.
        let steps = self.run_validation_ids(deadline, ids, false).await?;
        let ok = !steps.is_empty() && steps.iter().all(|step| step.ok);
        json_receipt_value(
            ok,
            if ok {
                "repository validation completed successfully"
            } else {
                "repository validation stopped at the first failed or timed-out bounded step"
            },
            json!({"head": snapshot.head, "steps": steps}),
        )
    }

    async fn commit_receipt(
        &self,
        deadline: Instant,
        message: &str,
        paths: &[String],
        topic_branch: Option<&str>,
    ) -> Result<ToolReceipt> {
        validate_commit_message(message)?;
        let mut snapshot = self.snapshot(deadline).await?;
        ensure!(
            !paths.is_empty(),
            "commit requires at least one explicit path"
        );
        let paths = validate_scoped_paths(&snapshot.root, paths)?;
        ensure_dirty_is_scoped(&snapshot, &paths)?;
        if let Some(branch) = topic_branch {
            validate_branch_name(branch)?;
            snapshot = self
                .ensure_topic_branch(snapshot, branch, true, deadline)
                .await?;
        }
        self.stage_paths(&snapshot.root, &paths, deadline).await?;
        let (staged, staged_truncated) = self.staged_paths(&snapshot.root, deadline).await?;
        ensure!(
            !staged.is_empty(),
            "explicit paths produced no staged change"
        );
        ensure_staged_is_scoped(&staged, staged_truncated, &paths)?;
        let result = self
            .git(
                &snapshot.root,
                ["commit", "--no-gpg-sign", "-m", message],
                deadline,
            )
            .await?;
        let current = self.snapshot(deadline).await?;
        json_receipt_value(
            result.success,
            if result.success {
                "committed only the explicitly staged repository paths"
            } else {
                "commit failed; staged work was preserved"
            },
            json!({
                "branch": current.branch,
                "commit": current.head,
                "stagedPaths": staged,
                "command": command_summary(&result),
            }),
        )
    }

    async fn push_receipt(
        &self,
        deadline: Instant,
        remote: &str,
        branch: &str,
    ) -> Result<ToolReceipt> {
        validate_remote_name(remote)?;
        validate_branch_name(branch)?;
        let snapshot = self.snapshot(deadline).await?;
        ensure_clean(&snapshot)?;
        self.ensure_operator_fork_remote(&snapshot, remote)?;
        let current_branch = snapshot
            .branch
            .as_deref()
            .context("detached HEAD cannot be pushed by maintenance")?;
        ensure!(
            current_branch == branch,
            "push branch must equal the checked-out branch"
        );
        let validation = self
            .run_validation_ids(deadline, self.policy.pr_validation.clone(), true)
            .await?;
        ensure!(
            validation.iter().all(|step| step.ok),
            "push validation failed; the local branch remains unpushed"
        );
        let fresh = self.snapshot(deadline).await?;
        ensure_post_validation_state(&snapshot, &fresh)?;
        self.ensure_operator_fork_remote(&fresh, remote)?;
        let push_ref = format!("HEAD:refs/heads/{branch}");
        let result = self
            .git_network(
                &fresh.root,
                ["push", "--porcelain", "--", remote, push_ref.as_str()],
                deadline,
            )
            .await?;
        json_receipt_value(
            result.success,
            if result.success {
                "pushed the current branch without force"
            } else {
                "push failed; no force or history rewrite was attempted"
            },
            json!({
                "remote": remote,
                "branch": branch,
                "commit": fresh.head,
                "validation": validation,
                "command": command_summary(&result),
            }),
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn pr_receipt(
        &self,
        deadline: Instant,
        branch: &str,
        title: &str,
        body: &str,
        commit_message: &str,
        paths: &[String],
        base: Option<&str>,
    ) -> Result<ToolReceipt> {
        validate_branch_name(branch)?;
        ensure!(
            branch != self.policy.default_base_branch,
            "pull requests require a topic branch, not the default branch"
        );
        validate_pr_text(title, body)?;
        validate_commit_message(commit_message)?;
        let base = base.unwrap_or(&self.policy.default_base_branch);
        validate_branch_name(base)?;
        let mut snapshot = self.snapshot(deadline).await?;
        self.ensure_operator_fork_remote(&snapshot, "origin")?;
        let paths = validate_scoped_paths(&snapshot.root, paths)?;
        ensure!(
            !paths.is_empty(),
            "pull requests require a nonempty explicit path scope"
        );
        ensure_dirty_is_scoped(&snapshot, &paths)?;
        let canonical_remote = snapshot
            .canonical_remote
            .clone()
            .context("pull-request workflow requires a verified canonical upstream remote")?;
        self.ensure_canonical_upstream_remote(&snapshot, &canonical_remote)?;
        let before_fetch = snapshot.clone();
        let fetch = self
            .git_network(
                &snapshot.root,
                ["fetch", "--prune", "--", canonical_remote.as_str()],
                deadline,
            )
            .await?;
        ensure!(
            fetch.success,
            "fetching the verified canonical pull-request base failed"
        );
        snapshot = self.snapshot(deadline).await?;
        ensure!(
            snapshot.head == before_fetch.head
                && snapshot.branch == before_fetch.branch
                && snapshot.dirty_entries == before_fetch.dirty_entries
                && snapshot.dirty_entries_truncated == before_fetch.dirty_entries_truncated,
            "canonical-base fetch changed local HEAD, branch, or working-tree state; pull-request preparation refused"
        );
        ensure!(
            snapshot.remotes == before_fetch.remotes
                && snapshot.topology == before_fetch.topology
                && snapshot.canonical_remote == before_fetch.canonical_remote,
            "repository remote identity or topology changed during canonical-base fetch; pull-request preparation refused"
        );
        self.ensure_operator_fork_remote(&snapshot, "origin")?;
        self.ensure_canonical_upstream_remote(&snapshot, &canonical_remote)?;
        let canonical_base = format!("{canonical_remote}/{base}");
        let canonical_base_ref = format!("refs/remotes/{canonical_remote}/{base}");
        let base_exists = self
            .git(
                &snapshot.root,
                [
                    "show-ref",
                    "--verify",
                    "--quiet",
                    canonical_base_ref.as_str(),
                ],
                deadline,
            )
            .await?;
        ensure!(
            base_exists.success,
            "verified canonical upstream does not expose the requested PR base branch"
        );
        let (existing_paths, existing_paths_truncated) = self
            .changed_paths_between(&snapshot.root, &canonical_base, "HEAD", deadline)
            .await?;
        ensure_pr_diff_is_scoped(&existing_paths, existing_paths_truncated, &paths, false)?;
        let reuse_prepared_head = snapshot.branch.as_deref() == Some(branch)
            && snapshot.dirty_entries.is_empty()
            && !existing_paths.is_empty();
        ensure!(
            reuse_prepared_head || !snapshot.dirty_entries.is_empty(),
            "clean pull-request retry requires the requested topic branch to be currently checked out with a nonempty fully scoped canonical-base diff"
        );
        let mut steps = vec![StepReceipt {
            step: format!("fetch canonical PR base {canonical_remote}/{base}"),
            ok: true,
            summary: command_summary(&fetch),
        }];
        if reuse_prepared_head {
            steps.push(StepReceipt {
                step: "reuse prepared topic HEAD".to_owned(),
                ok: true,
                summary:
                    "clean current topic branch already has a nonempty fully scoped canonical-base diff; no commit was created"
                        .to_owned(),
            });
        } else {
            snapshot = self
                .ensure_topic_branch(snapshot, branch, true, deadline)
                .await?;
            self.stage_paths(&snapshot.root, &paths, deadline).await?;
            let (staged, staged_truncated) = self.staged_paths(&snapshot.root, deadline).await?;
            ensure!(
                !staged.is_empty(),
                "explicit PR paths produced no staged change"
            );
            ensure_staged_is_scoped(&staged, staged_truncated, &paths)?;
            let commit = self
                .git(
                    &snapshot.root,
                    ["commit", "--no-gpg-sign", "-m", commit_message],
                    deadline,
                )
                .await?;
            steps.push(StepReceipt {
                step: "commit scoped change".to_owned(),
                ok: commit.success,
                summary: command_summary(&commit),
            });
            ensure!(commit.success, "scoped pull-request commit failed");
        }

        let prepared = self.snapshot(deadline).await?;
        ensure_clean(&prepared)?;
        ensure!(
            prepared.branch.as_deref() == Some(branch),
            "prepared pull-request branch changed unexpectedly"
        );
        let (prepared_paths, prepared_paths_truncated) = self
            .changed_paths_between(&prepared.root, &canonical_base, "HEAD", deadline)
            .await?;
        ensure_pr_diff_is_scoped(&prepared_paths, prepared_paths_truncated, &paths, true)?;

        let validation = self
            .run_validation_ids(deadline, self.policy.pr_validation.clone(), true)
            .await?;
        let validated = validation.iter().all(|step| step.ok);
        steps.extend(validation);
        ensure!(
            validated,
            "pull-request validation failed; prepared branch and commit were preserved"
        );

        snapshot = self.snapshot(deadline).await?;
        ensure_post_validation_state(&prepared, &snapshot)?;
        self.ensure_canonical_upstream_remote(&snapshot, &canonical_remote)?;
        let before_publication_fetch = snapshot.clone();
        let publication_fetch = self
            .git_network(
                &snapshot.root,
                ["fetch", "--prune", "--", canonical_remote.as_str()],
                deadline,
            )
            .await?;
        ensure!(
            publication_fetch.success,
            "refreshing the verified canonical PR base before publication failed"
        );
        snapshot = self.snapshot(deadline).await?;
        ensure!(
            snapshot.head == before_publication_fetch.head
                && snapshot.branch == before_publication_fetch.branch,
            "canonical-base refresh changed local HEAD or branch; pull-request publication refused"
        );
        ensure_clean(&snapshot)?;
        ensure!(
            snapshot.remotes == before_publication_fetch.remotes
                && snapshot.topology == before_publication_fetch.topology
                && snapshot.canonical_remote == before_publication_fetch.canonical_remote,
            "repository remote identity or topology changed during canonical-base publication refresh"
        );
        self.ensure_canonical_upstream_remote(&snapshot, &canonical_remote)?;
        let publication_base_exists = self
            .git(
                &snapshot.root,
                [
                    "show-ref",
                    "--verify",
                    "--quiet",
                    canonical_base_ref.as_str(),
                ],
                deadline,
            )
            .await?;
        ensure!(
            publication_base_exists.success,
            "verified canonical upstream no longer exposes the requested PR base branch"
        );
        let (publication_paths, publication_paths_truncated) = self
            .changed_paths_between(&snapshot.root, &canonical_base, "HEAD", deadline)
            .await?;
        ensure_pr_diff_is_scoped(
            &publication_paths,
            publication_paths_truncated,
            &paths,
            true,
        )?;
        steps.push(StepReceipt {
            step: format!("refresh canonical PR base {canonical_remote}/{base}"),
            ok: true,
            summary: command_summary(&publication_fetch),
        });
        let origin = self.ensure_operator_fork_remote(&snapshot, "origin")?;
        let origin_identity = origin
            .verified_identity()
            .context("origin is not a supported GitHub repository URL")?;
        let gh = self.gh_status(&snapshot.root, deadline).await;
        ensure!(
            gh.installed,
            "gh is not installed; prepared topic branch and commit remain local and unpushed"
        );
        ensure!(
            gh.authenticated_for_github_com,
            "gh is installed but not authenticated for github.com; prepared topic branch and commit remain local and unpushed"
        );
        let push_ref = format!("HEAD:refs/heads/{branch}");
        let push = self
            .git_network(
                &snapshot.root,
                [
                    "push",
                    "--porcelain",
                    "--set-upstream",
                    "--",
                    "origin",
                    push_ref.as_str(),
                ],
                deadline,
            )
            .await?;
        steps.push(StepReceipt {
            step: format!("push origin/{branch}"),
            ok: push.success,
            summary: command_summary(&push),
        });
        ensure!(
            push.success,
            "topic branch push failed; prepared branch and commit remain local"
        );

        let post_push = self.snapshot(deadline).await?;
        ensure!(
            post_push.head == snapshot.head
                && post_push.branch == snapshot.branch
                && post_push.topology == snapshot.topology
                && post_push.canonical_remote == snapshot.canonical_remote
                && post_push.remotes == snapshot.remotes,
            "repository identity changed after push; gh pull-request creation refused"
        );
        ensure_clean(&post_push)?;

        let head = if origin_identity
            .owner
            .eq_ignore_ascii_case(&self.policy.canonical_owner)
        {
            branch.to_owned()
        } else {
            format!("{}:{branch}", origin_identity.owner)
        };
        let canonical_slug = format!(
            "{}/{}",
            self.policy.canonical_owner, self.policy.canonical_repository
        );
        let gh_executable = self
            .gh_executable
            .as_deref()
            .context("gh is not installed as a trusted absolute executable")?;
        let gh_result = self
            .run_program(
                &post_push.root,
                gh_executable,
                [
                    "pr",
                    "create",
                    "--repo",
                    canonical_slug.as_str(),
                    "--base",
                    base,
                    "--head",
                    head.as_str(),
                    "--title",
                    title,
                    "--body",
                    body,
                ],
                deadline,
                CommandCapability::GithubCli,
            )
            .await?;
        let pr_url = extract_pr_url(
            &String::from_utf8_lossy(&gh_result.stdout),
            &self.policy.canonical_owner,
            &self.policy.canonical_repository,
        );
        let successful = gh_result.success && pr_url.is_some();
        steps.push(StepReceipt {
            step: "create upstream pull request with gh".to_owned(),
            ok: successful,
            summary: if successful {
                "gh returned a verified canonical pull-request URL".to_owned()
            } else {
                command_summary(&gh_result)
            },
        });
        json_receipt_value(
            successful,
            if successful {
                "created a real upstream pull request and verified its gh receipt"
            } else {
                "gh did not return a successful canonical pull-request receipt; no PR is claimed"
            },
            json!({
                "branch": branch,
                "base": base,
                "commit": post_push.head,
                "pullRequestUrl": pr_url,
                "steps": steps,
            }),
        )
    }

    async fn run_validation_ids(
        &self,
        deadline: Instant,
        ids: Vec<ValidationId>,
        require_clean: bool,
    ) -> Result<Vec<StepReceipt>> {
        let snapshot = self.snapshot(deadline).await?;
        if require_clean {
            ensure_clean(&snapshot)?;
        }
        let mut receipts = Vec::new();
        let mut ids = ids.into_iter();
        while let Some(id) = ids.next() {
            if Instant::now() >= deadline {
                receipts.push(StepReceipt {
                    step: validation_label(id).to_owned(),
                    ok: false,
                    summary: "skipped because the bounded maintenance deadline was exhausted"
                        .to_owned(),
                });
                receipts.extend(ids.map(|pending| StepReceipt {
                    step: validation_label(pending).to_owned(),
                    ok: false,
                    summary: "pending after the bounded validation sequence stopped".to_owned(),
                }));
                break;
            }
            if id == ValidationId::Submodules
                && let Err(error) = self.validate_submodules(&snapshot.root, deadline).await
            {
                receipts.push(StepReceipt {
                    step: validation_label(id).to_owned(),
                    ok: false,
                    summary: sanitize_text(&format!("submodule policy refused: {error:#}"), 512),
                });
                receipts.extend(ids.map(|pending| StepReceipt {
                    step: validation_label(pending).to_owned(),
                    ok: false,
                    summary:
                        "pending after an earlier validation step failed or timed out".to_owned(),
                }));
                break;
            }
            let spec = validation_command(id);
            let program = resolve_executable_in_path(
                spec.program,
                &snapshot.root,
                &self.validation_path,
                ExecutableTrust::Validation,
            )?
            .with_context(|| {
                format!(
                    "required validation executable `{}` is not installed",
                    spec.program
                )
            })?;
            let result = self
                .run_program(
                    &snapshot.root,
                    &program,
                    spec.args,
                    deadline,
                    CommandCapability::Validation,
                )
                .await;
            let (ok, summary) = match result {
                Ok(result) => (result.success, validation_command_summary(&result)),
                Err(error) => (false, sanitize_text(&format!("{error:#}"), 512)),
            };
            receipts.push(StepReceipt {
                step: spec.label.to_owned(),
                ok,
                summary,
            });
            if !ok {
                receipts.extend(ids.map(|pending| StepReceipt {
                    step: validation_label(pending).to_owned(),
                    ok: false,
                    summary:
                        "pending after an earlier validation step failed or timed out".to_owned(),
                }));
                break;
            }
        }
        Ok(receipts)
    }

    async fn ensure_topic_branch(
        &self,
        snapshot: RepositorySnapshot,
        branch: &str,
        allow_scoped_dirty: bool,
        deadline: Instant,
    ) -> Result<RepositorySnapshot> {
        if snapshot.branch.as_deref() == Some(branch) {
            return Ok(snapshot);
        }
        let branch_ref = format!("refs/heads/{branch}");
        let exists = self
            .git(
                &snapshot.root,
                ["show-ref", "--verify", "--quiet", branch_ref.as_str()],
                deadline,
            )
            .await?;
        ensure!(
            !exists.success,
            "topic branch already exists but is not checked out; maintenance refuses to switch across unrelated work"
        );
        if !allow_scoped_dirty {
            ensure_clean(&snapshot)?;
        }
        let switched = self
            .git(&snapshot.root, ["switch", "-c", branch], deadline)
            .await?;
        ensure!(switched.success, "creating topic branch failed");
        self.snapshot(deadline).await
    }

    async fn stage_paths(&self, root: &Path, paths: &[String], deadline: Instant) -> Result<()> {
        let mut args = vec!["add".to_owned(), "--".to_owned()];
        args.extend(paths.iter().cloned());
        let result = self.git_owned(root, args, deadline).await?;
        ensure!(result.success, "staging explicit paths failed");
        Ok(())
    }

    async fn staged_paths(&self, root: &Path, deadline: Instant) -> Result<(Vec<String>, bool)> {
        let result = self
            .git(root, ["diff", "--cached", "--name-only", "-z"], deadline)
            .await?;
        ensure!(result.success, "reading staged paths failed");
        Ok(parse_nul_paths(&result.stdout, MAX_PATHS))
    }

    async fn changed_paths_between(
        &self,
        root: &Path,
        base: &str,
        head: &str,
        deadline: Instant,
    ) -> Result<(Vec<String>, bool)> {
        let range = format!("{base}...{head}");
        let result = self
            .git(
                root,
                [
                    "diff",
                    "--name-status",
                    "-z",
                    "--find-renames",
                    "--find-copies",
                    range.as_str(),
                    "--",
                ],
                deadline,
            )
            .await?;
        ensure!(result.success, "reading canonical-base path diff failed");
        ensure!(
            !result.truncated,
            "canonical-base path diff exceeded the bounded command-output limit"
        );
        parse_name_status_paths(&result.stdout, MAX_PATHS)
    }

    async fn conflicted_paths(
        &self,
        root: &Path,
        deadline: Instant,
    ) -> Result<(Vec<String>, bool)> {
        let result = self
            .git(
                root,
                ["diff", "--name-only", "--diff-filter=U", "-z"],
                deadline,
            )
            .await?;
        ensure!(result.success, "reading conflicted files failed");
        Ok(parse_nul_paths(&result.stdout, MAX_PATHS))
    }

    async fn snapshot(&self, deadline: Instant) -> Result<RepositorySnapshot> {
        let git_executable = self.git_executable()?;
        let version_result = self
            .run_program(
                &self.workspace_root,
                git_executable,
                ["--version"],
                deadline,
                CommandCapability::Local,
            )
            .await
            .context("git is not installed or not executable")?;
        ensure!(version_result.success, "git --version failed");
        let git_version = first_sanitized_line(&version_result.stdout);

        let root_text = self
            .git_stdout(
                &self.workspace_root,
                ["rev-parse", "--show-toplevel"],
                deadline,
            )
            .await?;
        let root = fs::canonicalize(root_text.trim())
            .context("resolving the discovered repository root")?;
        ensure!(
            root.starts_with(&self.workspace_root),
            "discovered repository root escapes the configured operator workspace"
        );
        let dot_git = root.join(".git");
        let dot_git_metadata =
            fs::symlink_metadata(&dot_git).context("repository uses no in-tree .git directory")?;
        ensure!(
            dot_git_metadata.is_dir() && !dot_git_metadata.file_type().is_symlink(),
            "repository maintenance rejects linked worktrees, symlinked .git paths, and external Git directories"
        );
        let git_dir = self
            .git_stdout(&root, ["rev-parse", "--absolute-git-dir"], deadline)
            .await?;
        ensure!(
            fs::canonicalize(git_dir.trim())? == fs::canonicalize(&dot_git)?,
            "Git directory points outside the intended repository"
        );
        self.validate_git_configuration(&root, deadline).await?;

        let head = self
            .git_stdout(&root, ["rev-parse", "--verify", "HEAD"], deadline)
            .await?
            .trim()
            .to_owned();
        ensure!(is_hex_commit(&head), "Git returned a malformed HEAD commit");
        let branch_result = self
            .git(
                &root,
                ["symbolic-ref", "--quiet", "--short", "HEAD"],
                deadline,
            )
            .await?;
        let branch = branch_result
            .success
            .then(|| {
                String::from_utf8_lossy(&branch_result.stdout)
                    .trim()
                    .to_owned()
            })
            .filter(|value| !value.is_empty());
        if let Some(branch) = &branch {
            validate_branch_name(branch)?;
        }
        let tracked_ref = if let Some(branch) = &branch {
            let branch_ref = format!("refs/heads/{branch}");
            let value = self
                .git_stdout(
                    &root,
                    [
                        "for-each-ref",
                        "--format=%(upstream:short)",
                        branch_ref.as_str(),
                    ],
                    deadline,
                )
                .await?;
            let value = value.trim().to_owned();
            if value.is_empty() {
                None
            } else {
                let (remote, tracked_branch) = value
                    .split_once('/')
                    .context("tracked branch is not a bounded remote/branch ref")?;
                validate_remote_ref(remote, tracked_branch)?;
                Some(value)
            }
        } else {
            None
        };
        let remotes = self.remotes(&root, deadline).await?;
        let tracked_remote_is_safe = tracked_ref.as_deref().is_some_and(|tracked| {
            tracked
                .split_once('/')
                .and_then(|(name, _)| remotes.iter().find(|remote| remote.name == name))
                .is_some_and(|remote| remote.safe_for_network_maintenance)
        });
        let (ahead, behind) = if let Some(tracked) = &tracked_ref
            && tracked_remote_is_safe
        {
            let (ahead, behind) = self.divergence(&root, "HEAD", tracked, deadline).await?;
            (Some(ahead), Some(behind))
        } else {
            (None, None)
        };
        let status = self
            .git(
                &root,
                ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
                deadline,
            )
            .await?;
        ensure!(status.success, "git status failed");
        let (dirty_entries, dirty_entries_truncated) =
            parse_porcelain_entries(&status.stdout, MAX_DIRTY_ENTRIES);
        let canonical_remote = remotes
            .iter()
            .find(|remote| self.is_canonical_identity(remote.verified_identity()))
            .map(|remote| remote.name.clone());
        let origin = remotes.iter().find(|remote| remote.name == "origin");
        let topology = match (origin, canonical_remote.as_deref()) {
            (Some(origin), _) if self.is_canonical_identity(origin.verified_identity()) => {
                RepositoryTopology::Canonical
            }
            (Some(_), Some(_)) => RepositoryTopology::Fork,
            (Some(_), None) => RepositoryTopology::ForkMissingUpstream,
            (None, Some(_)) => RepositoryTopology::Canonical,
            _ => RepositoryTopology::Unknown,
        };
        Ok(RepositorySnapshot {
            root,
            head,
            branch,
            tracked_ref,
            ahead,
            behind,
            dirty_entries,
            dirty_entries_truncated,
            topology,
            canonical_remote,
            remotes,
            git_version,
        })
    }

    async fn validate_git_configuration(&self, root: &Path, deadline: Instant) -> Result<()> {
        // Values remain private to this check. System configuration is disabled for all children.
        let result = self
            .run_program(
                root,
                self.git_executable()?,
                ["config", "--no-includes", "--null", "--list"],
                deadline,
                CommandCapability::Local,
            )
            .await?;
        ensure!(result.success, "reading effective Git configuration failed");
        for item in result
            .stdout
            .split(|byte| *byte == 0)
            .filter(|item| !item.is_empty())
        {
            let text = String::from_utf8_lossy(item);
            let (key, value) = text.split_once('\n').unwrap_or((&text, ""));
            let key = key.to_ascii_lowercase();
            let dangerous = matches!(
                key.as_str(),
                "core.worktree"
                    | "core.hookspath"
                    | "core.fsmonitor"
                    | "core.sshcommand"
                    | "core.gitproxy"
                    | "core.askpass"
                    | "diff.external"
            ) || key.starts_with("include.")
                || key.starts_with("includeif.")
                || key.starts_with("filter.")
                || (key.starts_with("branch.") && key.ends_with(".mergeoptions"))
                || (key.starts_with("merge.") && key.ends_with(".driver"))
                || (key.starts_with("diff.")
                    && (key.ends_with(".command") || key.ends_with(".textconv")))
                || (key.starts_with("remote.")
                    && (key.ends_with(".uploadpack") || key.ends_with(".receivepack")))
                || key.starts_with("protocol.")
                || key.starts_with("url.")
                || (key.starts_with("http.")
                    && [
                        ".extraheader",
                        ".proxy",
                        ".proxyauthmethod",
                        ".cookiefile",
                        ".savecookies",
                        ".sslkey",
                        ".sslcert",
                        ".sslcertpasswordprotected",
                        ".sslcainfo",
                        ".sslcapath",
                    ]
                    .iter()
                    .any(|suffix| key.ends_with(suffix)))
                || (key.starts_with("submodule.")
                    && key.ends_with(".update")
                    && value.trim_start().starts_with('!'));
            ensure!(
                !dangerous,
                "effective Git configuration contains blocked executable/path override `{}`",
                sanitize_text(&key, 200)
            );
            if key == "credential.helper" {
                let helper = value.trim();
                ensure!(
                    helper.is_empty() || helper == "!gh auth git-credential",
                    "effective Git credential helper is not the allowlisted gh helper; use SSH-agent auth, GH_TOKEN/GITHUB_TOKEN, or `gh auth setup-git` without a custom helper"
                );
            }
        }
        Ok(())
    }

    async fn validate_submodules(&self, root: &Path, deadline: Instant) -> Result<()> {
        let gitmodules = root.join(".gitmodules");
        let metadata = match fs::symlink_metadata(&gitmodules) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).context("inspecting .gitmodules"),
        };
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            ".gitmodules must be a regular in-repository file"
        );
        let gitmodules = gitmodules
            .to_str()
            .context(".gitmodules path is not valid UTF-8")?;
        let result = self
            .git(
                root,
                [
                    "config",
                    "--file",
                    gitmodules,
                    "--no-includes",
                    "--null",
                    "--list",
                ],
                deadline,
            )
            .await?;
        ensure!(result.success, "parsing .gitmodules failed");

        let mut modules = BTreeMap::<String, SubmoduleConfig>::new();
        for item in result
            .stdout
            .split(|byte| *byte == 0)
            .filter(|item| !item.is_empty())
        {
            let text = String::from_utf8_lossy(item);
            let (key, value) = text.split_once('\n').unwrap_or((&text, ""));
            let Some(rest) = key.strip_prefix("submodule.") else {
                continue;
            };
            let Some((name, field)) = rest.rsplit_once('.') else {
                continue;
            };
            ensure!(
                !name.is_empty() && name.len() <= 512,
                "invalid bounded submodule name"
            );
            ensure!(
                modules.len() < MAX_PATHS || modules.contains_key(name),
                "too many submodules"
            );
            let module = modules.entry(name.to_owned()).or_default();
            let slot = match field {
                "path" => &mut module.path,
                "url" => &mut module.url,
                "update" => &mut module.update,
                _ => continue,
            };
            ensure!(slot.is_none(), "duplicate submodule {field} setting");
            *slot = Some(value.to_owned());
        }

        let mut paths = BTreeSet::new();
        for module in modules.values() {
            let path = module
                .path
                .as_deref()
                .context("submodule is missing a path")?;
            let url = module
                .url
                .as_deref()
                .context("submodule is missing a URL")?;
            let validated = validate_scoped_paths(root, &[path.to_owned()])?;
            ensure!(
                paths.insert(validated[0].clone()),
                "submodule paths must be unique"
            );
            let (_, safe_url) = parse_github_repository(url);
            ensure!(
                safe_url,
                "submodule URL must be one credential-free HTTPS or SSH GitHub repository"
            );
            ensure!(
                module
                    .update
                    .as_deref()
                    .is_none_or(|value| value == "checkout"),
                "submodule update mode must be absent or checkout"
            );
            validate_existing_submodule_git_dir(root, &validated[0])?;
        }
        Ok(())
    }

    async fn remotes(&self, root: &Path, deadline: Instant) -> Result<Vec<RemoteInfo>> {
        let names = self.git_stdout(root, ["remote"], deadline).await?;
        let mut remotes = Vec::new();
        for name in names.lines().take(MAX_REMOTE_COUNT + 1) {
            ensure!(
                remotes.len() < MAX_REMOTE_COUNT,
                "repository has too many remotes"
            );
            validate_remote_name(name)?;
            let fetch_url = self
                .git_stdout(root, ["remote", "get-url", "--all", "--", name], deadline)
                .await?;
            let fetch_url = exactly_one_remote_url(&fetch_url, name, "fetch")?;
            let push_url = self
                .git_stdout(
                    root,
                    ["remote", "get-url", "--push", "--all", "--", name],
                    deadline,
                )
                .await?;
            let push_url = exactly_one_remote_url(&push_url, name, "push")?;
            let refspec_key = format!("remote.{name}.fetch");
            let refspecs = self
                .git(
                    root,
                    ["config", "--get-all", refspec_key.as_str()],
                    deadline,
                )
                .await?;
            let expected_refspec = format!("+refs/heads/*:refs/remotes/{name}/*");
            let configured_refspec_text = String::from_utf8_lossy(&refspecs.stdout);
            let configured_refspecs = configured_refspec_text
                .lines()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            let fetch_refspec_safe = refspecs.success
                && configured_refspecs.len() == 1
                && configured_refspecs[0] == expected_refspec;
            let (fetch_identity, safe_fetch) = self.repository_identity(&fetch_url);
            let (push_identity, safe_push) = self.repository_identity(&push_url);
            let identities_match = fetch_identity == push_identity && fetch_identity.is_some();
            remotes.push(RemoteInfo {
                name: name.to_owned(),
                fetch_url,
                push_url,
                fetch_identity,
                push_identity,
                fetch_refspec_safe,
                safe_for_network_maintenance: safe_fetch
                    && safe_push
                    && identities_match
                    && fetch_refspec_safe,
            });
        }
        Ok(remotes)
    }

    async fn ensure_canonical_remote(
        &self,
        snapshot: &mut RepositorySnapshot,
        deadline: Instant,
    ) -> Result<String> {
        if let Some(name) = &snapshot.canonical_remote {
            self.ensure_safe_remote(snapshot, name)?;
            return Ok(name.clone());
        }
        ensure!(
            snapshot.topology == RepositoryTopology::ForkMissingUpstream,
            "repository has no provable canonical upstream remote"
        );
        let name = if snapshot
            .remotes
            .iter()
            .any(|remote| remote.name == "upstream")
        {
            "cthuwu-upstream"
        } else {
            "upstream"
        };
        ensure!(
            !snapshot.remotes.iter().any(|remote| remote.name == name),
            "no safe unused canonical-upstream remote name is available"
        );
        let result = self
            .git(
                &snapshot.root,
                [
                    "remote",
                    "add",
                    "--",
                    name,
                    self.policy.canonical_url.as_str(),
                ],
                deadline,
            )
            .await?;
        ensure!(
            result.success,
            "adding the canonical upstream remote failed"
        );
        *snapshot = self.snapshot(deadline).await?;
        ensure!(
            snapshot.canonical_remote.as_deref() == Some(name),
            "new canonical upstream remote did not verify"
        );
        Ok(name.to_owned())
    }

    async fn corresponding_upstream_branch(
        &self,
        root: &Path,
        remote: &str,
        current_branch: &str,
        deadline: Instant,
    ) -> Result<String> {
        for branch in [current_branch, self.policy.default_base_branch.as_str()] {
            validate_branch_name(branch)?;
            let remote_ref = format!("refs/remotes/{remote}/{branch}");
            let result = self
                .git(
                    root,
                    ["show-ref", "--verify", "--quiet", remote_ref.as_str()],
                    deadline,
                )
                .await?;
            if result.success {
                return Ok(branch.to_owned());
            }
        }
        bail!(
            "canonical remote exposes neither the current branch nor the configured default branch"
        )
    }

    fn ensure_safe_remote(&self, snapshot: &RepositorySnapshot, name: &str) -> Result<()> {
        let remote = snapshot
            .remotes
            .iter()
            .find(|remote| remote.name == name)
            .with_context(|| format!("remote {name} does not exist"))?;
        ensure!(
            remote.fetch_identity.is_some() && remote.push_identity.is_some(),
            "remote {name} uses an unsupported or credential-bearing fetch/push URL; maintenance refused before network access"
        );
        ensure!(
            remote.fetch_identity == remote.push_identity,
            "remote {name} fetch and push URLs identify different repositories; maintenance refused before network access"
        );
        ensure!(
            remote.fetch_refspec_safe,
            "remote {name} uses an unsupported fetch refspec; maintenance refused before network access"
        );
        ensure!(
            remote.safe_for_network_maintenance,
            "remote {name} uses an unsupported, credential-bearing, or non-GitHub URL; maintenance refused before network access"
        );
        Ok(())
    }

    fn ensure_operator_fork_remote<'a>(
        &self,
        snapshot: &'a RepositorySnapshot,
        name: &str,
    ) -> Result<&'a RemoteInfo> {
        ensure!(
            name == "origin",
            "publication is restricted to the verified operator-fork remote `origin`; no model-selected upstream or arbitrary remote may be pushed"
        );
        self.ensure_safe_remote(snapshot, name)?;
        let remote = snapshot
            .remotes
            .iter()
            .find(|remote| remote.name == name)
            .context("operator-fork origin remote is missing")?;
        let identity = remote
            .verified_identity()
            .context("operator-fork origin has no verified repository identity")?;
        ensure!(
            !self.is_canonical_identity(Some(identity)),
            "origin is the canonical upstream repository; publication requires a noncanonical operator fork as origin and the canonical repository as upstream"
        );
        ensure!(
            snapshot.topology == RepositoryTopology::Fork
                && snapshot
                    .canonical_remote
                    .as_deref()
                    .is_some_and(|canonical| canonical != name),
            "origin cannot be verified as an operator fork until the canonical repository is configured as a separate upstream remote; run the typed update/fetch workflow after configuring the fork"
        );
        Ok(remote)
    }

    fn ensure_canonical_upstream_remote<'a>(
        &self,
        snapshot: &'a RepositorySnapshot,
        name: &str,
    ) -> Result<&'a RemoteInfo> {
        self.ensure_safe_remote(snapshot, name)?;
        ensure!(
            snapshot.canonical_remote.as_deref() == Some(name),
            "typed merge may target only the verified canonical upstream remote"
        );
        let remote = snapshot
            .remotes
            .iter()
            .find(|remote| remote.name == name)
            .context("verified canonical upstream remote is missing")?;
        ensure!(
            self.is_canonical_identity(remote.verified_identity()),
            "typed merge remote is not the canonical upstream repository"
        );
        Ok(remote)
    }

    fn repository_identity(&self, value: &str) -> (Option<RepositoryIdentity>, bool) {
        #[cfg(test)]
        if let Some(canonical) = &self.policy.canonical_local_path {
            if let (Ok(value), Ok(canonical)) =
                (fs::canonicalize(value), fs::canonicalize(canonical))
                && value == canonical
            {
                return (
                    Some(RepositoryIdentity {
                        owner: self.policy.canonical_owner.clone(),
                        repository: self.policy.canonical_repository.clone(),
                    }),
                    true,
                );
            }
            if Path::new(value).is_absolute() {
                let repository = Path::new(value)
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .unwrap_or("fork")
                    .to_owned();
                return (
                    Some(RepositoryIdentity {
                        owner: "test-fork".to_owned(),
                        repository,
                    }),
                    true,
                );
            }
        }
        parse_github_repository(value)
    }

    fn is_canonical_identity(&self, identity: Option<&RepositoryIdentity>) -> bool {
        identity.is_some_and(|identity| {
            identity
                .owner
                .eq_ignore_ascii_case(&self.policy.canonical_owner)
                && identity
                    .repository
                    .eq_ignore_ascii_case(&self.policy.canonical_repository)
        })
    }

    async fn divergence(
        &self,
        root: &Path,
        left: &str,
        right: &str,
        deadline: Instant,
    ) -> Result<(u64, u64)> {
        let range = format!("{left}...{right}");
        let value = self
            .git_stdout(
                root,
                ["rev-list", "--left-right", "--count", range.as_str()],
                deadline,
            )
            .await?;
        let mut fields = value.split_whitespace();
        let ahead = fields
            .next()
            .context("Git divergence result omitted ahead count")?
            .parse()?;
        let behind = fields
            .next()
            .context("Git divergence result omitted behind count")?
            .parse()?;
        ensure!(
            fields.next().is_none(),
            "Git divergence result was malformed"
        );
        Ok((ahead, behind))
    }

    async fn divergent_commits(
        &self,
        root: &Path,
        target: &str,
        local_count: u64,
        upstream_count: u64,
        deadline: Instant,
    ) -> Result<DivergentCommitSummary> {
        let range = format!("HEAD...{target}");
        let maximum = (MAX_DIVERGENT_COMMITS + 1).to_string();
        let value = self
            .git_stdout(
                root,
                [
                    "log",
                    "--left-right",
                    "--format=%m%H",
                    "--max-count",
                    maximum.as_str(),
                    range.as_str(),
                ],
                deadline,
            )
            .await?;
        let mut local_hashes = Vec::new();
        let mut upstream_hashes = Vec::new();
        let mut count = 0_usize;
        let mut truncated = false;
        for line in value.lines() {
            if count >= MAX_DIVERGENT_COMMITS {
                truncated = true;
                break;
            }
            let (side, hash) = line.split_at(line.len().min(1));
            ensure!(
                is_hex_commit(hash),
                "Git returned a malformed divergent commit hash"
            );
            match side {
                "<" => local_hashes.push(hash.to_owned()),
                ">" => upstream_hashes.push(hash.to_owned()),
                _ => continue,
            }
            count += 1;
        }
        Ok(DivergentCommitSummary {
            local_hashes,
            upstream_hashes,
            local_count,
            upstream_count,
            truncated,
        })
    }

    async fn gh_status(&self, root: &Path, deadline: Instant) -> GhStatus {
        let Some(gh_executable) = self.gh_executable.as_deref() else {
            return GhStatus {
                installed: false,
                version: None,
                authenticated_for_github_com: false,
            };
        };
        let version = match self
            .run_program(
                root,
                gh_executable,
                ["--version"],
                deadline,
                CommandCapability::GithubCli,
            )
            .await
        {
            Ok(result) if result.success => Some(first_sanitized_line(&result.stdout)),
            _ => None,
        };
        if version.is_none() {
            return GhStatus {
                installed: false,
                version: None,
                authenticated_for_github_com: false,
            };
        }
        let authenticated = self
            .run_program(
                root,
                gh_executable,
                ["auth", "status", "--hostname", "github.com"],
                deadline,
                CommandCapability::GithubCli,
            )
            .await
            .is_ok_and(|result| result.success);
        GhStatus {
            installed: true,
            version,
            authenticated_for_github_com: authenticated,
        }
    }

    async fn git_stdout<I, S>(&self, root: &Path, args: I, deadline: Instant) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let result = self.git(root, args, deadline).await?;
        ensure!(
            result.success,
            "Git command failed: {}",
            command_summary(&result)
        );
        String::from_utf8(result.stdout).context("Git returned non-UTF-8 metadata")
    }

    fn git_executable(&self) -> Result<&Path> {
        self.git_executable
            .as_deref()
            .context("git is not installed as a trusted absolute executable")
    }

    async fn git<I, S>(&self, root: &Path, args: I, deadline: Instant) -> Result<CommandResult>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.git_owned(
            root,
            args.into_iter()
                .map(|value| value.as_ref().to_string_lossy().into_owned())
                .collect(),
            deadline,
        )
        .await
    }

    async fn git_network<I, S>(
        &self,
        root: &Path,
        args: I,
        deadline: Instant,
    ) -> Result<CommandResult>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut safe = vec![
            "-c".to_owned(),
            "core.hooksPath=/dev/null".to_owned(),
            "-c".to_owned(),
            "commit.gpgSign=false".to_owned(),
            "-c".to_owned(),
            "gc.auto=0".to_owned(),
        ];
        safe.extend(
            args.into_iter()
                .map(|value| value.as_ref().to_string_lossy().into_owned()),
        );
        self.run_program(
            root,
            self.git_executable()?,
            safe,
            deadline,
            CommandCapability::GitNetwork,
        )
        .await
    }

    async fn git_owned(
        &self,
        root: &Path,
        args: Vec<String>,
        deadline: Instant,
    ) -> Result<CommandResult> {
        let mut safe = vec![
            "-c".to_owned(),
            "core.hooksPath=/dev/null".to_owned(),
            "-c".to_owned(),
            "commit.gpgSign=false".to_owned(),
            "-c".to_owned(),
            "gc.auto=0".to_owned(),
        ];
        safe.extend(args);
        self.run_program(
            root,
            self.git_executable()?,
            safe,
            deadline,
            CommandCapability::Local,
        )
        .await
    }

    async fn run_program<I, S>(
        &self,
        root: &Path,
        program: &Path,
        args: I,
        deadline: Instant,
        capability: CommandCapability,
    ) -> Result<CommandResult>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        ensure!(
            root.starts_with(&self.workspace_root),
            "command root escaped workspace"
        );
        ensure!(
            program.is_absolute(),
            "maintenance executable was not resolved to an absolute path"
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(!remaining.is_zero(), "maintenance deadline exhausted");
        let limit = remaining.min(COMMAND_LIMIT);
        let command_path = if matches!(capability, CommandCapability::Validation) {
            &self.validation_path
        } else {
            &self.authentication_path
        };
        run_bounded_command(program, args, root, limit, capability, command_path).await
    }

    #[cfg(test)]
    fn for_local_test(
        workspace_root: &Path,
        canonical_remote: &Path,
        gh_executable: PathBuf,
    ) -> Result<Self> {
        let mut maintenance = Self::new(workspace_root, 60)?;
        maintenance.policy.canonical_owner = "canonical".to_owned();
        maintenance.policy.canonical_repository = "cthuwu".to_owned();
        maintenance.policy.canonical_url = canonical_remote.to_string_lossy().into_owned();
        maintenance.policy.canonical_local_path = Some(canonical_remote.to_owned());
        maintenance.policy.update_validation.clear();
        maintenance.policy.pr_validation.clear();
        maintenance.gh_executable = Some(gh_executable);
        Ok(maintenance)
    }
}

impl RepositorySnapshot {
    fn to_status(&self, gh: GhStatus, policy: &RepositoryPolicy) -> RepositoryStatus {
        RepositoryStatus {
            repository_root: self.root.display().to_string(),
            head: self.head.clone(),
            branch: self.branch.clone(),
            tracked_ref: self.tracked_ref.clone(),
            ahead: self.ahead,
            behind: self.behind,
            dirty: !self.dirty_entries.is_empty(),
            dirty_entries: self.dirty_entries.clone(),
            dirty_entries_truncated: self.dirty_entries_truncated,
            topology: self.topology,
            canonical_remote: self.canonical_remote.clone(),
            remotes: self
                .remotes
                .iter()
                .map(|remote| RemoteStatus {
                    name: remote.name.clone(),
                    fetch_url: sanitize_remote_url(&remote.fetch_url),
                    push_url: sanitize_remote_url(&remote.push_url),
                    fetch_repository: remote
                        .fetch_identity
                        .as_ref()
                        .map(|identity| format!("{}/{}", identity.owner, identity.repository)),
                    push_repository: remote
                        .push_identity
                        .as_ref()
                        .map(|identity| format!("{}/{}", identity.owner, identity.repository)),
                    identities_match: remote.fetch_identity == remote.push_identity
                        && remote.fetch_identity.is_some(),
                    canonical: remote.verified_identity().is_some_and(|identity| {
                        identity.owner.eq_ignore_ascii_case(&policy.canonical_owner)
                            && identity
                                .repository
                                .eq_ignore_ascii_case(&policy.canonical_repository)
                    }),
                    fetch_refspec_safe: remote.fetch_refspec_safe,
                    safe_for_network_maintenance: remote.safe_for_network_maintenance,
                })
                .collect(),
            git: CapabilityStatus {
                installed: true,
                version: Some(self.git_version.clone()),
            },
            gh,
            source_update_note: format!(
                "Repository source and the running executable are distinct. A successful source update never claims this already-running process changed; {}.",
                policy.restart
            ),
        }
    }
}

#[derive(Clone, Copy)]
enum ValidationKind {
    Test(TestProfile),
    Build(BuildProfile),
}

struct ValidationCommand {
    label: &'static str,
    program: &'static str,
    args: &'static [&'static str],
}

fn validation_ids(kind: ValidationKind) -> Vec<ValidationId> {
    match kind {
        ValidationKind::Test(TestProfile::Focused) => {
            vec![ValidationId::RustFmt, ValidationId::RustTest]
        }
        ValidationKind::Build(BuildProfile::Runtime) => vec![
            ValidationId::Submodules,
            ValidationId::AgentInstall,
            ValidationId::AgentBuild,
            ValidationId::RustBuild,
        ],
        ValidationKind::Test(TestProfile::Required) => vec![
            ValidationId::RustFmt,
            ValidationId::RustTest,
            ValidationId::RustClippy,
            ValidationId::AgentTypecheck,
            ValidationId::AgentTest,
            ValidationId::LauncherTest,
            ValidationId::InstallTest,
            ValidationId::WebTypecheck,
            ValidationId::WebTest,
            ValidationId::ForgeFmt,
            ValidationId::ForgeLint,
            ValidationId::ForgeTest,
        ],
        ValidationKind::Build(BuildProfile::Required) => vec![
            ValidationId::Submodules,
            ValidationId::AgentInstall,
            ValidationId::AgentBuild,
            ValidationId::LauncherSmoke,
            ValidationId::WebInstall,
            ValidationId::WebBuild,
            ValidationId::ForgeBuild,
            ValidationId::RustBuild,
        ],
    }
}

fn validation_label(id: ValidationId) -> &'static str {
    validation_command(id).label
}

fn validation_command(id: ValidationId) -> ValidationCommand {
    match id {
        ValidationId::Submodules => ValidationCommand {
            label: "initialize locked submodules",
            program: "git",
            args: &[
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "protocol.file.allow=never",
                "submodule",
                "update",
                "--init",
                "--recursive",
            ],
        },
        ValidationId::AgentInstall => ValidationCommand {
            label: "install locked agent dependencies",
            program: "npm",
            args: &[
                "--prefix",
                "agent",
                "ci",
                "--include=dev",
                "--no-audit",
                "--no-fund",
            ],
        },
        ValidationId::AgentTypecheck => ValidationCommand {
            label: "agent typecheck",
            program: "npm",
            args: &["--prefix", "agent", "run", "typecheck"],
        },
        ValidationId::AgentTest => ValidationCommand {
            label: "agent tests",
            program: "npm",
            args: &["--prefix", "agent", "test"],
        },
        ValidationId::AgentBuild => ValidationCommand {
            label: "agent build",
            program: "npm",
            args: &["--prefix", "agent", "run", "build"],
        },
        ValidationId::LauncherSmoke => ValidationCommand {
            label: "uwu launcher production smoke test",
            program: "bash",
            args: &["scripts/smoke-uwu.sh"],
        },
        ValidationId::LauncherTest => ValidationCommand {
            label: "uwu launcher tests",
            program: "bash",
            args: &["scripts/test-uwu.sh"],
        },
        ValidationId::InstallTest => ValidationCommand {
            label: "installer tests",
            program: "bash",
            args: &["scripts/test-install.sh"],
        },
        ValidationId::RustFmt => ValidationCommand {
            label: "Rust formatting",
            program: "cargo",
            args: &[
                "fmt",
                "--manifest-path",
                "cthuwu/Cargo.toml",
                "--all",
                "--",
                "--check",
            ],
        },
        ValidationId::RustTest => ValidationCommand {
            label: "Rust workspace tests",
            program: "cargo",
            args: &[
                "test",
                "--manifest-path",
                "cthuwu/Cargo.toml",
                "--workspace",
                "--locked",
            ],
        },
        ValidationId::RustClippy => ValidationCommand {
            label: "Rust clippy",
            program: "cargo",
            args: &[
                "clippy",
                "--manifest-path",
                "cthuwu/Cargo.toml",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
        },
        ValidationId::RustBuild => ValidationCommand {
            label: "Rust release build",
            program: "cargo",
            args: &[
                "build",
                "--manifest-path",
                "cthuwu/Cargo.toml",
                "--release",
                "--locked",
            ],
        },
        ValidationId::WebInstall => ValidationCommand {
            label: "install locked web dependencies",
            program: "npm",
            args: &[
                "--prefix",
                "web",
                "ci",
                "--include=dev",
                "--no-audit",
                "--no-fund",
            ],
        },
        ValidationId::WebTypecheck => ValidationCommand {
            label: "web typecheck",
            program: "npm",
            args: &["--prefix", "web", "run", "typecheck"],
        },
        ValidationId::WebTest => ValidationCommand {
            label: "web tests",
            program: "npm",
            args: &["--prefix", "web", "test"],
        },
        ValidationId::WebBuild => ValidationCommand {
            label: "web build",
            program: "npm",
            args: &["--prefix", "web", "run", "build"],
        },
        ValidationId::ForgeFmt => ValidationCommand {
            label: "Foundry formatting",
            program: "forge",
            args: &["fmt", "--root", "contracts", "--check"],
        },
        ValidationId::ForgeLint => ValidationCommand {
            label: "Foundry lint",
            program: "forge",
            args: &["lint", "--root", "contracts"],
        },
        ValidationId::ForgeBuild => ValidationCommand {
            label: "Foundry size build",
            program: "forge",
            args: &["build", "--root", "contracts", "--sizes"],
        },
        ValidationId::ForgeTest => ValidationCommand {
            label: "Foundry tests",
            program: "forge",
            args: &["test", "--root", "contracts", "-vvv"],
        },
    }
}

fn ensure_clean(snapshot: &RepositorySnapshot) -> Result<()> {
    ensure!(
        snapshot.dirty_entries.is_empty() && !snapshot.dirty_entries_truncated,
        "working tree is dirty; maintenance preserved local changes and refused to alter Git state"
    );
    Ok(())
}

fn ensure_post_validation_state(
    expected: &RepositorySnapshot,
    actual: &RepositorySnapshot,
) -> Result<()> {
    ensure!(
        actual.head == expected.head,
        "validation changed HEAD; publication refused"
    );
    ensure!(
        actual.branch == expected.branch,
        "validation changed the checked-out branch; publication refused"
    );
    ensure_clean(actual)?;
    ensure!(
        actual.topology == expected.topology
            && actual.canonical_remote == expected.canonical_remote
            && actual.remotes == expected.remotes,
        "validation changed repository remotes or topology; publication refused"
    );
    Ok(())
}

fn ensure_dirty_is_scoped(snapshot: &RepositorySnapshot, scoped_paths: &[String]) -> Result<()> {
    ensure!(
        !snapshot.dirty_entries_truncated,
        "working tree has too many dirty paths for a complete scoped review; maintenance preserved all changes and refused to commit"
    );
    for entry in &snapshot.dirty_entries {
        let path = entry.get(3..).unwrap_or_default();
        ensure!(
            path_is_scoped(path, scoped_paths),
            "working tree contains an unrelated dirty path; the scoped commit/PR workflow preserved it and refused to switch branches"
        );
    }
    Ok(())
}

fn ensure_staged_is_scoped(
    staged_paths: &[String],
    staged_paths_truncated: bool,
    scoped_paths: &[String],
) -> Result<()> {
    ensure!(
        !staged_paths_truncated,
        "staged path list exceeded the bounded review limit; maintenance preserved the index and refused to commit"
    );
    for path in staged_paths {
        ensure!(
            path_is_scoped(path, scoped_paths),
            "staged changes include a path outside the explicit scope; maintenance preserved the index and refused to commit"
        );
    }
    Ok(())
}

fn ensure_pr_diff_is_scoped(
    changed_paths: &[String],
    changed_paths_truncated: bool,
    scoped_paths: &[String],
    require_nonempty: bool,
) -> Result<()> {
    ensure!(
        !changed_paths_truncated,
        "canonical-base PR path diff exceeded the bounded review limit; pull-request publication was refused"
    );
    if require_nonempty {
        ensure!(
            !changed_paths.is_empty(),
            "canonical-base PR path diff is empty; pull-request publication was refused"
        );
    }
    for path in changed_paths {
        ensure!(
            path_is_scoped(path, scoped_paths),
            "canonical-base PR path diff includes a path outside the explicit PR scope; existing fork commits were preserved and publication was refused"
        );
    }
    Ok(())
}

fn path_is_scoped(path: &str, scoped_paths: &[String]) -> bool {
    scoped_paths.iter().any(|scope| {
        path == scope
            || path
                .strip_prefix(scope)
                .is_some_and(|tail| tail.starts_with('/'))
    })
}

fn validate_remote_name(value: &str) -> Result<()> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && value != "."
        && value != "..";
    ensure!(valid, "invalid Git remote name");
    Ok(())
}

fn validate_branch_name(value: &str) -> Result<()> {
    let invalid = value.is_empty()
        || value.chars().count() > MAX_BRANCH_CHARS
        || value.starts_with('-')
        || value.starts_with('.')
        || value.ends_with('.')
        || value.ends_with('/')
        || value.ends_with(".lock")
        || value.contains("..")
        || value.contains("@{")
        || value.contains("//")
        || value
            .chars()
            .any(|character| character.is_control() || " ~^:?*[\\\\".contains(character));
    ensure!(!invalid, "invalid Git branch/ref name");
    Ok(())
}

fn validate_remote_ref(remote: &str, branch: &str) -> Result<()> {
    validate_remote_name(remote)?;
    validate_branch_name(branch)
}

fn validate_commit_message(value: &str) -> Result<()> {
    let trimmed = value.trim();
    ensure!(
        !trimmed.is_empty()
            && trimmed.chars().count() <= MAX_COMMIT_MESSAGE_CHARS
            && !trimmed.chars().any(char::is_control),
        "commit message must be one bounded printable line"
    );
    Ok(())
}

fn validate_pr_text(title: &str, body: &str) -> Result<()> {
    ensure!(
        !title.trim().is_empty()
            && title.chars().count() <= MAX_PR_TITLE_CHARS
            && !title.chars().any(char::is_control),
        "PR title must be one bounded printable line"
    );
    ensure!(
        !body.trim().is_empty()
            && body.len() <= MAX_PR_BODY_BYTES
            && !body.chars().any(|character| character == '\0'),
        "PR body must be non-empty bounded UTF-8 text"
    );
    Ok(())
}

fn validate_scoped_paths(root: &Path, values: &[String]) -> Result<Vec<String>> {
    ensure!(
        values.len() <= MAX_PATHS,
        "too many scoped repository paths"
    );
    let mut paths = Vec::new();
    for value in values {
        ensure!(
            !value.trim().is_empty()
                && value.len() <= 2048
                && !value.chars().any(|character| character == '\0'),
            "invalid scoped repository path"
        );
        let path = Path::new(value);
        ensure!(
            !path.is_absolute(),
            "scoped path must be repository-relative"
        );
        ensure!(
            !path.components().any(|component| {
                matches!(
                    component,
                    Component::CurDir
                        | Component::ParentDir
                        | Component::RootDir
                        | Component::Prefix(_)
                )
            }),
            "scoped path contains traversal or a current-directory scope"
        );
        ensure!(
            !path
                .components()
                .any(|component| component == Component::Normal(OsStr::new(".git"))),
            "maintenance never accepts Git administrative paths"
        );
        let candidate = root.join(path);
        if let Ok(metadata) = fs::symlink_metadata(&candidate) {
            ensure!(
                !metadata.file_type().is_symlink(),
                "scoped path is a symlink"
            );
        }
        let parent = candidate.parent().context("scoped path has no parent")?;
        let canonical_parent = fs::canonicalize(parent)
            .with_context(|| format!("resolving scoped path parent {}", parent.display()))?;
        ensure!(
            canonical_parent.starts_with(root),
            "scoped path escapes repository root"
        );
        paths.push(path.to_string_lossy().replace('\\', "/"));
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn validate_existing_submodule_git_dir(root: &Path, submodule_path: &str) -> Result<()> {
    let dot_git = root.join(submodule_path).join(".git");
    let metadata = match fs::symlink_metadata(&dot_git) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("inspecting existing submodule Git metadata"),
    };
    ensure!(
        !metadata.file_type().is_symlink(),
        "submodule Git metadata may not be a symlink"
    );
    let modules_root = fs::canonicalize(root.join(".git").join("modules"))
        .unwrap_or_else(|_| root.join(".git").join("modules"));
    if metadata.is_dir() {
        let resolved = fs::canonicalize(&dot_git)?;
        ensure!(
            resolved.starts_with(root) || resolved.starts_with(&modules_root),
            "submodule Git directory escapes the repository"
        );
        return Ok(());
    }
    ensure!(metadata.is_file(), "invalid submodule Git metadata type");
    let contents = fs::read_to_string(&dot_git).context("reading submodule Git metadata")?;
    ensure!(
        contents.len() <= 4096,
        "submodule Git metadata pointer is too large"
    );
    let relative = contents
        .trim()
        .strip_prefix("gitdir:")
        .map(str::trim)
        .context("invalid submodule Git metadata pointer")?;
    let pointer = Path::new(relative);
    ensure!(
        !pointer.is_absolute()
            && !pointer
                .components()
                .any(|component| matches!(component, Component::RootDir | Component::Prefix(_))),
        "submodule Git metadata pointer must be repository-relative"
    );
    let resolved = fs::canonicalize(
        dot_git
            .parent()
            .context("submodule .git has no parent")?
            .join(pointer),
    )?;
    ensure!(
        resolved.starts_with(&modules_root),
        "submodule Git metadata pointer escapes .git/modules"
    );
    Ok(())
}

fn resolve_executable_in_path(
    name: &str,
    workspace_root: &Path,
    search_path: &OsStr,
    trust: ExecutableTrust,
) -> Result<Option<PathBuf>> {
    ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "invalid compiled executable name"
    );
    for directory in std::env::split_paths(search_path) {
        if !directory.is_absolute() {
            continue;
        }
        match fs::symlink_metadata(&directory) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("inspecting executable search directory"),
        }
        let resolved_directory =
            fs::canonicalize(&directory).context("resolving executable search directory")?;
        if resolved_directory.starts_with(workspace_root) {
            continue;
        }
        let candidate = directory.join(name);
        match fs::symlink_metadata(&candidate) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("inspecting maintenance executable"),
        }
        let resolved = match fs::canonicalize(&candidate) {
            Ok(resolved) => resolved,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("resolving maintenance executable"),
        };
        if resolved.starts_with(workspace_root) {
            continue;
        }
        let metadata = fs::metadata(&resolved)?;
        if !metadata.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }
        if matches!(trust, ExecutableTrust::Authentication)
            && !authentication_path_is_trusted(&resolved)
        {
            continue;
        }
        return Ok(Some(if matches!(trust, ExecutableTrust::Authentication) {
            // Authentication-bearing tools are pinned to the canonical regular file so a PATH
            // symlink cannot be swapped after capability discovery.
            resolved
        } else {
            // Rustup/npm shims depend on argv[0], so validation preserves the absolute shim path
            // after proving both it and its target are outside the writable workspace. Validation
            // children receive no GitHub or SSH authentication environment.
            candidate
        }));
    }
    Ok(None)
}

fn authentication_path_is_trusted(path: &Path) -> bool {
    [
        "/usr/bin",
        "/usr/local/bin",
        "/usr/sbin",
        "/usr/local/sbin",
        "/bin",
        "/sbin",
        "/nix/store",
        "/opt/homebrew/bin",
        "/opt/homebrew/Cellar",
        "/opt/homebrew/opt",
        "/opt/local/bin",
    ]
    .iter()
    .map(Path::new)
    .any(|prefix| path.starts_with(prefix))
}

fn sanitized_command_path(
    workspace_root: &Path,
    search_path: &OsStr,
    trust: ExecutableTrust,
    pinned_executables: &[PathBuf],
) -> Result<OsString> {
    let mut directories = Vec::new();
    for directory in std::env::split_paths(search_path) {
        if !directory.is_absolute() {
            continue;
        }
        let resolved = match fs::canonicalize(&directory) {
            Ok(resolved) => resolved,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("resolving child executable search directory"),
        };
        if resolved.starts_with(workspace_root) || !resolved.is_dir() {
            continue;
        }
        if matches!(trust, ExecutableTrust::Authentication)
            && !authentication_path_is_trusted(&resolved)
        {
            continue;
        }
        if !directories.contains(&resolved) {
            directories.push(resolved);
        }
    }
    for executable in pinned_executables {
        let Some(parent) = executable.parent() else {
            continue;
        };
        let resolved = fs::canonicalize(parent).context("resolving pinned executable directory")?;
        if resolved.starts_with(workspace_root)
            || !resolved.is_dir()
            || (matches!(trust, ExecutableTrust::Authentication)
                && !authentication_path_is_trusted(&resolved))
        {
            continue;
        }
        if !directories.contains(&resolved) {
            directories.push(resolved);
        }
    }
    std::env::join_paths(directories).context("building sanitized maintenance PATH")
}

fn parse_github_repository(value: &str) -> (Option<RepositoryIdentity>, bool) {
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        let identity = parse_owner_repository(rest);
        let safe = identity.is_some();
        return (identity, safe);
    }
    let Ok(url) = reqwest::Url::parse(trimmed) else {
        return (None, false);
    };
    if !url
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("github.com"))
        || !matches!(url.scheme(), "https" | "ssh")
        || url.password().is_some()
        || (!url.username().is_empty() && url.username() != "git")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return (None, false);
    }
    let identity = parse_owner_repository(url.path().trim_start_matches('/'));
    let safe = identity.is_some();
    (identity, safe)
}

fn parse_owner_repository(value: &str) -> Option<RepositoryIdentity> {
    let value = value.trim_end_matches('/');
    // Remove exactly one transport suffix. A repository actually named `cthuwu.git` is encoded
    // as `cthuwu.git.git` and must never collapse into the canonical `cthuwu` identity.
    let value = value.strip_suffix(".git").unwrap_or(value);
    let mut parts = value.split('/');
    let owner = parts.next()?;
    let repository = parts.next()?;
    if parts.next().is_some()
        || !valid_github_component(owner)
        || !valid_github_component(repository)
    {
        return None;
    }
    Some(RepositoryIdentity {
        owner: owner.to_owned(),
        repository: repository.to_owned(),
    })
}

fn valid_github_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn exactly_one_remote_url(value: &str, name: &str, direction: &str) -> Result<String> {
    let values = value
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    ensure!(
        values.len() == 1,
        "remote {name} must have exactly one {direction} URL; maintenance refused ambiguous remote configuration"
    );
    Ok(values[0].to_owned())
}

fn sanitize_remote_url(value: &str) -> String {
    let (identity, safe) = parse_github_repository(value);
    if let Some(identity) = identity
        && safe
    {
        return format!(
            "https://github.com/{}/{}.git",
            identity.owner, identity.repository
        );
    }
    #[cfg(test)]
    if Path::new(value).is_absolute() {
        return "[local-test-remote]".to_owned();
    }
    if value.contains('@') || value.contains("://") {
        "[redacted-or-unsupported-remote-url]".to_owned()
    } else {
        sanitize_text(value, 256)
    }
}

fn sanitize_text(value: &str, maximum_bytes: usize) -> String {
    let mut output = value
        .chars()
        .map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\t') {
                '�'
            } else {
                character
            }
        })
        .collect::<String>();
    for prefix in ["github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "sk-"] {
        redact_prefixed_tokens(&mut output, prefix);
    }
    output = redact_url_userinfo(&output);
    for key in ["TOKEN=", "PASSWORD=", "SECRET=", "API_KEY=", "PRIVATE_KEY="] {
        redact_assignment(&mut output, key);
    }
    truncate_string(output, maximum_bytes)
}

fn redact_prefixed_tokens(value: &mut String, prefix: &str) {
    let mut start = 0;
    while let Some(relative) = value[start..].find(prefix) {
        let token_start = start + relative;
        let token_end = value[token_start..]
            .char_indices()
            .find_map(|(offset, character)| {
                (offset >= prefix.len()
                    && !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-')))
                .then_some(token_start + offset)
            })
            .unwrap_or(value.len());
        value.replace_range(token_start..token_end, "[redacted-secret]");
        start = token_start + "[redacted-secret]".len();
    }
}

fn redact_assignment(value: &mut String, key: &str) {
    let mut cursor = 0;
    while cursor < value.len() {
        let upper = value[cursor..].to_ascii_uppercase();
        let Some(relative) = upper.find(key) else {
            break;
        };
        let start = cursor + relative;
        let value_start = start + key.len();
        let value_end = value[value_start..]
            .char_indices()
            .find_map(|(offset, character)| {
                character.is_whitespace().then_some(value_start + offset)
            })
            .unwrap_or(value.len());
        if value[value_start..value_end] != *"[redacted]" {
            value.replace_range(value_start..value_end, "[redacted]");
        }
        cursor = value_start + "[redacted]".len();
    }
}

fn redact_url_userinfo(value: &str) -> String {
    let mut output = value.to_owned();
    for scheme in ["https://", "http://", "ssh://"] {
        let mut cursor = 0;
        while let Some(relative) = output[cursor..].find(scheme) {
            let authority_start = cursor + relative + scheme.len();
            let authority_end = output[authority_start..]
                .char_indices()
                .find_map(|(offset, character)| {
                    matches!(character, '/' | ' ' | '\n' | '\t').then_some(authority_start + offset)
                })
                .unwrap_or(output.len());
            if let Some(at) = output[authority_start..authority_end].find('@') {
                let at = authority_start + at;
                let username = &output[authority_start..at];
                if username != "git" {
                    output.replace_range(authority_start..at, "[redacted]");
                }
            }
            cursor = authority_end.min(output.len());
            if cursor == output.len() {
                break;
            }
        }
    }
    output
}

fn truncate_string(mut value: String, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value;
    }
    let suffix = "…[truncated]";
    let mut boundary = maximum_bytes.saturating_sub(suffix.len());
    while !value.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    value.truncate(boundary);
    value.push_str(suffix);
    value
}

fn parse_porcelain_entries(value: &[u8], maximum: usize) -> (Vec<String>, bool) {
    let mut entries = Vec::new();
    let mut parts = value
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty());
    let mut truncated = false;
    while let Some(entry) = parts.next() {
        if entries.len() >= maximum {
            truncated = true;
            break;
        }
        let rendered = String::from_utf8_lossy(entry);
        let status = rendered.get(..2).unwrap_or("??");
        let path = rendered.get(3..).unwrap_or_default();
        entries.push(format!(
            "{} {}",
            sanitize_text(status, 2),
            sanitize_text(path, 512)
        ));
        if matches!(status.as_bytes().first(), Some(b'R' | b'C'))
            || matches!(status.as_bytes().get(1), Some(b'R' | b'C'))
        {
            let Some(source) = parts.next() else {
                truncated = true;
                break;
            };
            if entries.len() >= maximum {
                truncated = true;
                break;
            }
            entries.push(format!(
                "{} {}",
                sanitize_text(status, 2),
                sanitize_text(&String::from_utf8_lossy(source), 512)
            ));
        }
    }
    (entries, truncated)
}

fn parse_nul_paths(value: &[u8], maximum: usize) -> (Vec<String>, bool) {
    let paths = value
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| sanitize_text(&String::from_utf8_lossy(part), 512))
        .collect::<Vec<_>>();
    let truncated = paths.len() > maximum;
    (paths.into_iter().take(maximum).collect(), truncated)
}

fn parse_name_status_paths(value: &[u8], maximum: usize) -> Result<(Vec<String>, bool)> {
    let mut fields = value
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut paths = Vec::new();
    let mut truncated = false;
    while let Some(status) = fields.next() {
        let status = std::str::from_utf8(status).context("Git diff status was not UTF-8")?;
        ensure!(
            status.len() <= 4
                && matches!(
                    status.as_bytes().first(),
                    Some(b'A' | b'C' | b'D' | b'M' | b'R' | b'T' | b'U' | b'X' | b'B')
                ),
            "Git diff returned an unsupported path status"
        );
        let path_count = if status.starts_with('R') || status.starts_with('C') {
            2
        } else {
            1
        };
        for _ in 0..path_count {
            let raw_path = fields
                .next()
                .context("Git diff path status omitted a path")?;
            let path = std::str::from_utf8(raw_path)
                .context("Git diff path was not UTF-8")?
                .to_owned();
            ensure!(!path.is_empty(), "Git diff returned an empty path");
            if paths.len() < maximum {
                paths.push(path);
            } else {
                truncated = true;
            }
        }
    }
    Ok((paths, truncated))
}

fn is_hex_commit(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn first_sanitized_line(value: &[u8]) -> String {
    sanitize_text(
        String::from_utf8_lossy(value)
            .lines()
            .next()
            .unwrap_or_default(),
        256,
    )
}

fn command_summary(result: &CommandResult) -> String {
    if result.timed_out {
        return "timed out and terminated with its process group".to_owned();
    }
    let mut summary = format!(
        "exit {}; bounded output{}",
        result
            .exit_code
            .map_or_else(|| "signal".to_owned(), |code| code.to_string()),
        if result.truncated {
            " truncated"
        } else {
            " captured"
        }
    );
    if !result.success {
        let diagnostic = if !result.stderr.is_empty() {
            &result.stderr
        } else {
            &result.stdout
        };
        let diagnostic = first_sanitized_line(diagnostic);
        if !diagnostic.is_empty() {
            summary.push_str(": ");
            summary.push_str(&diagnostic);
        }
    }
    sanitize_text(&summary, 768)
}

fn validation_command_summary(result: &CommandResult) -> String {
    // Validation executes source-controlled build and test logic. Its output may contain arbitrary
    // workspace, HOME, or toolchain data that cannot be made safe with pattern-based redaction.
    // Receipts therefore expose only bounded execution metadata and never stdout or stderr.
    format!(
        "{}; exit {}; timedOut={}; outputTruncated={}; stdout/stderr withheld",
        if result.success { "passed" } else { "failed" },
        result
            .exit_code
            .map_or_else(|| "signal".to_owned(), |code| code.to_string()),
        result.timed_out,
        result.truncated,
    )
}

fn extract_pr_url(value: &str, owner: &str, repository: &str) -> Option<String> {
    let prefix = format!("https://github.com/{owner}/{repository}/pull/");
    value.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|character: char| {
            matches!(character, '"' | '\'' | '(' | ')' | '[' | ']' | ',' | '.')
        });
        let number = token.strip_prefix(&prefix)?;
        (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| token.to_owned())
    })
}

fn json_receipt<T: Serialize>(ok: bool, summary: &str, value: &T) -> ToolReceipt {
    let raw = serde_json::to_string_pretty(value)
        .unwrap_or_else(|_| "{\"error\":\"receipt serialization failed\"}".to_owned());
    let truncated = raw.len() > MAX_RECEIPT_BYTES;
    ToolReceipt {
        tool: "repository_maintenance".to_owned(),
        ok,
        summary: summary.to_owned(),
        output: sanitize_text(&raw, MAX_RECEIPT_BYTES),
        exit_code: None,
        timed_out: false,
        truncated,
    }
}

fn json_receipt_value(ok: bool, summary: &str, value: serde_json::Value) -> Result<ToolReceipt> {
    let raw = serde_json::to_string_pretty(&value)?;
    let truncated = raw.len() > MAX_RECEIPT_BYTES;
    Ok(ToolReceipt {
        tool: "repository_maintenance".to_owned(),
        ok,
        summary: summary.to_owned(),
        output: sanitize_text(&raw, MAX_RECEIPT_BYTES),
        exit_code: None,
        timed_out: false,
        truncated,
    })
}

async fn run_bounded_command<I, S>(
    program: &Path,
    args: I,
    cwd: &Path,
    limit: Duration,
    capability: CommandCapability,
    command_path: &OsStr,
) -> Result<CommandResult>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("PATH", command_path);
    for name in [
        "HOME",
        "USER",
        "LOGNAME",
        "LANG",
        "LC_ALL",
        "TERM",
        "TMPDIR",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    if matches!(
        capability,
        CommandCapability::Local | CommandCapability::GitNetwork | CommandCapability::GithubCli
    ) && let Some(value) = std::env::var_os("XDG_CONFIG_HOME")
    {
        command.env("XDG_CONFIG_HOME", value);
    }
    if matches!(capability, CommandCapability::GitNetwork)
        && let Some(value) = std::env::var_os("SSH_AUTH_SOCK")
    {
        command.env("SSH_AUTH_SOCK", value);
    }
    if matches!(capability, CommandCapability::GithubCli) {
        for name in ["GH_CONFIG_DIR", "GH_HOST", "GH_TOKEN", "GITHUB_TOKEN"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("starting bounded maintenance program {}", program.display()))?;
    let process_id = child.id();
    let stdout = child
        .stdout
        .take()
        .context("maintenance stdout was not piped")?;
    let stderr = child
        .stderr
        .take()
        .context("maintenance stderr was not piped")?;
    let stdout_task = tokio::spawn(capture_bounded(stdout));
    let stderr_task = tokio::spawn(capture_bounded(stderr));
    let (status, timed_out) = match timeout(limit, child.wait()).await {
        Ok(status) => (
            Some(status.context("waiting for maintenance command")?),
            false,
        ),
        Err(_) => {
            #[cfg(unix)]
            if let Some(process_id) = process_id {
                let _ = Command::new("/bin/kill")
                    .args(["-KILL", "--", &format!("-{process_id}")])
                    .status()
                    .await;
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
            (None, true)
        }
    };
    let (stdout, stdout_truncated) = stdout_task.await.context("joining maintenance stdout")??;
    let (stderr, stderr_truncated) = stderr_task.await.context("joining maintenance stderr")??;
    Ok(CommandResult {
        success: status
            .as_ref()
            .is_some_and(std::process::ExitStatus::success)
            && !timed_out,
        exit_code: status.as_ref().and_then(std::process::ExitStatus::code),
        stdout,
        stderr,
        timed_out,
        truncated: stdout_truncated || stderr_truncated,
    })
}

async fn capture_bounded<R: AsyncRead + Unpin>(mut reader: R) -> Result<(Vec<u8>, bool)> {
    let mut kept = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let remaining = MAX_COMMAND_OUTPUT_BYTES.saturating_sub(kept.len());
        let retained = remaining.min(count);
        kept.extend_from_slice(&buffer[..retained]);
        truncated |= retained < count;
    }
    Ok((kept, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;
    use tempfile::TempDir;

    struct GitFixture {
        _root: TempDir,
        canonical: PathBuf,
        fork: Option<PathBuf>,
        seed: PathBuf,
        checkout: PathBuf,
    }

    impl GitFixture {
        fn canonical() -> Self {
            Self::new(false)
        }

        fn fork() -> Self {
            Self::new(true)
        }

        fn new(with_fork: bool) -> Self {
            let root = TempDir::new().unwrap();
            let canonical = root.path().join("canonical.git");
            git(
                root.path(),
                &[
                    "init",
                    "--bare",
                    "--initial-branch=main",
                    canonical.to_str().unwrap(),
                ],
            );
            let seed = root.path().join("seed");
            fs::create_dir(&seed).unwrap();
            git(&seed, &["init", "--initial-branch=main"]);
            configure_identity(&seed);
            fs::write(seed.join("base.txt"), "base\n").unwrap();
            git(&seed, &["add", "--", "base.txt"]);
            git(&seed, &["commit", "-m", "base"]);
            git(
                &seed,
                &["remote", "add", "origin", canonical.to_str().unwrap()],
            );
            git(&seed, &["push", "-u", "origin", "main"]);

            let fork = with_fork.then(|| root.path().join("fork.git"));
            if let Some(fork) = &fork {
                git(
                    root.path(),
                    &[
                        "clone",
                        "--bare",
                        canonical.to_str().unwrap(),
                        fork.to_str().unwrap(),
                    ],
                );
            }
            let checkout = root.path().join("checkout");
            let clone_source = fork.as_ref().unwrap_or(&canonical);
            git(
                root.path(),
                &[
                    "clone",
                    clone_source.to_str().unwrap(),
                    checkout.to_str().unwrap(),
                ],
            );
            configure_identity(&checkout);
            if with_fork {
                git(
                    &checkout,
                    &["remote", "add", "upstream", canonical.to_str().unwrap()],
                );
            }
            Self {
                _root: root,
                canonical,
                fork,
                seed,
                checkout,
            }
        }

        fn upstream_commit(&self, path: &str, content: &str, message: &str) -> String {
            fs::write(self.seed.join(path), content).unwrap();
            git(&self.seed, &["add", "--", path]);
            git(&self.seed, &["commit", "-m", message]);
            git(&self.seed, &["push", "origin", "main"]);
            git_output(&self.seed, &["rev-parse", "HEAD"])
        }

        fn maintenance(&self, gh: PathBuf) -> RepositoryMaintenance {
            RepositoryMaintenance::for_local_test(&self.checkout, &self.canonical, gh).unwrap()
        }
    }

    fn git(cwd: &Path, args: &[&str]) {
        let output = StdCommand::new("git")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(args)
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout={}\nstderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_fails(cwd: &Path, args: &[&str]) -> String {
        let output = StdCommand::new("git")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(args)
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "git command unexpectedly succeeded"
        );
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn git_output(cwd: &Path, args: &[&str]) -> String {
        let output = StdCommand::new("git")
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(args)
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();
        assert!(output.status.success(), "git {:?} failed", args);
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn configure_identity(root: &Path) {
        git(root, &["config", "user.name", "Cthuwu Test"]);
        git(
            root,
            &["config", "user.email", "cthuwu-test@example.invalid"],
        );
    }

    fn missing_gh(root: &Path) -> PathBuf {
        root.join("definitely-missing-gh")
    }

    fn fake_gh(root: &Path) -> PathBuf {
        let path = root.join("gh");
        fs::write(
            &path,
            "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'gh version 9.9.9'; exit 0 ;;\n  auth) exit 0 ;;\n  pr) echo 'https://github.com/canonical/cthuwu/pull/42'; exit 0 ;;\n  *) exit 2 ;;\nesac\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }

    fn failing_pr_gh(root: &Path) -> PathBuf {
        let path = root.join("failing-pr-gh");
        fs::write(
            &path,
            "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'gh version 9.9.9'; exit 0 ;;\n  auth) exit 0 ;;\n  pr) echo 'transient create failure' >&2; exit 17 ;;\n  *) exit 2 ;;\nesac\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }

    fn recording_gh(root: &Path, marker: &Path) -> PathBuf {
        let path = root.join("recording-gh");
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf called > '{}'\nexit 1\n",
                marker.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }

    fn validation_env_probe(root: &Path) -> PathBuf {
        let path = root.join("validation-env-probe");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        use std::io::Write as _;
        file.write_all(
            b"#!/bin/sh\nprintf 'GH_TOKEN=%s GITHUB_TOKEN=%s SSH_AUTH_SOCK=%s GH_CONFIG_DIR=%s' \"${GH_TOKEN-unset}\" \"${GITHUB_TOKEN-unset}\" \"${SSH_AUTH_SOCK-unset}\" \"${GH_CONFIG_DIR-unset}\"\n",
        )
        .unwrap();
        file.sync_all().unwrap();
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }

    fn commit_validation_script(fixture: &GitFixture, body: &str) {
        let scripts = fixture.checkout.join("scripts");
        fs::create_dir_all(&scripts).unwrap();
        let path = scripts.join("test-install.sh");
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        git(&fixture.checkout, &["add", "--", "scripts/test-install.sh"]);
        git(
            &fixture.checkout,
            &["commit", "-m", "test: install validation fixture"],
        );
        git(&fixture.checkout, &["push", "origin", "main"]);
    }

    #[tokio::test]
    async fn clean_canonical_checkout_fast_forwards_to_historical_upstream_head() {
        let fixture = GitFixture::canonical();
        let old = git_output(&fixture.checkout, &["rev-parse", "HEAD"]);
        let expected =
            fixture.upstream_commit("upstream.txt", "upstream\n", "canonical upstream change");
        let maintenance = fixture.maintenance(missing_gh(fixture._root.path()));
        let receipt = maintenance
            .update_receipt(Instant::now() + Duration::from_secs(30), true)
            .await
            .unwrap();
        assert!(receipt.ok, "{}", receipt.summary);
        assert_ne!(old, expected);
        assert_eq!(
            git_output(&fixture.checkout, &["rev-parse", "HEAD"]),
            expected
        );
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("upstream.txt")).unwrap(),
            "upstream\n"
        );
        assert!(receipt.output.contains("\"runningProcessUpdated\": false"));
    }

    #[tokio::test]
    async fn validation_child_receives_no_github_or_ssh_auth_environment() {
        let fixture = GitFixture::canonical();
        let validation_path = sanitized_command_path(
            &fixture.checkout,
            &std::env::var_os("PATH").unwrap_or_default(),
            ExecutableTrust::Validation,
            &[],
        )
        .unwrap();
        let result = run_bounded_command(
            &validation_env_probe(fixture._root.path()),
            std::iter::empty::<&str>(),
            &fixture.checkout,
            Duration::from_secs(5),
            CommandCapability::Validation,
            &validation_path,
        )
        .await
        .unwrap();

        assert!(result.success);
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            "GH_TOKEN=unset GITHUB_TOKEN=unset SSH_AUTH_SOCK=unset GH_CONFIG_DIR=unset"
        );
    }

    #[tokio::test]
    async fn validation_receipts_never_include_unrecognized_child_output() {
        const OPAQUE_PRIVATE_MATERIAL: &str =
            "CthuwuWalletMaterial::violet-lantern-7492-unrecognized";
        let fixture = GitFixture::canonical();
        commit_validation_script(
            &fixture,
            &format!(
                "printf '%s\\n' '{OPAQUE_PRIVATE_MATERIAL}' >&2\nprintf '%s\\n' '{OPAQUE_PRIVATE_MATERIAL}'\nexit 19"
            ),
        );
        let maintenance = fixture.maintenance(missing_gh(fixture._root.path()));

        let steps = maintenance
            .run_validation_ids(
                Instant::now() + Duration::from_secs(10),
                vec![ValidationId::InstallTest],
                true,
            )
            .await
            .unwrap();
        let receipt =
            json_receipt_value(false, "validation failed", json!({"steps": steps})).unwrap();

        assert!(!receipt.output.contains(OPAQUE_PRIVATE_MATERIAL));
        assert!(receipt.output.contains("exit 19"));
        assert!(receipt.output.contains("stdout/stderr withheld"));
    }

    #[test]
    fn workspace_path_hijacks_cannot_resolve_git_gh_or_validation_programs() {
        let fixture = GitFixture::canonical();
        let bin = fixture.checkout.join("bin");
        fs::create_dir(&bin).unwrap();
        for name in ["git", "gh", "cargo"] {
            let executable = bin.join(name);
            fs::write(&executable, "#!/bin/sh\nprintf called > credential-leak\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let search_path = std::env::join_paths([bin.as_path()]).unwrap();

        for (name, trust) in [
            ("git", ExecutableTrust::Authentication),
            ("gh", ExecutableTrust::Authentication),
            ("cargo", ExecutableTrust::Validation),
        ] {
            assert!(
                resolve_executable_in_path(name, &fixture.checkout, &search_path, trust)
                    .unwrap()
                    .is_none()
            );
        }
        assert!(!fixture.checkout.join("credential-leak").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn auth_bearing_children_never_search_a_later_workspace_path_entry() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = GitFixture::canonical();
        let bin = fixture.checkout.join("bin");
        fs::create_dir(&bin).unwrap();
        for name in ["ssh", "gh"] {
            let executable = bin.join(name);
            fs::write(
                &executable,
                "#!/bin/sh\nprintf called >> fake-auth-helper-called\nprintf '%s\\n' \"${GH_TOKEN-}${GITHUB_TOKEN-}${SSH_AUTH_SOCK-}\"\n",
            )
            .unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let system_git = fs::canonicalize("/usr/local/bin/git")
            .or_else(|_| fs::canonicalize("/usr/bin/git"))
            .unwrap();
        let system_bin = system_git.parent().unwrap();
        let search_path = std::env::join_paths([system_bin, bin.as_path()]).unwrap();
        let pinned_git = resolve_executable_in_path(
            "git",
            &fixture.checkout,
            &search_path,
            ExecutableTrust::Authentication,
        )
        .unwrap()
        .unwrap();
        let authentication_path = sanitized_command_path(
            &fixture.checkout,
            &search_path,
            ExecutableTrust::Authentication,
            &[pinned_git],
        )
        .unwrap();
        assert!(
            std::env::split_paths(&authentication_path)
                .all(|directory| !directory.starts_with(&fixture.checkout))
        );

        for (capability, command) in [
            (
                CommandCapability::GitNetwork,
                "ssh -V >/dev/null 2>&1 || true",
            ),
            (
                CommandCapability::GithubCli,
                "gh --version >/dev/null 2>&1 || true",
            ),
        ] {
            let result = run_bounded_command(
                Path::new("/bin/sh"),
                ["-c", command],
                &fixture.checkout,
                Duration::from_secs(5),
                capability,
                &authentication_path,
            )
            .await
            .unwrap();
            assert!(result.success);
            assert!(!result.stdout.windows(6).any(|window| window == b"called"));
            assert!(!result.stderr.windows(6).any(|window| window == b"called"));
        }
        assert!(!fixture.checkout.join("fake-auth-helper-called").exists());
    }

    #[cfg(unix)]
    #[test]
    fn trusted_symlink_target_is_pinned_absolute_and_optional_gh_may_be_absent() {
        use std::os::unix::fs::symlink;

        let fixture = GitFixture::canonical();
        let links = fixture._root.path().join("trusted-links");
        fs::create_dir(&links).unwrap();
        let system_git = fs::canonicalize("/usr/local/bin/git")
            .or_else(|_| fs::canonicalize("/usr/bin/git"))
            .unwrap();
        symlink(&system_git, links.join("git")).unwrap();
        let search_path = std::env::join_paths([links.as_path()]).unwrap();

        let resolved = resolve_executable_in_path(
            "git",
            &fixture.checkout,
            &search_path,
            ExecutableTrust::Authentication,
        )
        .unwrap();
        assert_eq!(resolved.as_deref(), Some(system_git.as_path()));
        assert!(
            resolve_executable_in_path(
                "gh",
                &fixture.checkout,
                &search_path,
                ExecutableTrust::Authentication,
            )
            .unwrap()
            .is_none()
        );
    }

    #[tokio::test]
    async fn dirty_checkout_is_refused_and_preserved_before_fetch_or_merge() {
        let fixture = GitFixture::canonical();
        fixture.upstream_commit("upstream.txt", "upstream\n", "upstream change");
        let local = fixture.checkout.join("local.txt");
        fs::write(&local, "intentional local work\n").unwrap();
        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Update)
            .await;
        assert!(!receipt.ok);
        assert!(receipt.summary.contains("working tree is dirty"));
        assert_eq!(
            fs::read_to_string(local).unwrap(),
            "intentional local work\n"
        );
        assert!(!fixture.checkout.join("upstream.txt").exists());
    }

    #[tokio::test]
    async fn diverged_canonical_checkout_preserves_intentional_local_commits() {
        let fixture = GitFixture::canonical();
        fs::write(fixture.checkout.join("local.txt"), "intentional commit\n").unwrap();
        git(&fixture.checkout, &["add", "--", "local.txt"]);
        git(
            &fixture.checkout,
            &["commit", "-m", "intentional local commit"],
        );
        let local_head = git_output(&fixture.checkout, &["rev-parse", "HEAD"]);
        fixture.upstream_commit("upstream.txt", "upstream\n", "upstream change");

        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Update)
            .await;

        assert!(!receipt.ok);
        assert!(receipt.summary.contains("canonical checkout diverged"));
        assert_eq!(
            git_output(&fixture.checkout, &["rev-parse", "HEAD"]),
            local_head
        );
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("local.txt")).unwrap(),
            "intentional commit\n"
        );
        assert!(!fixture.checkout.join("upstream.txt").exists());
    }

    #[tokio::test]
    async fn divergence_receipts_never_include_commit_subject_text() {
        const LOCAL_PRIVATE_SUBJECT: &str = "local opaque wallet material violet-lantern-9821";
        const UPSTREAM_PRIVATE_SUBJECT: &str = "upstream opaque api material copper-orchid-4470";
        let fixture = GitFixture::canonical();
        fs::write(fixture.checkout.join("local-secret.txt"), "local\n").unwrap();
        git(&fixture.checkout, &["add", "--", "local-secret.txt"]);
        git(&fixture.checkout, &["commit", "-m", LOCAL_PRIVATE_SUBJECT]);
        fixture.upstream_commit(
            "upstream-secret.txt",
            "upstream\n",
            UPSTREAM_PRIVATE_SUBJECT,
        );

        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Update)
            .await;

        assert!(!receipt.ok);
        assert!(!receipt.output.contains(LOCAL_PRIVATE_SUBJECT));
        assert!(!receipt.output.contains(UPSTREAM_PRIVATE_SUBJECT));
        assert!(receipt.output.contains("local_hashes"));
        assert!(receipt.output.contains("upstream_hashes"));
        assert!(receipt.output.contains("local_count"));
        assert!(receipt.output.contains("upstream_count"));
    }

    #[tokio::test]
    async fn typed_merge_integrates_verified_upstream_into_a_fork_without_publishing() {
        let fixture = GitFixture::fork();
        fs::write(fixture.checkout.join("fork-local.txt"), "fork work\n").unwrap();
        git(&fixture.checkout, &["add", "--", "fork-local.txt"]);
        git(&fixture.checkout, &["commit", "-m", "fork-specific work"]);
        let fork = fixture.fork.as_ref().unwrap();
        let fork_published_before = git_output(fork, &["rev-parse", "refs/heads/main"]);
        fixture.upstream_commit(
            "canonical-change.txt",
            "canonical work\n",
            "canonical upstream work",
        );

        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Merge {
                remote: "upstream".to_owned(),
                branch: "main".to_owned(),
            })
            .await;

        assert!(receipt.ok, "{}\n{}", receipt.summary, receipt.output);
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("fork-local.txt")).unwrap(),
            "fork work\n"
        );
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("canonical-change.txt")).unwrap(),
            "canonical work\n"
        );
        assert_eq!(
            git_output(fork, &["rev-parse", "refs/heads/main"]),
            fork_published_before
        );
        assert_eq!(
            git_output(
                &fixture.checkout,
                &["rev-list", "--parents", "-n", "1", "HEAD"]
            )
            .split_whitespace()
            .count(),
            3
        );
    }

    #[tokio::test]
    async fn typed_merge_allows_explicit_canonical_divergence_integration() {
        let fixture = GitFixture::canonical();
        fs::write(
            fixture.checkout.join("local.txt"),
            "intentional local commit\n",
        )
        .unwrap();
        git(&fixture.checkout, &["add", "--", "local.txt"]);
        git(
            &fixture.checkout,
            &["commit", "-m", "intentional local commit"],
        );
        let canonical_head = fixture.upstream_commit(
            "canonical-change.txt",
            "canonical change\n",
            "canonical remote commit",
        );

        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Merge {
                remote: "origin".to_owned(),
                branch: "main".to_owned(),
            })
            .await;

        assert!(receipt.ok, "{}\n{}", receipt.summary, receipt.output);
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("local.txt")).unwrap(),
            "intentional local commit\n"
        );
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("canonical-change.txt")).unwrap(),
            "canonical change\n"
        );
        assert_eq!(
            git_output(&fixture.canonical, &["rev-parse", "refs/heads/main"]),
            canonical_head
        );
        assert_eq!(
            git_output(
                &fixture.checkout,
                &["rev-list", "--parents", "-n", "1", "HEAD"]
            )
            .split_whitespace()
            .count(),
            3
        );
    }

    #[tokio::test]
    async fn scoped_commit_refuses_and_preserves_an_unrelated_prestaged_change() {
        let fixture = GitFixture::canonical();
        fs::write(fixture.checkout.join("requested.txt"), "requested\n").unwrap();
        fs::write(fixture.checkout.join("unrelated.txt"), "unrelated\n").unwrap();
        git(&fixture.checkout, &["add", "--", "unrelated.txt"]);
        let old_head = git_output(&fixture.checkout, &["rev-parse", "HEAD"]);

        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Commit {
                message: "fix: requested scope".to_owned(),
                paths: vec!["requested.txt".to_owned()],
                topic_branch: None,
            })
            .await;

        assert!(!receipt.ok);
        assert!(receipt.summary.contains("unrelated dirty path"));
        assert_eq!(
            git_output(&fixture.checkout, &["rev-parse", "HEAD"]),
            old_head
        );
        assert_eq!(
            git_output(&fixture.checkout, &["diff", "--cached", "--name-only"]),
            "unrelated.txt"
        );
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("requested.txt")).unwrap(),
            "requested\n"
        );
    }

    #[tokio::test]
    async fn scoped_commit_rejects_a_whole_repository_current_directory_scope() {
        let fixture = GitFixture::canonical();
        fs::write(fixture.checkout.join("requested.txt"), "requested\n").unwrap();
        let old_head = git_output(&fixture.checkout, &["rev-parse", "HEAD"]);

        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Commit {
                message: "fix: unsafe root scope".to_owned(),
                paths: vec![".".to_owned()],
                topic_branch: None,
            })
            .await;

        assert!(!receipt.ok);
        assert!(receipt.summary.contains("current-directory scope"));
        assert_eq!(
            git_output(&fixture.checkout, &["rev-parse", "HEAD"]),
            old_head
        );
        assert!(git_output(&fixture.checkout, &["diff", "--cached", "--name-only"]).is_empty());
    }

    #[tokio::test]
    async fn long_lived_fork_merges_upstream_and_pushes_only_after_validation() {
        let fixture = GitFixture::fork();
        fs::write(fixture.checkout.join("fork.txt"), "fork work\n").unwrap();
        git(&fixture.checkout, &["add", "--", "fork.txt"]);
        git(&fixture.checkout, &["commit", "-m", "fork change"]);
        git(&fixture.checkout, &["push", "origin", "main"]);
        fixture.upstream_commit("upstream.txt", "upstream work\n", "upstream change");

        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .update_receipt(Instant::now() + Duration::from_secs(30), true)
            .await
            .unwrap();
        assert!(receipt.ok, "{}\n{}", receipt.summary, receipt.output);
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("fork.txt")).unwrap(),
            "fork work\n"
        );
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("upstream.txt")).unwrap(),
            "upstream work\n"
        );
        let fork = fixture.fork.as_ref().unwrap();
        let fork_head = git_output(fork, &["rev-parse", "refs/heads/main"]);
        assert_eq!(
            fork_head,
            git_output(&fixture.checkout, &["rev-parse", "HEAD"])
        );
        assert!(receipt.output.contains("\"forkPushed\": true"));
    }

    #[tokio::test]
    async fn fork_conflict_is_reported_and_left_for_deliberate_resolution() {
        let fixture = GitFixture::fork();
        fs::write(fixture.checkout.join("base.txt"), "fork\n").unwrap();
        git(&fixture.checkout, &["add", "--", "base.txt"]);
        git(&fixture.checkout, &["commit", "-m", "fork edits base"]);
        git(&fixture.checkout, &["push", "origin", "main"]);
        fixture.upstream_commit("base.txt", "upstream\n", "upstream edits base");

        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .update_receipt(Instant::now() + Duration::from_secs(30), true)
            .await
            .unwrap();
        assert!(!receipt.ok);
        assert!(receipt.output.contains("base.txt"));
        assert!(receipt.output.contains("conflictedFiles"));
        assert!(
            fs::read_to_string(fixture.checkout.join("base.txt"))
                .unwrap()
                .contains("<<<<<<<")
        );
        assert!(
            git_output(
                &fixture.checkout,
                &["diff", "--name-only", "--diff-filter=U"]
            )
            .contains("base.txt")
        );
    }

    #[tokio::test]
    async fn fork_update_rechecks_branch_and_head_after_source_controlled_validation() {
        let fixture = GitFixture::fork();
        commit_validation_script(&fixture, "git switch -c validation-hijack >/dev/null");
        fixture.upstream_commit("upstream.txt", "upstream\n", "upstream change");
        let fork = fixture.fork.as_ref().unwrap();
        let published_before = git_output(fork, &["rev-parse", "refs/heads/main"]);
        let mut maintenance = fixture.maintenance(missing_gh(fixture._root.path()));
        maintenance.policy.update_validation = vec![ValidationId::InstallTest];

        let receipt = maintenance
            .update_receipt(Instant::now() + Duration::from_secs(30), true)
            .await
            .unwrap();

        assert!(!receipt.ok);
        assert!(
            receipt
                .output
                .contains("validation changed the checked-out branch")
        );
        assert_eq!(
            git_output(fork, &["rev-parse", "refs/heads/main"]),
            published_before
        );
        assert_eq!(
            git_output(&fixture.checkout, &["branch", "--show-current"]),
            "validation-hijack"
        );
    }

    #[tokio::test]
    async fn pr_rechecks_remote_configuration_after_source_controlled_validation() {
        let fixture = GitFixture::fork();
        let attacker = fixture._root.path().join("attacker.git");
        git(
            fixture._root.path(),
            &[
                "init",
                "--bare",
                "--initial-branch=main",
                attacker.to_str().unwrap(),
            ],
        );
        commit_validation_script(
            &fixture,
            &format!("git remote set-url origin '{}'", attacker.display()),
        );
        fs::write(fixture.checkout.join("fix.txt"), "scoped fix\n").unwrap();
        let mut maintenance = fixture.maintenance(fake_gh(fixture._root.path()));
        maintenance.policy.pr_validation = vec![ValidationId::InstallTest];

        let receipt = maintenance
            .execute(RepositoryMaintenanceRequest::Pr {
                branch: "fix/remote-recheck".to_owned(),
                title: "Recheck remote state".to_owned(),
                body: "Exercise the post-validation remote gate.".to_owned(),
                commit_message: "fix: remote recheck".to_owned(),
                paths: vec!["fix.txt".to_owned(), "scripts/test-install.sh".to_owned()],
                base: None,
            })
            .await;

        assert!(!receipt.ok);
        assert!(receipt.summary.contains("changed repository remotes"));
        let fork = fixture.fork.as_ref().unwrap();
        git_fails(
            fork,
            &["rev-parse", "--verify", "refs/heads/fix/remote-recheck"],
        );
        git_fails(
            &attacker,
            &["rev-parse", "--verify", "refs/heads/fix/remote-recheck"],
        );
    }

    #[tokio::test]
    async fn scoped_commit_rejects_a_rename_whose_source_is_outside_scope() {
        let fixture = GitFixture::canonical();
        fs::create_dir(fixture.checkout.join("scoped")).unwrap();
        git(
            &fixture.checkout,
            &["mv", "--", "base.txt", "scoped/base.txt"],
        );
        let old_head = git_output(&fixture.checkout, &["rev-parse", "HEAD"]);

        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Commit {
                message: "fix: unsafe partial rename scope".to_owned(),
                paths: vec!["scoped".to_owned()],
                topic_branch: None,
            })
            .await;

        assert!(!receipt.ok);
        assert!(receipt.summary.contains("unrelated dirty path"));
        assert_eq!(
            git_output(&fixture.checkout, &["rev-parse", "HEAD"]),
            old_head
        );
        assert!(!fixture.checkout.join("base.txt").exists());
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("scoped/base.txt")).unwrap(),
            "base\n"
        );
    }

    #[tokio::test]
    async fn scoped_commit_refuses_a_truncated_dirty_path_review() {
        let fixture = GitFixture::canonical();
        let bulk = fixture.checkout.join("bulk");
        fs::create_dir(&bulk).unwrap();
        for index in 0..=MAX_DIRTY_ENTRIES {
            fs::write(bulk.join(format!("file-{index:03}.txt")), "dirty\n").unwrap();
        }

        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Commit {
                message: "fix: bounded dirty review".to_owned(),
                paths: vec!["bulk".to_owned()],
                topic_branch: None,
            })
            .await;

        assert!(!receipt.ok);
        assert!(receipt.summary.contains("too many dirty paths"));
        assert!(git_output(&fixture.checkout, &["diff", "--cached", "--name-only"]).is_empty());
    }

    #[test]
    fn truncated_staged_path_review_fails_closed_before_scope_comparison() {
        let mut raw = Vec::new();
        for index in 0..MAX_PATHS {
            raw.extend_from_slice(format!("scoped/file-{index:03}.txt\0").as_bytes());
        }
        raw.extend_from_slice(b"outside.txt\0");
        let (staged, truncated) = parse_nul_paths(&raw, MAX_PATHS);

        let error =
            ensure_staged_is_scoped(&staged, truncated, &["scoped".to_owned()]).unwrap_err();

        assert!(format!("{error:#}").contains("bounded review limit"));
    }

    #[test]
    fn canonical_diff_parser_preserves_both_rename_and_copy_paths() {
        let raw = b"R100\0outside-old.txt\0scoped/new.txt\0C075\0copy-source.txt\0scoped/copy.txt\0M\0scoped/edited.txt\0";
        let (paths, truncated) = parse_name_status_paths(raw, MAX_PATHS).unwrap();

        assert!(!truncated);
        assert_eq!(
            paths,
            vec![
                "outside-old.txt",
                "scoped/new.txt",
                "copy-source.txt",
                "scoped/copy.txt",
                "scoped/edited.txt",
            ]
        );
        let error =
            ensure_pr_diff_is_scoped(&paths, false, &["scoped".to_owned()], true).unwrap_err();
        assert!(format!("{error:#}").contains("outside the explicit PR scope"));
    }

    #[tokio::test]
    async fn standalone_push_refuses_the_canonical_upstream_remote() {
        let fixture = GitFixture::canonical();
        let published_before = git_output(&fixture.canonical, &["rev-parse", "refs/heads/main"]);
        fs::write(
            fixture.checkout.join("local.txt"),
            "intentional local commit\n",
        )
        .unwrap();
        git(&fixture.checkout, &["add", "--", "local.txt"]);
        git(&fixture.checkout, &["commit", "-m", "local commit"]);

        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Push {
                remote: "origin".to_owned(),
                branch: "main".to_owned(),
            })
            .await;

        assert!(!receipt.ok);
        assert!(receipt.summary.contains("canonical upstream repository"));
        assert_eq!(
            git_output(&fixture.canonical, &["rev-parse", "refs/heads/main"]),
            published_before
        );
    }

    #[tokio::test]
    async fn pull_request_refuses_a_canonical_origin_before_changing_local_state() {
        let fixture = GitFixture::canonical();
        fs::write(fixture.checkout.join("fix.txt"), "uncommitted scoped fix\n").unwrap();

        let receipt = fixture
            .maintenance(fake_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Pr {
                branch: "fix/canonical-origin-refusal".to_owned(),
                title: "Must use an operator fork".to_owned(),
                body: "The canonical checkout must remain untouched.".to_owned(),
                commit_message: "fix: should not be committed".to_owned(),
                paths: vec!["fix.txt".to_owned()],
                base: None,
            })
            .await;

        assert!(!receipt.ok);
        assert!(receipt.summary.contains("canonical upstream repository"));
        assert!(receipt.summary.contains("noncanonical operator fork"));
        assert_eq!(
            git_output(&fixture.checkout, &["branch", "--show-current"]),
            "main"
        );
        assert!(git_output(&fixture.checkout, &["diff", "--cached", "--name-only"]).is_empty());
        git_fails(
            &fixture.checkout,
            &[
                "show-ref",
                "--verify",
                "refs/heads/fix/canonical-origin-refusal",
            ],
        );
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("fix.txt")).unwrap(),
            "uncommitted scoped fix\n"
        );
    }

    #[tokio::test]
    async fn mismatched_fetch_and_push_repository_identities_fail_before_network_access() {
        let fixture = GitFixture::fork();
        git(
            &fixture.checkout,
            &[
                "remote",
                "set-url",
                "--push",
                "origin",
                fixture.canonical.to_str().unwrap(),
            ],
        );
        let maintenance = fixture.maintenance(missing_gh(fixture._root.path()));

        let fetch = maintenance
            .execute(RepositoryMaintenanceRequest::Fetch)
            .await;
        assert!(!fetch.ok);
        assert!(fetch.summary.contains("identify different repositories"));

        let push = maintenance
            .execute(RepositoryMaintenanceRequest::Push {
                remote: "origin".to_owned(),
                branch: "main".to_owned(),
            })
            .await;
        assert!(!push.ok);
        assert!(push.summary.contains("identify different repositories"));

        let update = maintenance
            .execute(RepositoryMaintenanceRequest::Update)
            .await;
        assert!(!update.ok);
        assert!(update.summary.contains("identify different repositories"));

        fs::write(fixture.checkout.join("mismatch-fix.txt"), "preserved\n").unwrap();
        let pull_request = maintenance
            .execute(RepositoryMaintenanceRequest::Pr {
                branch: "fix/mismatched-remote".to_owned(),
                title: "Reject mismatched remote".to_owned(),
                body: "No local PR preparation may happen before remote verification.".to_owned(),
                commit_message: "fix: should remain uncommitted".to_owned(),
                paths: vec!["mismatch-fix.txt".to_owned()],
                base: None,
            })
            .await;
        assert!(!pull_request.ok);
        assert!(
            pull_request
                .summary
                .contains("identify different repositories")
        );
        assert_eq!(
            git_output(&fixture.checkout, &["branch", "--show-current"]),
            "main"
        );
    }

    #[tokio::test]
    async fn pull_request_refuses_unrelated_preexisting_fork_commits() {
        let fixture = GitFixture::fork();
        fs::write(
            fixture.checkout.join("fork-only.txt"),
            "unrelated fork work\n",
        )
        .unwrap();
        git(&fixture.checkout, &["add", "--", "fork-only.txt"]);
        git(&fixture.checkout, &["commit", "-m", "fork-only commit"]);
        git(&fixture.checkout, &["push", "origin", "main"]);
        let local_head = git_output(&fixture.checkout, &["rev-parse", "HEAD"]);
        let fork = fixture.fork.as_ref().unwrap();
        let fork_head = git_output(fork, &["rev-parse", "refs/heads/main"]);
        fs::write(fixture.checkout.join("requested.txt"), "requested fix\n").unwrap();
        let gh_marker = fixture._root.path().join("gh-was-called");
        let maintenance = fixture.maintenance(recording_gh(fixture._root.path(), &gh_marker));

        let receipt = maintenance
            .execute(RepositoryMaintenanceRequest::Pr {
                branch: "fix/scoped-with-fork-divergence".to_owned(),
                title: "Scoped fix".to_owned(),
                body: "Only the requested file may enter this PR.".to_owned(),
                commit_message: "fix: requested scope".to_owned(),
                paths: vec!["requested.txt".to_owned()],
                base: None,
            })
            .await;

        assert!(!receipt.ok);
        assert!(receipt.summary.contains("outside the explicit PR scope"));
        assert_eq!(
            git_output(&fixture.checkout, &["branch", "--show-current"]),
            "main"
        );
        assert_eq!(
            git_output(&fixture.checkout, &["rev-parse", "HEAD"]),
            local_head
        );
        assert_eq!(
            git_output(fork, &["rev-parse", "refs/heads/main"]),
            fork_head
        );
        git_fails(
            fork,
            &[
                "rev-parse",
                "--verify",
                "refs/heads/fix/scoped-with-fork-divergence",
            ],
        );
        assert!(!gh_marker.exists());
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("requested.txt")).unwrap(),
            "requested fix\n"
        );
    }

    #[tokio::test]
    async fn pull_request_rejects_an_empty_explicit_path_scope() {
        let fixture = GitFixture::fork();
        let receipt = fixture
            .maintenance(fake_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Pr {
                branch: "fix/empty-scope".to_owned(),
                title: "Empty scope must fail".to_owned(),
                body: "No arbitrary existing commit may be published.".to_owned(),
                commit_message: "fix: impossible empty scope".to_owned(),
                paths: Vec::new(),
                base: None,
            })
            .await;

        assert!(!receipt.ok);
        assert!(receipt.summary.contains("nonempty explicit path scope"));
        assert_eq!(
            git_output(&fixture.checkout, &["branch", "--show-current"]),
            "main"
        );
    }

    #[tokio::test]
    async fn missing_gh_retry_reuses_prepared_topic_head_without_an_extra_commit() {
        let fixture = GitFixture::fork();
        fs::write(fixture.checkout.join("fix.txt"), "scoped fix\n").unwrap();
        let maintenance = fixture.maintenance(missing_gh(fixture._root.path()));
        let receipt = maintenance
            .execute(RepositoryMaintenanceRequest::Pr {
                branch: "fix/scoped".to_owned(),
                title: "Fix scoped behavior".to_owned(),
                body: "This is a bounded test PR body.".to_owned(),
                commit_message: "fix: scoped behavior".to_owned(),
                paths: vec!["fix.txt".to_owned()],
                base: None,
            })
            .await;
        assert!(!receipt.ok);
        assert!(receipt.summary.contains("gh is not installed"));
        assert_eq!(
            git_output(&fixture.checkout, &["branch", "--show-current"]),
            "fix/scoped"
        );
        assert_eq!(
            git_output(&fixture.checkout, &["log", "-1", "--pretty=%s"]),
            "fix: scoped behavior"
        );
        let prepared_head = git_output(&fixture.checkout, &["rev-parse", "HEAD"]);
        let prepared_commit_count = git_output(&fixture.checkout, &["rev-list", "--count", "HEAD"]);
        let fork = fixture.fork.as_ref().unwrap();
        git_fails(fork, &["rev-parse", "--verify", "refs/heads/fix/scoped"]);

        let retry = fixture
            .maintenance(fake_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Pr {
                branch: "fix/scoped".to_owned(),
                title: "Fix scoped behavior".to_owned(),
                body: "This is a bounded test PR body.".to_owned(),
                commit_message: "fix: scoped behavior".to_owned(),
                paths: vec!["fix.txt".to_owned()],
                base: None,
            })
            .await;

        assert!(retry.ok, "{}\n{}", retry.summary, retry.output);
        assert!(retry.output.contains("reuse prepared topic HEAD"));
        assert_eq!(
            git_output(&fixture.checkout, &["rev-parse", "HEAD"]),
            prepared_head
        );
        assert_eq!(
            git_output(&fixture.checkout, &["rev-list", "--count", "HEAD"]),
            prepared_commit_count
        );
        assert_eq!(
            git_output(fork, &["rev-parse", "refs/heads/fix/scoped"]),
            prepared_head
        );
    }

    #[tokio::test]
    async fn transient_gh_create_failure_can_retry_the_pushed_head_without_recommit() {
        let fixture = GitFixture::fork();
        fs::write(fixture.checkout.join("retry.txt"), "retry-safe fix\n").unwrap();
        let first = fixture
            .maintenance(failing_pr_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Pr {
                branch: "fix/transient-gh".to_owned(),
                title: "Retry transient gh failure".to_owned(),
                body: "The prepared and pushed HEAD must be reusable.".to_owned(),
                commit_message: "fix: retry transient gh".to_owned(),
                paths: vec!["retry.txt".to_owned()],
                base: None,
            })
            .await;
        assert!(!first.ok);
        assert!(first.summary.contains("no PR is claimed"));
        let prepared_head = git_output(&fixture.checkout, &["rev-parse", "HEAD"]);
        let prepared_commit_count = git_output(&fixture.checkout, &["rev-list", "--count", "HEAD"]);
        let fork = fixture.fork.as_ref().unwrap();
        assert_eq!(
            git_output(fork, &["rev-parse", "refs/heads/fix/transient-gh"]),
            prepared_head
        );

        let retry = fixture
            .maintenance(fake_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Pr {
                branch: "fix/transient-gh".to_owned(),
                title: "Retry transient gh failure".to_owned(),
                body: "The prepared and pushed HEAD must be reusable.".to_owned(),
                commit_message: "fix: retry transient gh".to_owned(),
                paths: vec!["retry.txt".to_owned()],
                base: None,
            })
            .await;

        assert!(retry.ok, "{}\n{}", retry.summary, retry.output);
        assert_eq!(
            git_output(&fixture.checkout, &["rev-parse", "HEAD"]),
            prepared_head
        );
        assert_eq!(
            git_output(&fixture.checkout, &["rev-list", "--count", "HEAD"]),
            prepared_commit_count
        );
    }

    #[tokio::test]
    async fn mocked_authenticated_gh_requires_and_records_a_verified_pr_url() {
        let fixture = GitFixture::fork();
        fs::write(fixture.checkout.join("fix.txt"), "scoped fix\n").unwrap();
        let maintenance = fixture.maintenance(fake_gh(fixture._root.path()));
        let receipt = maintenance
            .execute(RepositoryMaintenanceRequest::Pr {
                branch: "fix/scoped".to_owned(),
                title: "Fix scoped behavior".to_owned(),
                body: "This is a bounded test PR body.".to_owned(),
                commit_message: "fix: scoped behavior".to_owned(),
                paths: vec!["fix.txt".to_owned()],
                base: None,
            })
            .await;
        assert!(receipt.ok, "{}\n{}", receipt.summary, receipt.output);
        assert!(
            receipt
                .output
                .contains("https://github.com/canonical/cthuwu/pull/42")
        );
        let fork = fixture.fork.as_ref().unwrap();
        assert_eq!(
            git_output(fork, &["rev-parse", "refs/heads/fix/scoped"]),
            git_output(&fixture.checkout, &["rev-parse", "HEAD"])
        );
    }

    #[tokio::test]
    async fn suspicious_effective_git_configuration_fails_closed() {
        let fixture = GitFixture::canonical();
        git(
            &fixture.checkout,
            &["config", "core.sshCommand", "sh -c 'echo secret'"],
        );
        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Status)
            .await;
        assert!(!receipt.ok);
        assert!(receipt.summary.contains("core.sshcommand"));
        assert!(!receipt.summary.contains("echo secret"));
    }

    #[tokio::test]
    async fn external_git_config_include_is_rejected_without_receipting_its_path() {
        let fixture = GitFixture::canonical();
        let included = fixture._root.path().join("outside-secret-config");
        fs::write(&included, "[core]\n\thooksPath = /tmp/never-run\n").unwrap();
        git(
            &fixture.checkout,
            &["config", "include.path", included.to_str().unwrap()],
        );

        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Status)
            .await;

        assert!(!receipt.ok);
        assert!(receipt.summary.contains("include.path"));
        assert!(!receipt.summary.contains("outside-secret-config"));
    }

    #[tokio::test]
    async fn dangerous_merge_options_and_remote_refspecs_fail_closed() {
        let fixture = GitFixture::canonical();
        git(
            &fixture.checkout,
            &["config", "branch.main.mergeOptions", "-s ours"],
        );
        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Status)
            .await;
        assert!(!receipt.ok);
        assert!(receipt.summary.contains("branch.main.mergeoptions"));
        assert!(!receipt.summary.contains("-s ours"));

        git(
            &fixture.checkout,
            &["config", "--unset-all", "branch.main.mergeOptions"],
        );
        let old_remote_head = git_output(
            &fixture.checkout,
            &["rev-parse", "refs/remotes/origin/main"],
        );
        fixture.upstream_commit("not-fetched.txt", "upstream\n", "not fetched");
        git(
            &fixture.checkout,
            &[
                "config",
                "--replace-all",
                "remote.origin.fetch",
                "+refs/heads/*:refs/heads/overwrite-me/*",
            ],
        );
        let receipt = fixture
            .maintenance(missing_gh(fixture._root.path()))
            .execute(RepositoryMaintenanceRequest::Fetch)
            .await;
        assert!(!receipt.ok);
        assert!(
            receipt.summary.contains("unsupported"),
            "{}",
            receipt.summary
        );
        assert_eq!(
            git_output(
                &fixture.checkout,
                &["rev-parse", "refs/remotes/origin/main"]
            ),
            old_remote_head
        );
    }

    #[tokio::test]
    async fn submodule_policy_rejects_traversal_before_any_submodule_command() {
        let fixture = GitFixture::canonical();
        fs::write(
            fixture.checkout.join(".gitmodules"),
            "[submodule \"escape\"]\n\tpath = ../escape\n\turl = https://github.com/example/escape.git\n",
        )
        .unwrap();
        let maintenance = fixture.maintenance(missing_gh(fixture._root.path()));

        let error = maintenance
            .validate_submodules(&fixture.checkout, Instant::now() + Duration::from_secs(10))
            .await
            .unwrap_err();

        assert!(format!("{error:#}").contains("traversal"));
        assert!(!fixture._root.path().join("escape").exists());
    }

    #[test]
    fn embedded_validation_manifest_covers_repository_and_ci_instructions() {
        let policy = RepositoryPolicy::load().unwrap();
        for id in [
            ValidationId::LauncherSmoke,
            ValidationId::LauncherTest,
            ValidationId::InstallTest,
            ValidationId::ForgeFmt,
            ValidationId::ForgeLint,
            ValidationId::ForgeBuild,
            ValidationId::ForgeTest,
        ] {
            assert!(policy.update_validation.contains(&id), "missing {id:?}");
            assert!(policy.pr_validation.contains(&id), "missing {id:?}");
        }
        assert_eq!(
            validation_command(ValidationId::LauncherTest).program,
            "bash"
        );
        assert_eq!(validation_command(ValidationId::ForgeTest).program, "forge");
    }

    #[test]
    fn command_and_remote_receipts_redact_credentials_and_known_secret_shapes() {
        let sanitized = sanitize_text(
            "https://alice:hunter2@github.com/pierce403/cthuwu.git ghp_abcdefghijkl TOKEN=first token=second PASSWORD=third",
            1024,
        );
        assert!(!sanitized.contains("alice:hunter2"));
        assert!(!sanitized.contains("ghp_abcdefghijkl"));
        assert!(!sanitized.contains("first"));
        assert!(!sanitized.contains("second"));
        assert!(!sanitized.contains("third"));
        assert!(sanitized.contains("[redacted]"));
        assert_eq!(
            sanitize_remote_url("https://token@github.com/pierce403/cthuwu.git"),
            "[redacted-or-unsupported-remote-url]"
        );
    }

    #[test]
    fn canonical_lookalike_repository_suffix_is_not_collapsed_or_adopted() {
        let (identity, safe) =
            parse_github_repository("https://github.com/pierce403/cthuwu.git.git");
        let identity = identity.expect("lookalike remains a syntactically valid distinct repo");

        assert!(safe);
        assert_eq!(identity.owner, "pierce403");
        assert_eq!(identity.repository, "cthuwu.git");
        let fixture = GitFixture::canonical();
        let maintenance = fixture.maintenance(missing_gh(fixture._root.path()));
        assert!(!maintenance.is_canonical_identity(Some(&identity)));
    }
}
