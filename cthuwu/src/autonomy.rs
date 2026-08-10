use crate::evolution::{LifecycleIntent, LifecycleReceipt};
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Seek},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    time::timeout,
};

const MAX_EXECUTOR_RECEIPT_BYTES: u64 = 64 * 1024;
const DEFAULT_EXECUTOR_TIMEOUT: Duration = Duration::from_secs(120);
#[cfg(unix)]
const TRUSTED_EXECUTOR_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Shell-free boundary for process provisioning, absorption transfers, and Base transactions.
///
/// The configured executable receives one canonical JSON `LifecycleIntent` on stdin and must emit
/// one JSON `LifecycleReceipt` on stdout. It must obtain signing through a separately isolated key
/// service; uwubot never forwards a private key in an environment, intent, or receipt.
#[derive(Clone, Debug)]
pub struct LifecycleExecutor {
    executable: PathBuf,
    executable_sha256: [u8; 32],
    timeout: Duration,
}

impl LifecycleExecutor {
    pub fn new(executable: impl Into<PathBuf>) -> Result<Self> {
        let executable = executable.into();
        let (executable, executable_sha256) = inspect_executor(&executable)?;
        Ok(Self {
            executable,
            executable_sha256,
            timeout: DEFAULT_EXECUTOR_TIMEOUT,
        })
    }

    /// Refuses an executor that an authenticated operator can replace through workspace tools.
    pub fn ensure_outside_operator_root(&self, operator_root: &Path) -> Result<()> {
        let operator_root = fs::canonicalize(operator_root).with_context(|| {
            format!(
                "resolving operator workspace root {}",
                operator_root.display()
            )
        })?;
        ensure!(
            !self.executable.starts_with(&operator_root),
            "lifecycle executor must be outside UWUBOT_OPERATOR_ROOT"
        );
        Ok(())
    }

    #[cfg(test)]
    fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[cfg(test)]
    async fn execute(&self, intent: &LifecycleIntent) -> Result<LifecycleReceipt> {
        self.execute_with_rpc(intent, None).await
    }

    /// Executes with the exact already-validated public RPC endpoint selected by Rust. All token,
    /// wallet, amount, and action identity fields remain authoritative in the signed durable intent.
    pub async fn execute_with_rpc(
        &self,
        intent: &LifecycleIntent,
        rpc_endpoint: Option<&str>,
    ) -> Result<LifecycleReceipt> {
        let encoded = serde_json::to_vec(intent).context("encoding lifecycle intent")?;
        // Open, hash, and then execute the same file description. This closes the path-swap window
        // between verification and handing the process its dedicated economics environment.
        let executable = open_verified_executor(&self.executable, self.executable_sha256)?;
        #[cfg(target_os = "linux")]
        let launch_path = {
            use std::os::fd::AsRawFd;
            let descriptor = executable.as_raw_fd();
            // Script interpreters must be able to reopen /proc/self/fd/N after exec. The child gets
            // this one executable descriptor; the parent closes it immediately after spawn.
            let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
            ensure!(
                flags >= 0,
                "reading lifecycle executor descriptor flags failed"
            );
            ensure!(
                unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } >= 0,
                "pinning lifecycle executor descriptor failed"
            );
            PathBuf::from(format!("/proc/self/fd/{descriptor}"))
        };
        #[cfg(not(target_os = "linux"))]
        let launch_path = self.executable.clone();
        let mut command = Command::new(&launch_path);
        command
            .arg("execute")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // An executor must return sanitized diagnostics in its receipt. Its stderr is not
            // inherited because transaction libraries may include credential-bearing RPC URLs.
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.current_dir("/");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }
        copy_executor_environment(&mut command, rpc_endpoint);
        let mut child = command.spawn().with_context(|| {
            format!("starting lifecycle executor {}", self.executable.display())
        })?;
        drop(executable);
        #[cfg(unix)]
        let _process_group = ProcessGroupGuard::new(child.id());
        let mut stdin = child
            .stdin
            .take()
            .context("lifecycle executor stdin missing")?;
        let stdout = child
            .stdout
            .take()
            .context("lifecycle executor stdout missing")?;

        let operation = async {
            stdin.write_all(&encoded).await?;
            stdin.shutdown().await?;
            drop(stdin);

            let mut receipt_bytes = Vec::new();
            stdout
                .take(MAX_EXECUTOR_RECEIPT_BYTES + 1)
                .read_to_end(&mut receipt_bytes)
                .await?;
            let status = child.wait().await?;
            ensure!(
                receipt_bytes.len() as u64 <= MAX_EXECUTOR_RECEIPT_BYTES,
                "lifecycle executor receipt exceeds {MAX_EXECUTOR_RECEIPT_BYTES} bytes"
            );
            ensure!(status.success(), "lifecycle executor returned {status}");
            let receipt: LifecycleReceipt = serde_json::from_slice(&receipt_bytes)
                .context("lifecycle executor returned invalid receipt JSON")?;
            ensure!(
                receipt.action_id == intent.action_id,
                "lifecycle executor receipt action ID does not match its intent"
            );
            Ok::<_, anyhow::Error>(receipt)
        };

        let receipt = timeout(self.timeout, operation)
            .await
            .context("lifecycle executor timed out")??;
        Ok(receipt)
    }
}

