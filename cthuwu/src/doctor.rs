//! Fixed diagnostics; no model-generated commands or private content in reports.
use std::{fs, path::Path};

pub fn workspace(root: &Path, repair: bool) -> String {
    let mut lines = vec!["WORKSPACE:".to_owned()];
    let mut runtime_safe = false;
    if repair {
        match crate::workspace_runtime::environment_for(root) {
            Ok(_) => {
                runtime_safe = true;
                lines.push("PASS: workspace-local temporary/cache/tool directories verified or created.".into());
            },
            Err(_) => lines.push("FAIL: unsafe or unwritable workspace directories; inspect ownership and symlinks on the host. No outside directory was used.".into()),
        }
    }
    for (relative, directory) in [
        ("tmp", true),
        ("tools", true),
        (".git", true),
        ("code/.git", true),
        ("CODE.md", false),
        ("scripts/code.py", false),
        ("scripts/workspace.py", false),
    ] {
        let mut path = root.to_path_buf();
        let mut safe = true;
        for part in relative.split('/') {
            path.push(part);
            if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                safe = false;
                break;
            }
        }
        let good = safe
            && fs::metadata(&path).is_ok_and(|m| if directory { m.is_dir() } else { m.is_file() });
        lines.push(format!(
            "{}: {relative}{}",
            if good { "PASS" } else { "ACTION" },
            if good {
                ""
            } else {
                " missing or unsafe; inspect before repair"
            }
        ));
    }
    if runtime_safe
        && fs::symlink_metadata(root.join("tmp"))
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
    {
        lines.push(match tempfile::NamedTempFile::new_in(root.join("tmp")) {
            Ok(_) => "PASS: workspace temporary-file creation and cleanup.".into(),
            Err(_) => {
                "FAIL: workspace temp is not writable; correct ownership or free disk space.".into()
            }
        });
    }
    let mut paths = vec![
        root.join("tools/bin"),
        root.join("tools/venv/bin"),
        root.join("tools/cargo/bin"),
        root.join("tools/npm/bin"),
        root.join("tools/brew/bin"),
    ];
    if let Ok(value) = std::env::var("UWUBOT_READONLY_TOOL_PATH") {
        paths.extend(std::env::split_paths(&value).filter(|p| p.is_absolute()));
    }
    paths.extend(["/usr/local/bin", "/usr/bin", "/bin"].map(Into::into));
    for name in ["python3", "git", "rg", "node", "npm", "cargo", "rustc"] {
        let found = paths.iter().any(|p| {
            fs::metadata(p.join(name)).is_ok_and(|m| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    m.is_file() && m.permissions().mode() & 0o111 != 0
                }
                #[cfg(not(unix))]
                {
                    m.is_file()
                }
            })
        });
        lines.push(format!(
            "{}: {name} {}",
            if found { "FOUND" } else { "ACTION" },
            if found {
                "executable found (version not tested)"
            } else {
                "missing; install under workspace/tools when needed"
            }
        ));
    }
    lines.push("Source/release integrity: /update performs build and activation validation; doctor does not execute workspace scripts or install updates.".into());
    lines.join("\n")
}

pub fn inference_error(error: &anyhow::Error) -> &'static str {
    for cause in error.chain() {
        if let Some(http) = cause.downcast_ref::<reqwest::Error>() {
            if let Some(status) = http.status() {
                return match status.as_u16() {
                    401 | 403 => {
                        "authentication/access rejected (401/403); replace the key or check model permissions"
                    }
                    402 => "insufficient credit (402); fund the provider account",
                    404 | 410 => {
                        "endpoint/model missing or retired (404/410); verify the configured model ID"
                    }
                    429 => "rate limited (429); wait for capacity or quota recovery",
                    400 | 422 => {
                        "request/model compatibility rejected (400/422); check model capabilities and API compatibility"
                    }
                    500..=599 => "provider service failure (5xx); retry after recovery",
                    _ => "provider HTTP failure; inspect provider service status",
                };
            }
            if http.is_timeout() {
                return "request timed out; check service capacity and connectivity";
            }
            if http.is_connect() {
                return "connection failed; check service availability, DNS and network access";
            }
        }
    }
    // Match only to fixed labels; never return a raw URL, body, key, or provider error string.
    let text = format!("{error:#}").to_ascii_lowercase();
    if text.contains("function-calling") {
        "configured model lacks advertised function-calling support; choose a compatible TEE model explicitly"
    } else if text.contains("exact configured model") {
        "configured model absent from provider catalog; verify its current ID"
    } else if text.contains("attestation") || text.contains("tee") {
        "TEE capability/attestation validation failed; privacy requirements were preserved"
    } else if text.contains("timed out") {
        "diagnostic deadline exceeded; no working credential was established"
    } else {
        "invalid or incomplete provider response; check model/API compatibility"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn check_is_read_only_and_repair_creates_only_local_directories() {
        let root = tempfile::tempdir().unwrap();
        assert!(workspace(root.path(), false).contains("ACTION: tmp"));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        assert!(workspace(root.path(), true).contains("PASS: workspace temporary-file"));
        assert!(root.path().join("tools/home").is_dir());
    }
    #[test]
    fn errors_never_echo_sensitive_provider_details() {
        for value in [
            "bad https://secret@example.com key=my-private-key",
            "TEE nonce mismatch secret=123",
            "model lacks function-calling secret",
        ] {
            let result = inference_error(&anyhow::anyhow!(value));
            assert!(!result.contains("secret"));
            assert!(!result.contains("my-private-key"));
        }
    }
}
