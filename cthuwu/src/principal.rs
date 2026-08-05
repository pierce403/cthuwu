use crate::{
    contact::normalize_inbox_id,
    storage::{ensure_private_directory, restrict_file, sync_directory},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;

const OPERATOR_CONFIG_VERSION: u32 = 2;
const MAX_OPERATOR_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_OPERATORS: usize = 64;
const MAX_LABEL_CHARS: usize = 80;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrincipalRole {
    Operator,
    PendingOperator,
    RevokedOperator,
    User,
}

#[derive(Clone, Debug)]
pub struct OperatorStore {
    state_directory: PathBuf,
    path: PathBuf,
    xmtp_environment: String,
    operators: Vec<OperatorRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OperatorConfig {
    version: u32,
    xmtp_environment: String,
    operators: Vec<OperatorRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OperatorRecord {
    inbox_id: String,
    label: String,
    status: OperatorStatus,
    generation: u64,
    activation_token_hash: Option<String>,
    added_at_unix: u64,
    activated_at_unix: Option<u64>,
    #[serde(default)]
    activated_after_sent_at_ns: Option<String>,
    revoked_at_unix: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum OperatorStatus {
    Pending,
    Active,
    Revoked,
}

#[derive(Debug, Eq, PartialEq)]
pub struct PendingOperator {
    pub inbox_id: String,
    pub activation_token: String,
    pub generation: u64,
}

impl OperatorStore {
    pub fn new(data_dir: &Path, xmtp_environment: &str) -> Result<Self> {
        if !matches!(xmtp_environment, "dev" | "production" | "local") {
            bail!("invalid XMTP environment for operator configuration");
        }
        let state_directory = data_dir.join("state");
        ensure_private_directory(&state_directory)?;
        let path = state_directory.join("operators.json");
        reject_symlink(&path)?;

        let operators = match fs::metadata(&path) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.len() > MAX_OPERATOR_CONFIG_BYTES {
                    bail!("operator configuration must be a bounded regular file");
                }
                assert_owner_only(&metadata)?;
                let bytes =
                    fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
                let config: OperatorConfig =
                    serde_json::from_slice(&bytes).context("operator configuration is invalid")?;
                if config.version != OPERATOR_CONFIG_VERSION {
                    bail!(
                        "unsupported operator configuration version {}",
                        config.version
                    );
                }
                if config.xmtp_environment != xmtp_environment {
                    bail!(
                        "operator configuration belongs to XMTP environment {:?}, not {:?}",
                        config.xmtp_environment,
                        xmtp_environment
                    );
                }
                validate_records(config.operators)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", path.display()));
            }
        };

        Ok(Self {
            state_directory,
            path,
            xmtp_environment: xmtp_environment.to_owned(),
            operators,
        })
    }

    /// Classify the authenticated XMTP sender before any message content is parsed.
    #[cfg(test)]
    pub fn role_for(&self, authenticated_sender_inbox_id: &str) -> Result<PrincipalRole> {
        let inbox_id = normalize_inbox_id(authenticated_sender_inbox_id)?;
        Ok(
            match self
                .operators
                .iter()
                .find(|operator| operator.inbox_id == inbox_id)
                .map(|operator| operator.status)
            {
                Some(OperatorStatus::Active) => PrincipalRole::Operator,
                Some(OperatorStatus::Pending) => PrincipalRole::PendingOperator,
                Some(OperatorStatus::Revoked) => PrincipalRole::RevokedOperator,
                None => PrincipalRole::User,
            },
        )
    }

    /// Classify an XMTP message at the authorization epoch in which it was authenticated.
    /// Messages authored at or before activation remain non-privileged even if delivered later.
    pub fn role_for_message(
        &self,
        authenticated_sender_inbox_id: &str,
        authenticated_sent_at_ns: &str,
    ) -> Result<PrincipalRole> {
        let inbox_id = normalize_inbox_id(authenticated_sender_inbox_id)?;
        let sent_at_ns = parse_sent_at_ns(authenticated_sent_at_ns)?;
        let Some(operator) = self
            .operators
            .iter()
            .find(|operator| operator.inbox_id == inbox_id)
        else {
            return Ok(PrincipalRole::User);
        };
        Ok(match operator.status {
            OperatorStatus::Pending => PrincipalRole::PendingOperator,
            OperatorStatus::Revoked => PrincipalRole::RevokedOperator,
            OperatorStatus::Active => {
                let activation_boundary = operator
                    .activated_after_sent_at_ns
                    .as_deref()
                    .context("active operator is missing its activation message boundary")?;
                if sent_at_ns > parse_sent_at_ns(activation_boundary)? {
                    PrincipalRole::Operator
                } else {
                    PrincipalRole::PendingOperator
                }
            }
        })
    }

    /// Create a pending role. The inbox must prove fresh control by sending the returned token.
    pub fn add(&mut self, inbox_id: &str, label: &str) -> Result<PendingOperator> {
        let inbox_id = normalize_operator_inbox_id(inbox_id)?;
        let label = validate_label(label)?;
        let mut random = [0_u8; 32];
        getrandom::getrandom(&mut random)
            .map_err(|error| anyhow::anyhow!("generating operator activation token: {error}"))?;
        let activation_token = hex(&random);
        let activation_token_hash = hash_token(&activation_token);
        let now = unix_seconds();
        let mut operators = self.operators.clone();

        let generation = if let Some(existing) = operators
            .iter_mut()
            .find(|operator| operator.inbox_id == inbox_id)
        {
            if existing.status == OperatorStatus::Active {
                bail!("that XMTP inbox is already an active operator");
            }
            existing.generation = existing
                .generation
                .checked_add(1)
                .context("operator generation overflow")?;
            existing.label = label;
            existing.status = OperatorStatus::Pending;
            existing.activation_token_hash = Some(activation_token_hash);
            existing.added_at_unix = now;
            existing.activated_at_unix = None;
            existing.activated_after_sent_at_ns = None;
            existing.revoked_at_unix = None;
            existing.generation
        } else {
            if operators.len() >= MAX_OPERATORS {
                bail!("operator configuration reached the {MAX_OPERATORS}-operator limit");
            }
            operators.push(OperatorRecord {
                inbox_id: inbox_id.clone(),
                label,
                status: OperatorStatus::Pending,
                generation: 1,
                activation_token_hash: Some(activation_token_hash),
                added_at_unix: now,
                activated_at_unix: None,
                activated_after_sent_at_ns: None,
                revoked_at_unix: None,
            });
            1
        };
        operators.sort_by(|left, right| left.inbox_id.cmp(&right.inbox_id));
        self.commit(operators)?;
        Ok(PendingOperator {
            inbox_id,
            activation_token,
            generation,
        })
    }

    #[cfg(test)]
    pub fn activate(&mut self, inbox_id: &str, token: &str) -> Result<bool> {
        self.activate_at(inbox_id, token, &unix_nanoseconds().to_string())
    }

    pub fn activate_at(
        &mut self,
        inbox_id: &str,
        token: &str,
        authenticated_sent_at_ns: &str,
    ) -> Result<bool> {
        let inbox_id = normalize_operator_inbox_id(inbox_id)?;
        let sent_at_ns = parse_sent_at_ns(authenticated_sent_at_ns)?;
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(false);
        }
        let mut operators = self.operators.clone();
        let Some(operator) = operators
            .iter_mut()
            .find(|operator| operator.inbox_id == inbox_id)
        else {
            return Ok(false);
        };
        if operator.status != OperatorStatus::Pending
            || operator.activation_token_hash.as_deref() != Some(&hash_token(token))
        {
            return Ok(false);
        }
        operator.status = OperatorStatus::Active;
        operator.activation_token_hash = None;
        operator.activated_at_unix = Some(unix_seconds());
        operator.activated_after_sent_at_ns = Some(sent_at_ns.to_string());
        self.commit(operators)?;
        Ok(true)
    }

    /// Revocation is a tombstone: the inbox stays blocked instead of becoming a public user.
    pub fn revoke(&mut self, inbox_id: &str) -> Result<bool> {
        let inbox_id = normalize_operator_inbox_id(inbox_id)?;
        let mut operators = self.operators.clone();
        let Some(operator) = operators
            .iter_mut()
            .find(|operator| operator.inbox_id == inbox_id)
        else {
            return Ok(false);
        };
        if operator.status == OperatorStatus::Revoked {
            return Ok(false);
        }
        operator.status = OperatorStatus::Revoked;
        operator.activation_token_hash = None;
        operator.revoked_at_unix = Some(unix_seconds());
        self.commit(operators)?;
        Ok(true)
    }

    pub fn list(&self) -> impl Iterator<Item = (&str, &str, &'static str, u64)> {
        self.operators.iter().map(|operator| {
            (
                operator.inbox_id.as_str(),
                operator.label.as_str(),
                match operator.status {
                    OperatorStatus::Pending => "pending",
                    OperatorStatus::Active => "active",
                    OperatorStatus::Revoked => "revoked",
                },
                operator.generation,
            )
        })
    }

    fn commit(&mut self, operators: Vec<OperatorRecord>) -> Result<()> {
        self.save_records(&operators)?;
        self.operators = operators;
        Ok(())
    }

    fn save_records(&self, operators: &[OperatorRecord]) -> Result<()> {
        reject_symlink(&self.path)?;
        let config = OperatorConfig {
            version: OPERATOR_CONFIG_VERSION,
            xmtp_environment: self.xmtp_environment.clone(),
            operators: operators.to_vec(),
        };
        let mut encoded = serde_json::to_vec_pretty(&config)?;
        encoded.push(b'\n');
        if encoded.len() as u64 > MAX_OPERATOR_CONFIG_BYTES {
            bail!("operator configuration is too large");
        }

        let mut temp = NamedTempFile::new_in(&self.state_directory).with_context(|| {
            format!(
                "creating temporary operator configuration in {}",
                self.state_directory.display()
            )
        })?;
        restrict_file(temp.as_file(), "temporary operator configuration")?;
        temp.write_all(&encoded)?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path)
            .map_err(|error| error.error)
            .with_context(|| format!("replacing {}", self.path.display()))?;
        sync_directory(&self.state_directory)
    }
}