#[cfg(unix)]
struct ProcessGroupGuard {
    process_id: Option<i32>,
}

#[cfg(unix)]
impl ProcessGroupGuard {
    fn new(process_id: Option<u32>) -> Self {
        Self {
            process_id: process_id.and_then(|value| i32::try_from(value).ok()),
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if let Some(process_id) = self.process_id {
            // The executor is its process-group leader. A negative PID kills the whole group,
            // including signer/provisioner descendants that could otherwise outlive a timeout and
            // race the supervisor's idempotent retry.
            unsafe {
                libc::kill(-process_id, libc::SIGKILL);
            }
        }
    }
}

fn copy_executor_environment(command: &mut Command, rpc_endpoint: Option<&str>) {
    // The executor may need one dedicated Base signer, but it never receives XMTP identity,
    // database, model, search, operator-workspace credentials, caller-controlled loader paths, or
    // a raw signing key. The executor must use a separately isolated signer/key service.
    #[cfg(unix)]
    command.env("PATH", TRUSTED_EXECUTOR_PATH);
    for name in [
        "SYSTEMROOT",
        "WINDIR",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ] {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
    if let Some(rpc_endpoint) = rpc_endpoint {
        command.env("CTHUWU_RPC_ENDPOINT", rpc_endpoint);
    }
}

fn inspect_executor(path: &Path) -> Result<(PathBuf, [u8; 32])> {
    if !path.is_absolute() {
        bail!("CTHUWU_LIFECYCLE_EXECUTOR must be an absolute executable path");
    }
    let path_metadata = fs::symlink_metadata(path)
        .with_context(|| format!("reading lifecycle executor {}", path.display()))?;
    ensure!(
        !path_metadata.file_type().is_symlink(),
        "lifecycle executor must not be a symlink"
    );
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("resolving lifecycle executor {}", path.display()))?;
    let mut file = open_executor_no_follow(&canonical)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "lifecycle executor must be a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o111 != 0,
            "lifecycle executor is not executable"
        );
        ensure!(
            metadata.permissions().mode() & 0o022 == 0,
            "lifecycle executor must not be group- or world-writable"
        );
    }
    let digest = hash_executor(&mut file)?;
    Ok((canonical, digest))
}

fn open_verified_executor(path: &Path, expected_sha256: [u8; 32]) -> Result<File> {
    let mut file = open_executor_no_follow(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "lifecycle executor must remain a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o111 != 0
                && metadata.permissions().mode() & 0o022 == 0,
            "lifecycle executor permissions changed after startup"
        );
    }
    ensure!(
        hash_executor(&mut file)? == expected_sha256,
        "lifecycle executor content changed after startup"
    );
    Ok(file)
}

fn hash_executor(file: &mut File) -> Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    file.rewind()?;
    Ok(hasher.finalize().into())
}

