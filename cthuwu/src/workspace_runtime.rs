//! Workspace-local process defaults and a private Git journal. This is not an OS sandbox.
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;

const LOG: &str = "WORKSPACE_LOG.md";
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_FILES: usize = 4096;
const IGNORED: &[&str] = &[
    ".git",
    "code",
    "tmp",
    "tools",
    "releases",
    ".knowledge-index",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    ".home",
    "state",
    "private",
    "secrets",
    "credentials",
    "contacts",
    "sessions",
    "coaching",
];
type Snapshot = BTreeMap<String, String>;

/// Create only owned directories; symlinks may never redirect caches or temporary writes.
fn owned_directory(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        path.push(part);
        if !path.exists() {
            match fs::create_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "workspace runtime directories must be real directories"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(path)
}

pub fn environment_for(root: &Path) -> Result<BTreeMap<String, String>> {
    let root = fs::canonicalize(root)?;
    ensure!(root.is_dir(), "workspace root must be a directory");
    let mut env = BTreeMap::new();
    for (name, relative) in [
        ("HOME", "tools/home"),
        ("TMPDIR", "tmp"),
        ("TMP", "tmp"),
        ("TEMP", "tmp"),
        ("XDG_CONFIG_HOME", "tools/xdg/config"),
        ("XDG_CACHE_HOME", "tools/xdg/cache"),
        ("XDG_DATA_HOME", "tools/xdg/data"),
        ("XDG_STATE_HOME", "tools/xdg/state"),
        ("XDG_RUNTIME_DIR", "tools/xdg/runtime"),
        ("VIRTUAL_ENV", "tools/venv"),
        ("PIP_CACHE_DIR", "tools/pip"),
        ("PIP_PREFIX", "tools/venv"),
        ("npm_config_prefix", "tools/npm"),
        ("NPM_CONFIG_PREFIX", "tools/npm"),
        ("npm_config_cache", "tools/npm-cache"),
        ("NPM_CONFIG_CACHE", "tools/npm-cache"),
        ("PNPM_HOME", "tools/pnpm"),
        ("PNPM_STORE_DIR", "tools/pnpm-store"),
        ("npm_config_store_dir", "tools/pnpm-store"),
        ("CARGO_HOME", "tools/cargo"),
        ("RUSTUP_HOME", "tools/rustup"),
        ("HOMEBREW_PREFIX", "tools/brew"),
        ("HOMEBREW_REPOSITORY", "tools/brew"),
        ("HOMEBREW_CACHE", "tools/brew-cache"),
        ("HOMEBREW_LOGS", "tools/brew-logs"),
        ("GOPATH", "tools/go"),
        ("GOBIN", "tools/go/bin"),
        ("GOCACHE", "tools/go-cache"),
        ("GOMODCACHE", "tools/go-mod"),
        ("GEM_HOME", "tools/gems"),
        ("BUNDLE_PATH", "tools/bundle"),
        ("UV_CACHE_DIR", "tools/uv-cache"),
        ("UV_TOOL_DIR", "tools/uv-tools"),
        ("UV_PYTHON_INSTALL_DIR", "tools/uv-python"),
        ("UV_TOOL_BIN_DIR", "tools/bin"),
        ("PYTHONUSERBASE", "tools/python"),
        ("PYTHONPYCACHEPREFIX", "tools/python-cache"),
        ("OLLAMA_MODELS", "tools/ollama"),
        ("HF_HOME", "tools/huggingface"),
    ] {
        env.insert(
            name.to_owned(),
            owned_directory(&root, relative)?
                .to_string_lossy()
                .into_owned(),
        );
    }
    // Authentication tools may use this directory, but the variable itself is capability-scoped.
    owned_directory(&root, "tools/xdg/config/gh")?;
    let mut paths = Vec::new();
    for relative in [
        "tools/bin",
        "tools/venv/bin",
        "tools/pnpm",
        "tools/cargo/bin",
        "tools/npm/bin",
        "tools/brew/bin",
        "tools/brew/sbin",
        "tools/go/bin",
        "tools/gems/bin",
        "tools/python/bin",
    ] {
        paths.push(owned_directory(&root, relative)?);
    }
    if let Some(value) = std::env::var_os("UWUBOT_READONLY_TOOL_PATH") {
        let readonly = std::env::split_paths(&value)
            .filter(|path| path.is_absolute() && path.is_dir())
            .collect::<Vec<_>>();
        for path in &readonly {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        env.insert(
            "UWUBOT_READONLY_TOOL_PATH".into(),
            std::env::join_paths(readonly)?
                .to_string_lossy()
                .into_owned(),
        );
    }
    paths.extend([
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ]);
    env.insert(
        "PATH".into(),
        std::env::join_paths(paths)?.to_string_lossy().into_owned(),
    );
    env.insert("PIP_REQUIRE_VIRTUALENV".into(), "true".into());
    env.insert("PIP_DISABLE_PIP_VERSION_CHECK".into(), "1".into());
    env.insert("HOMEBREW_NO_AUTO_UPDATE".into(), "1".into());
    env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    env.insert("GIT_CONFIG_GLOBAL".into(), "/dev/null".into());
    env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    for name in ["LANG", "LC_ALL", "TERM"] {
        if let Ok(value) = std::env::var(name) {
            env.insert(name.into(), value);
        }
    }
    if let Ok(value) = std::env::var("UWUBOT_RUNNING_SOURCE")
        && value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        env.insert("UWUBOT_RUNNING_SOURCE".into(), value);
    }
    Ok(env)
}

pub struct WorkspaceRuntime {
    root: PathBuf,
    environment: BTreeMap<String, String>,
    previous: Mutex<Snapshot>,
    preexisting: Mutex<BTreeSet<String>>,
}

impl WorkspaceRuntime {
    pub fn new(root: &Path) -> Result<Self> {
        let root = fs::canonicalize(root)?;
        let environment = environment_for(&root)?;
        let new_repository = !root.join(".git").exists();
        let instance = Self {
            root,
            environment,
            previous: Mutex::new(BTreeMap::new()),
            preexisting: Mutex::new(BTreeSet::new()),
        };
        if new_repository {
            instance.git(
                &["init", "--quiet", "--initial-branch=workspace"],
                None,
                None,
            )?;
        }
        instance.validate_repository()?;
        // Existing dirty and staged work belongs to its author, not this agent's first checkpoint.
        if !new_repository {
            *instance
                .previous
                .lock()
                .map_err(|_| anyhow::anyhow!("workspace journal lock poisoned"))? =
                instance.snapshot()?;
        }
        if !new_repository {
            *instance
                .preexisting
                .lock()
                .map_err(|_| anyhow::anyhow!("workspace journal lock poisoned"))? =
                instance.dirty_paths()?;
        }
        instance.checkpoint("initialize workspace journal")?;
        Ok(instance)
    }

    fn validate_repository(&self) -> Result<()> {
        let git = self.root.join(".git");
        let metadata = fs::symlink_metadata(&git)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "workspace journal needs an owned .git directory, not a linked worktree or symlink"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&git, fs::Permissions::from_mode(0o700))?;
        }
        for name in ["config", "HEAD", "index", "objects", "refs", "info"] {
            let path = git.join(name);
            if let Ok(metadata) = fs::symlink_metadata(path) {
                ensure!(
                    !metadata.file_type().is_symlink(),
                    "workspace Git metadata must not contain symlinks"
                );
            }
        }
        ensure!(
            !git.join("objects/info/alternates").exists(),
            "workspace journal must own its object store"
        );
        reject_git_symlinks(&git.join("objects"))?;
        reject_git_symlinks(&git.join("refs"))?;
        let top = self.git(&["rev-parse", "--show-toplevel"], None, None)?;
        ensure!(
            Path::new(top.trim()) == self.root,
            "workspace journal refuses an enclosing repository"
        );
        let excludes = git.join("info/exclude");
        if let Ok(meta) = fs::symlink_metadata(&excludes) {
            ensure!(
                !meta.file_type().is_symlink(),
                "workspace Git excludes must not be a symlink"
            );
        }
        let mut contents = fs::read_to_string(&excludes).unwrap_or_default();
        if !contents.contains("# Tentacle workspace runtime") {
            contents.push_str("\n# Tentacle workspace runtime\n/code/\n/tmp/\n/tools/\n/releases/\n/.knowledge-index/\n.env\n.env.*\n*.pem\n*.key\n*.sqlite*\n*.db\nnode_modules/\ntarget/\n__pycache__/\n");
            fs::write(excludes, contents)?;
        }
        Ok(())
    }

    /// Record changes since the previous completed step, without staging any pre-existing work.
    /// The reason is a fixed runtime description, never an operator prompt or a shell command.
    pub fn checkpoint(&self, reason: &str) -> Result<Option<String>> {
        ensure!(
            !reason.is_empty() && reason.len() <= 160 && !reason.contains(['\n', '\r']),
            "workspace journal reason must be a short runtime description"
        );
        let mut previous = self
            .previous
            .lock()
            .map_err(|_| anyhow::anyhow!("workspace journal lock poisoned"))?;
        self.validate_repository()?;
        let lock_path = self.root.join(".git/index.lock");
        let mut index_lock = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .context("workspace Git index is busy; journal checkpoint postponed")?;
        let mut lock_cleanup = RemoveFile(Some(lock_path.clone()));
        let current = self.snapshot()?;
        let mut preexisting = self
            .preexisting
            .lock()
            .map_err(|_| anyhow::anyhow!("workspace journal lock poisoned"))?;
        let dirty = self.dirty_paths()?;
        preexisting.retain(|path| dirty.contains(path));
        let staged = self
            .git(
                &[
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--cached",
                    "--name-only",
                    "-z",
                ],
                None,
                None,
            )?
            .split('\0')
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let mut changed = current
            .keys()
            .chain(previous.keys())
            .filter(|name| current.get(*name) != previous.get(*name))
            .filter(|name| !staged.contains(*name) && !preexisting.contains(*name))
            .cloned()
            .collect::<BTreeSet<_>>();
        let protected_changed = current.keys().chain(previous.keys()).any(|name| {
            (staged.contains(name) || preexisting.contains(name))
                && current.get(name) != previous.get(name)
        });
        if changed.is_empty() && protected_changed {
            bail!(
                "pre-existing dirty or staged operator files changed; they remain uncommitted for operator review"
            );
        }
        if changed.is_empty() {
            *previous = current;
            return Ok(None);
        }
        ensure!(
            !staged.contains(LOG) && !preexisting.contains(LOG),
            "workspace log has staged operator changes; checkpoint postponed"
        );
        let log_path = self.root.join(LOG);
        if let Ok(meta) = fs::symlink_metadata(&log_path) {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "workspace log must be a regular file"
            );
        }
        let mut log = fs::read_to_string(&log_path).unwrap_or_else(|_| "# Workspace change log\n\nPrivate local checkpoints; source history lives in code/.\n".into());
        ensure!(
            log.len() < MAX_FILE_BYTES as usize,
            "workspace change log needs rotation before checkpointing"
        );
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        log.push_str(&format!(
            "\n## {timestamp}\n\n- Why: {reason}.\n- Workspace files changed: {}.\n",
            changed.len()
        ));
        let mut log_file = NamedTempFile::new_in(self.root.join("tmp"))?;
        log_file.write_all(log.as_bytes())?;
        log_file.as_file().sync_all()?;
        log_file.persist(&log_path).map_err(|error| error.error)?;
        changed.insert(LOG.into());
        let index_path = self.root.join(".git/index");
        let commit_index = NamedTempFile::new_in(self.root.join("tmp"))?.into_temp_path();
        fs::remove_file(&commit_index)?;
        let next_index = NamedTempFile::new_in(self.root.join("tmp"))?.into_temp_path();
        let head = self
            .git(&["rev-parse", "--verify", "HEAD"], None, None)
            .ok()
            .map(|value| value.trim().to_owned());
        if let Some(head) = &head {
            self.git(&["read-tree", head], Some(&commit_index), None)?;
        } else {
            self.git(&["read-tree", "--empty"], Some(&commit_index), None)?;
        }
        if index_path.exists() {
            fs::copy(&index_path, &next_index)?;
        } else {
            fs::remove_file(&next_index)?;
            self.git(&["read-tree", "--empty"], Some(&next_index), None)?;
        }
        for name in &changed {
            let path = self.root.join(name);
            if path.exists() {
                let metadata = fs::symlink_metadata(&path)?;
                ensure!(
                    metadata.is_file() && !metadata.file_type().is_symlink(),
                    "workspace file changed type while checkpointing"
                );
                let content = fs::read(&path)?;
                ensure!(
                    content.len() <= MAX_FILE_BYTES as usize,
                    "workspace file grew beyond checkpoint limit"
                );
                if name != LOG {
                    ensure!(
                        current.get(name) == Some(&digest(&content)),
                        "workspace file changed while checkpointing; retry next step"
                    );
                }
                let blob = self.git(&["hash-object", "-w", "--stdin"], None, Some(&content))?;
                #[cfg(unix)]
                let mode = {
                    use std::os::unix::fs::PermissionsExt;
                    if metadata.permissions().mode() & 0o111 != 0 {
                        "100755"
                    } else {
                        "100644"
                    }
                };
                #[cfg(not(unix))]
                let mode = "100644";
                for index in [&commit_index, &next_index] {
                    self.git(
                        &[
                            "update-index",
                            "--add",
                            "--cacheinfo",
                            mode,
                            blob.trim(),
                            name,
                        ],
                        Some(index),
                        None,
                    )?;
                }
            } else {
                for index in [&commit_index, &next_index] {
                    self.git(
                        &["update-index", "--force-remove", "--", name],
                        Some(index),
                        None,
                    )?;
                }
            }
        }
        let tree = self.git(&["write-tree"], Some(&commit_index), None)?;
        let message = format!("workspace: {reason}");
        let mut args = vec!["commit-tree", tree.trim()];
        if let Some(head) = &head {
            args.extend(["-p", head]);
        }
        args.extend(["-m", &message]);
        let commit = self.git(&args, None, None)?;
        let old = head
            .as_deref()
            .unwrap_or("0000000000000000000000000000000000000000");
        index_lock.write_all(&fs::read(&next_index)?)?;
        index_lock.sync_all()?;
        self.git(&["update-ref", "HEAD", commit.trim(), old], None, None)?;
        fs::rename(&lock_path, &index_path)?;
        lock_cleanup.0 = None;
        *previous = self.snapshot()?;
        if protected_changed {
            bail!(
                "eligible workspace changes journaled; changed pre-existing dirty or staged operator files remain uncommitted for review"
            );
        }
        Ok(Some(commit.trim().to_owned()))
    }

    fn dirty_paths(&self) -> Result<BTreeSet<String>> {
        let tracked = self
            .git(
                &[
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "HEAD",
                    "--name-only",
                    "-z",
                ],
                None,
                None,
            )
            .unwrap_or_default();
        let untracked = self.git(
            &["ls-files", "--others", "--exclude-standard", "-z"],
            None,
            None,
        )?;
        Ok(tracked
            .split('\0')
            .chain(untracked.split('\0'))
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect())
    }

    fn snapshot(&self) -> Result<Snapshot> {
        let mut snapshot = BTreeMap::new();
        collect(&self.root, &self.root, &mut snapshot)?;
        Ok(snapshot)
    }

    fn git(&self, args: &[&str], index: Option<&Path>, input: Option<&[u8]>) -> Result<String> {
        // Fixed system Git prevents a learned workspace executable from intercepting the journal.
        let program = ["/usr/bin/git", "/usr/local/bin/git"]
            .into_iter()
            .find(|path| Path::new(path).is_file())
            .context("Git is required for the workspace journal")?;
        let output = NamedTempFile::new_in(self.root.join("tmp"))?;
        let mut command = Command::new(program);
        command
            .current_dir(&self.root)
            .env_clear()
            .envs(&self.environment)
            .env("GIT_AUTHOR_NAME", "Tentacle")
            .env("GIT_AUTHOR_EMAIL", "tentacle@workspace.invalid")
            .env("GIT_COMMITTER_NAME", "Tentacle")
            .env("GIT_COMMITTER_EMAIL", "tentacle@workspace.invalid")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "gc.auto=0",
                "-c",
                "maintenance.auto=false",
            ])
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::from(output.reopen()?))
            .stderr(Stdio::null());
        if let Some(index) = index {
            command.env("GIT_INDEX_FILE", index);
        }
        let mut child = command.spawn().context("starting workspace journal Git")?;
        if let Some(input) = input {
            child
                .stdin
                .take()
                .context("Git input unavailable")?
                .write_all(input)?;
        }
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                ensure!(status.success(), "workspace journal Git operation failed");
                ensure!(
                    output.as_file().metadata()?.len() <= 2 * 1024 * 1024,
                    "workspace journal Git output exceeded its limit"
                );
                return Ok(fs::read_to_string(output.path())?);
            }
            if start.elapsed() >= Duration::from_secs(15) {
                let _ = child.kill();
                let _ = child.wait();
                bail!("workspace journal Git operation timed out");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

struct RemoveFile(Option<PathBuf>);
impl Drop for RemoveFile {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}
fn digest(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

fn reject_git_symlinks(directory: &Path) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "workspace Git storage must not contain symlinks"
        );
        if metadata.is_dir() {
            reject_git_symlinks(&entry.path())?;
        }
    }
    Ok(())
}