fn validate_records(records: Vec<OperatorRecord>) -> Result<Vec<OperatorRecord>> {
    if records.len() > MAX_OPERATORS {
        bail!("operator configuration exceeds the {MAX_OPERATORS}-operator limit");
    }
    let mut seen = std::collections::BTreeSet::new();
    for operator in &records {
        let normalized = normalize_operator_inbox_id(&operator.inbox_id)?;
        if normalized != operator.inbox_id || !seen.insert(normalized) {
            bail!("operator configuration contains a non-canonical or duplicate inbox ID");
        }
        validate_label(&operator.label)?;
        if operator.generation == 0 {
            bail!("operator generation must be positive");
        }
        match operator.status {
            OperatorStatus::Pending => {
                let hash = operator
                    .activation_token_hash
                    .as_deref()
                    .context("pending operator is missing an activation-token hash")?;
                if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    bail!("pending operator has an invalid activation-token hash");
                }
                if operator.activated_after_sent_at_ns.is_some() {
                    bail!("pending operator must not have an activation message boundary");
                }
            }
            OperatorStatus::Active | OperatorStatus::Revoked
                if operator.activation_token_hash.is_some() =>
            {
                bail!("non-pending operator must not retain an activation-token hash");
            }
            _ => {}
        }
        if operator.status == OperatorStatus::Active {
            let boundary = operator
                .activated_after_sent_at_ns
                .as_deref()
                .context("active operator is missing an activation message boundary")?;
            parse_sent_at_ns(boundary)?;
        } else if let Some(boundary) = operator.activated_after_sent_at_ns.as_deref() {
            parse_sent_at_ns(boundary)?;
        }
    }
    Ok(records)
}