fn open_executor_no_follow(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options
        .open(path)
        .with_context(|| format!("opening lifecycle executor {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolution::{LifecycleAction, LifecycleReceiptStatus};

    #[test]
    fn executor_requires_an_absolute_regular_executable() {
        assert!(LifecycleExecutor::new("relative-executor").is_err());
        let directory = tempfile::tempdir().unwrap();
        assert!(LifecycleExecutor::new(directory.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn executor_rejects_symlinks_writable_files_and_operator_workspace_paths() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let directory = tempfile::tempdir().unwrap();
        let executor_path = directory.path().join("executor.sh");
        fs::write(&executor_path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&executor_path, fs::Permissions::from_mode(0o722)).unwrap();
        assert!(LifecycleExecutor::new(&executor_path).is_err());

        fs::set_permissions(&executor_path, fs::Permissions::from_mode(0o700)).unwrap();
        let link = directory.path().join("executor-link");
        symlink(&executor_path, &link).unwrap();
        assert!(LifecycleExecutor::new(&link).is_err());

        let executor = LifecycleExecutor::new(&executor_path).unwrap();
        assert!(
            executor
                .ensure_outside_operator_root(directory.path())
                .is_err()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn executor_content_is_pinned_before_economic_environment_is_forwarded() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let executor_path = directory.path().join("executor.sh");
        let side_effect = directory.path().join("ran");
        fs::write(&executor_path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&executor_path, fs::Permissions::from_mode(0o700)).unwrap();
        let executor = LifecycleExecutor::new(&executor_path).unwrap();
        fs::write(
            &executor_path,
            format!("#!/bin/sh\ntouch '{}'\n", side_effect.display()),
        )
        .unwrap();
        let intent = LifecycleIntent {
            action_id: "a".repeat(64),
            created_at_ms: 1,
            action: LifecycleAction::Shutdown {
                tentacle_id: "tentacle-test".to_owned(),
                judgment_id: "b".repeat(64),
                after_action_id: None,
            },
        };

        assert!(executor.execute(&intent).await.is_err());
        assert!(!side_effect.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn executor_ignores_caller_loader_path_and_operator_working_directory() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let poisoned_path = directory.path().join("poisoned-bin");
        fs::create_dir(&poisoned_path).unwrap();
        let poisoned_helper = poisoned_path.join("helper");
        let poisoned_effect = directory.path().join("poisoned-effect");
        fs::write(
            &poisoned_helper,
            format!("#!/bin/sh\ntouch '{}'\n", poisoned_effect.display()),
        )
        .unwrap();
        fs::set_permissions(&poisoned_helper, fs::Permissions::from_mode(0o700)).unwrap();

        let executor_path = directory.path().join("executor.sh");
        let environment_report = directory.path().join("environment");
        fs::write(
            &executor_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n%s\\n%s' \"$PATH\" \"$LD_LIBRARY_PATH\" \"$PWD\" > '{}'\nhelper >/dev/null 2>&1 || true\n/bin/cat >/dev/null\nprintf '{{}}'\n",
                environment_report.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&executor_path, fs::Permissions::from_mode(0o700)).unwrap();
        let executor = LifecycleExecutor::new(&executor_path).unwrap();
        let intent = LifecycleIntent {
            action_id: "a".repeat(64),
            created_at_ms: 1,
            action: LifecycleAction::Shutdown {
                tentacle_id: "tentacle-test".to_owned(),
                judgment_id: "b".repeat(64),
                after_action_id: None,
            },
        };
        assert!(executor.execute(&intent).await.is_err());

        let report = fs::read_to_string(environment_report).unwrap();
        let mut lines = report.lines();
        assert_eq!(lines.next(), Some(TRUSTED_EXECUTOR_PATH));
        assert_eq!(lines.next(), Some(""));
        assert_eq!(lines.next(), Some("/"));
        assert!(!poisoned_effect.exists());
    }

    #[test]
    fn test_timeout_override_is_bounded_to_the_instance() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("executor");
        fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let executor = LifecycleExecutor::new(executable)
            .unwrap()
            .with_timeout(Duration::from_millis(1));
        assert_eq!(executor.timeout, Duration::from_millis(1));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_executor_descendants_before_retry() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let executor_path = directory.path().join("executor.sh");
        let late_effect = directory.path().join("late-effect");
        std::fs::write(
            &executor_path,
            format!(
                "#!/bin/sh\ncat >/dev/null\n(sleep 0.2; printf escaped > '{}') &\nsleep 5\n",
                late_effect.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&executor_path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let executor = LifecycleExecutor::new(&executor_path)
            .unwrap()
            .with_timeout(Duration::from_millis(50));
        let intent = LifecycleIntent {
            action_id: "a".repeat(64),
            created_at_ms: 1,
            action: LifecycleAction::Shutdown {
                tentacle_id: "tentacle-test".to_owned(),
                judgment_id: "b".repeat(64),
                after_action_id: None,
            },
        };

        assert!(executor.execute(&intent).await.is_err());
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert!(!late_effect.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn successful_receipt_cannot_leave_executor_descendants_running() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let executor_path = directory.path().join("executor.sh");
        let late_effect = directory.path().join("late-success-effect");
        let action_id = "a".repeat(64);
        let receipt = format!(
            r#"{{"actionId":"{action_id}","completedAtMs":2,"status":"succeeded","externalReference":"complete","detail":null}}"#
        );
        fs::write(
            &executor_path,
            format!(
                "#!/bin/sh\n/bin/cat >/dev/null\n(exec >/dev/null 2>&1; sleep 0.2; touch '{}') &\nprintf '%s' '{}'\n",
                late_effect.display(),
                receipt
            ),
        )
        .unwrap();
        fs::set_permissions(&executor_path, fs::Permissions::from_mode(0o700)).unwrap();
        let executor = LifecycleExecutor::new(&executor_path).unwrap();
        let intent = LifecycleIntent {
            action_id,
            created_at_ms: 1,
            action: LifecycleAction::Shutdown {
                tentacle_id: "tentacle-test".to_owned(),
                judgment_id: "b".repeat(64),
                after_action_id: None,
            },
        };

        assert_eq!(
            executor.execute(&intent).await.unwrap().status,
            LifecycleReceiptStatus::Succeeded
        );
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert!(!late_effect.exists());
    }
}