fn collect(root: &Path, directory: &Path, output: &mut Snapshot) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_ascii_lowercase();
        if IGNORED.contains(&lower.as_str())
            || lower.starts_with(".env")
            || [".pem", ".key", ".sqlite", ".sqlite3", ".db", ".pyc"]
                .iter()
                .any(|suffix| lower.ends_with(suffix))
            || lower.contains("credential")
            || lower.contains("secret")
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect(root, &path, output)?;
        } else if metadata.is_file() && metadata.len() <= MAX_FILE_BYTES {
            ensure!(
                output.len() < MAX_FILES,
                "workspace journal exceeds its bounded file inventory"
            );
            let content = fs::read(&path)?;
            if let Ok(text) = std::str::from_utf8(&content) {
                if text.contains('\0')
                    || text.contains("-----BEGIN PRIVATE KEY-----")
                    || text.contains("-----BEGIN RSA PRIVATE KEY-----")
                    || text
                        .lines()
                        .any(|line| line.trim().eq_ignore_ascii_case("private: true"))
                {
                    continue;
                }
                output.insert(
                    path.strip_prefix(root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                    digest(&content),
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("/usr/bin/git")
            .current_dir(root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    #[test]
    fn environment_routes_temporary_files_and_installs_inside_workspace() {
        let root = tempfile::tempdir().unwrap();
        let env = environment_for(root.path()).unwrap();
        for name in [
            "HOME",
            "TMPDIR",
            "TMP",
            "TEMP",
            "XDG_CONFIG_HOME",
            "XDG_RUNTIME_DIR",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "npm_config_prefix",
            "npm_config_cache",
            "npm_config_store_dir",
            "PNPM_HOME",
            "PIP_CACHE_DIR",
            "HOMEBREW_PREFIX",
            "UV_TOOL_DIR",
            "OLLAMA_MODELS",
        ] {
            assert!(Path::new(&env[name]).starts_with(root.path()), "{name}");
            assert!(Path::new(&env[name]).is_dir(), "{name}");
        }
        assert_eq!(env["PIP_REQUIRE_VIRTUALENV"], "true");
        assert!(
            env["PATH"].starts_with(&root.path().join("tools/bin").to_string_lossy().into_owned())
        );
        assert!(!env.contains_key("VENICE_API_KEY"));
    }

    #[test]
    fn journal_records_changes_and_deletions_with_reasons_excluding_runtime_state() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("MISSION.md"),
            "Help people choose useful goals.\n",
        )
        .unwrap();
        let runtime = WorkspaceRuntime::new(root.path()).unwrap();
        let initial = git(root.path(), &["rev-parse", "HEAD"]);
        fs::write(
            root.path().join("MISSION.md"),
            "Help people build small habits.\n",
        )
        .unwrap();
        for relative in ["tmp", "tools", "code", "releases", "knowledge"] {
            fs::create_dir_all(root.path().join(relative)).unwrap();
        }
        for relative in [
            "tmp/scratch.md",
            "tools/key.md",
            "code/source.md",
            "releases/build.md",
            ".env",
            "knowledge/private.md",
        ] {
            fs::write(root.path().join(relative), "private: true\nnot shared\n").unwrap();
        }
        fs::write(
            root.path().join("knowledge/habits.md"),
            "Make goals small.\n",
        )
        .unwrap();
        let next = runtime
            .checkpoint("operator tool edit_file")
            .unwrap()
            .unwrap();
        assert_ne!(initial.trim(), next);
        let names = git(root.path(), &["ls-tree", "-r", "--name-only", "HEAD"]);
        assert!(names.contains("MISSION.md"));
        assert!(names.contains("knowledge/habits.md"));
        assert!(!names.contains("private.md"));
        assert!(!names.contains("source.md"));
        assert!(!names.contains(".env"));
        assert!(
            git(root.path(), &["log", "-1", "--format=%B"]).contains("operator tool edit_file")
        );
        assert!(
            fs::read_to_string(root.path().join(LOG))
                .unwrap()
                .contains("Why: operator tool edit_file")
        );
        assert!(git(root.path(), &["diff", "--cached", "--name-only"]).is_empty());
        let mut log = fs::read_to_string(root.path().join(LOG)).unwrap();
        log.push_str("\nOperator added a useful review note.\n");
        fs::write(root.path().join(LOG), log).unwrap();
        assert!(
            runtime
                .checkpoint("operator tool edit_file")
                .unwrap()
                .is_some()
        );
        assert!(
            git(root.path(), &["show", "HEAD:WORKSPACE_LOG.md"])
                .contains("Operator added a useful review note.")
        );
        fs::remove_file(root.path().join("knowledge/habits.md")).unwrap();
        runtime.checkpoint("operator tool delete_file").unwrap();
        assert!(!git(root.path(), &["ls-tree", "-r", "--name-only", "HEAD"]).contains("habits.md"));
    }

    #[test]
    fn checkpoint_preserves_pre_staged_index_and_unrelated_dirty_work() {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init", "--quiet"]);
        for name in ["dirty.md", "staged.md", "owned.md"] {
            fs::write(root.path().join(name), "base\n").unwrap();
        }
        git(root.path(), &["add", "."]);
        git(root.path(), &["commit", "--quiet", "-m", "initial"]);
        fs::write(root.path().join("dirty.md"), "operator unfinished\n").unwrap();
        fs::write(root.path().join("staged.md"), "operator staged\n").unwrap();
        git(root.path(), &["add", "staged.md"]);
        let runtime = WorkspaceRuntime::new(root.path()).unwrap();
        fs::write(root.path().join("owned.md"), "agent change\n").unwrap();
        runtime.checkpoint("operator tool edit_file").unwrap();
        assert_eq!(git(root.path(), &["show", "HEAD:dirty.md"]), "base\n");
        assert_eq!(git(root.path(), &["show", "HEAD:staged.md"]), "base\n");
        assert_eq!(
            git(root.path(), &["show", ":staged.md"]),
            "operator staged\n"
        );
        assert_eq!(
            git(root.path(), &["show", "HEAD:owned.md"]),
            "agent change\n"
        );
        fs::write(
            root.path().join("dirty.md"),
            "operator unfinished plus agent work\n",
        )
        .unwrap();
        assert!(runtime.checkpoint("operator tool edit_file").is_err());
        assert_eq!(git(root.path(), &["show", "HEAD:dirty.md"]), "base\n");
    }

    #[test]
    fn nested_workspace_gets_its_own_git_without_modifying_parent_index() {
        let outer = tempfile::tempdir().unwrap();
        git(outer.path(), &["init", "--quiet"]);
        fs::write(outer.path().join("parent.md"), "owned by operator\n").unwrap();
        git(outer.path(), &["add", "parent.md"]);
        let before = fs::read(outer.path().join(".git/index")).unwrap();
        let workspace = outer.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("MISSION.md"), "Local mission\n").unwrap();
        WorkspaceRuntime::new(&workspace).unwrap();
        assert_eq!(fs::read(outer.path().join(".git/index")).unwrap(), before);
        assert_eq!(
            git(&workspace, &["rev-parse", "--show-toplevel"]).trim(),
            workspace.to_string_lossy()
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_runtime_directories_and_git_storage_are_rejected() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("tmp")).unwrap();
        assert!(environment_for(root.path()).is_err());
        fs::remove_file(root.path().join("tmp")).unwrap();
        let runtime = WorkspaceRuntime::new(root.path()).unwrap();
        symlink(outside.path(), root.path().join(".git/objects/aa")).unwrap();
        assert!(runtime.checkpoint("operator tool exec").is_err());
        assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
    }
}