fn normalize_operator_inbox_id(value: &str) -> Result<String> {
    let value = normalize_inbox_id(value)?;
    if value.len() != 64 {
        bail!("operator roles require the full 64-character XMTP inbox ID");
    }
    Ok(value)
}

fn parse_sent_at_ns(value: &str) -> Result<u128> {
    if value.is_empty() || value.len() > 32 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("invalid authenticated XMTP sentAtNs value");
    }
    value
        .parse::<u128>()
        .context("authenticated XMTP sentAtNs is out of range")
}

fn validate_label(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_LABEL_CHARS
        || value.chars().any(char::is_control)
    {
        bail!("operator label must be 1-{MAX_LABEL_CHARS} printable characters");
    }
    Ok(value.to_owned())
}

fn hash_token(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(DIGITS[(byte >> 4) as usize] as char);
        value.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    value
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
fn unix_nanoseconds() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "operator configuration {} must not be a symlink",
                path.display()
            )
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

#[cfg(unix)]
fn assert_owner_only(metadata: &fs::Metadata) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o077 != 0 {
        bail!("operator configuration must not be accessible by group or other users");
    }
    Ok(())
}

#[cfg(not(unix))]
fn assert_owner_only(_metadata: &fs::Metadata) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPERATOR_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER_OPERATOR_ID: &str =
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn operator_activation_is_fresh_exact_persistent_and_one_time() {
        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::new(root.path(), "production").unwrap();
        let pending = store
            .add(&OPERATOR_ID.to_ascii_uppercase(), "Dean")
            .unwrap();
        assert_eq!(pending.inbox_id, OPERATOR_ID);
        assert_eq!(
            store.role_for(OPERATOR_ID).unwrap(),
            PrincipalRole::PendingOperator
        );
        assert!(!store.activate(OPERATOR_ID, &"0".repeat(64)).unwrap());
        assert!(
            store
                .activate(OPERATOR_ID, &pending.activation_token)
                .unwrap()
        );
        assert!(
            !store
                .activate(OPERATOR_ID, &pending.activation_token)
                .unwrap()
        );
        assert_eq!(
            store.role_for(OPERATOR_ID).unwrap(),
            PrincipalRole::Operator
        );
        assert_eq!(
            store.role_for(OTHER_OPERATOR_ID).unwrap(),
            PrincipalRole::User
        );

        let mut reloaded = OperatorStore::new(root.path(), "production").unwrap();
        assert_eq!(
            reloaded.list().collect::<Vec<_>>(),
            vec![(OPERATOR_ID, "Dean", "active", 1)]
        );
        assert!(reloaded.revoke(&OPERATOR_ID.to_ascii_uppercase()).unwrap());
        assert_eq!(
            OperatorStore::new(root.path(), "production")
                .unwrap()
                .role_for(OPERATOR_ID)
                .unwrap(),
            PrincipalRole::RevokedOperator
        );
    }

    #[test]
    fn malformed_inbox_ids_fail_before_role_selection() {
        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::new(root.path(), "dev").unwrap();
        assert!(store.role_for("operator").is_err());
        assert!(store.role_for("../aabbcc").is_err());
        assert!(store.add("aabbcc", "Dean").is_err());
    }

    #[test]
    fn active_role_rejects_messages_authored_before_its_activation_boundary() {
        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::new(root.path(), "production").unwrap();
        let pending = store.add(OPERATOR_ID, "Dean").unwrap();
        assert!(
            store
                .activate_at(OPERATOR_ID, &pending.activation_token, "200")
                .unwrap()
        );
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "199").unwrap(),
            PrincipalRole::PendingOperator
        );
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "200").unwrap(),
            PrincipalRole::PendingOperator
        );
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "201").unwrap(),
            PrincipalRole::Operator
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_activation_persistence_leaves_live_authority_pending() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::new(root.path(), "production").unwrap();
        let pending = store.add(OPERATOR_ID, "Dean").unwrap();
        let path = root.path().join("state/operators.json");
        let backup = root.path().join("state/operators.backup.json");
        fs::rename(&path, &backup).unwrap();
        symlink(&backup, &path).unwrap();

        assert!(
            store
                .activate_at(OPERATOR_ID, &pending.activation_token, "200")
                .is_err()
        );
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "201").unwrap(),
            PrincipalRole::PendingOperator
        );

        fs::remove_file(&path).unwrap();
        fs::rename(&backup, &path).unwrap();
        assert!(
            store
                .activate_at(OPERATOR_ID, &pending.activation_token, "200")
                .unwrap()
        );
    }

    #[test]
    fn environment_mismatch_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        OperatorStore::new(root.path(), "dev")
            .unwrap()
            .add(OPERATOR_ID, "Dean")
            .unwrap();
        assert!(OperatorStore::new(root.path(), "production").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn persisted_operator_acl_is_owner_only_and_rejects_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::new(root.path(), "dev").unwrap();
        store.add(OPERATOR_ID, "Dean").unwrap();
        let path = root.path().join("state/operators.json");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::remove_file(&path).unwrap();
        let outside = root.path().join("outside.json");
        fs::write(&outside, "{}\n").unwrap();
        symlink(&outside, &path).unwrap();
        assert!(OperatorStore::new(root.path(), "dev").is_err());
    }
}
