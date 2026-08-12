use crate::{
    contact::normalize_inbox_id,
    storage::{ensure_private_directory, restrict_file, sync_directory},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;

const OPERATOR_CONFIG_VERSION: u32 = 3;
const LEGACY_OPERATOR_CONFIG_VERSION: u32 = 2;
const MAX_OPERATOR_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_OPERATORS: usize = 64;
const MAX_LABEL_CHARS: usize = 80;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrincipalRole {
    Operator,
    StaleOperator,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activation_token_hash: Option<String>,
    added_at_unix: u64,
    #[serde(alias = "activated_at_unix")]
    authorized_at_unix: Option<u64>,
    #[serde(default, alias = "activated_after_sent_at_ns")]
    authorized_after_sent_at_ns: Option<String>,
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
pub struct AuthorizedOperator {
    pub inbox_id: String,
    pub generation: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub struct OperatorImprint {
    pub role: PrincipalRole,
    pub imprinted_address: Option<String>,
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

        let (operators, migrated) = match fs::metadata(&path) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.len() > MAX_OPERATOR_CONFIG_BYTES {
                    bail!("operator configuration must be a bounded regular file");
                }
                assert_owner_only(&metadata)?;
                let bytes =
                    fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
                let config: OperatorConfig =
                    serde_json::from_slice(&bytes).context("operator configuration is invalid")?;
                if config.xmtp_environment != xmtp_environment {
                    bail!(
                        "operator configuration belongs to XMTP environment {:?}, not {:?}",
                        config.xmtp_environment,
                        xmtp_environment
                    );
                }
                match config.version {
                    OPERATOR_CONFIG_VERSION => (validate_records(config.operators)?, false),
                    LEGACY_OPERATOR_CONFIG_VERSION => {
                        let records = validate_legacy_records(config.operators)?;
                        (
                            validate_records(migrate_legacy_records(records, unix_nanoseconds()))?,
                            true,
                        )
                    }
                    version => bail!("unsupported operator configuration version {version}"),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Vec::new(), false),
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", path.display()));
            }
        };

        let store = Self {
            state_directory,
            path,
            xmtp_environment: xmtp_environment.to_owned(),
            operators,
        };
        if migrated {
            store.save_records(&store.operators)?;
        }
        Ok(store)
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
                Some(OperatorStatus::Pending) => PrincipalRole::StaleOperator,
                Some(OperatorStatus::Revoked) => PrincipalRole::RevokedOperator,
                None => PrincipalRole::User,
            },
        )
    }

    /// Classify an XMTP message at the authorization epoch in which it was authenticated.
    /// Messages authored at or before authorization remain non-privileged even if delivered later.
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
            OperatorStatus::Pending => PrincipalRole::StaleOperator,
            OperatorStatus::Revoked => PrincipalRole::RevokedOperator,
            OperatorStatus::Active => {
                let authorization_boundary = operator
                    .authorized_after_sent_at_ns
                    .as_deref()
                    .context("active operator is missing its authorization boundary")?;
                if sent_at_ns > parse_sent_at_ns(authorization_boundary)? {
                    PrincipalRole::Operator
                } else {
                    PrincipalRole::StaleOperator
                }
            }
        })
    }

    /// Atomically classify an authenticated sender and, only when no operator history exists,
    /// imprint the first sender with an SDK-authenticated Ethereum address. The triggering message
    /// remains fenced at the public/stale boundary and gains no operator authority itself.
    pub fn classify_or_imprint(
        &mut self,
        authenticated_sender_inbox_id: &str,
        authenticated_sender_address: Option<&str>,
        authenticated_sent_at_ns: &str,
    ) -> Result<OperatorImprint> {
        let existing_role =
            self.role_for_message(authenticated_sender_inbox_id, authenticated_sent_at_ns)?;
        if existing_role != PrincipalRole::User || !self.operators.is_empty() {
            return Ok(OperatorImprint {
                role: existing_role,
                imprinted_address: None,
            });
        }
        let Some(address) = authenticated_sender_address else {
            return Ok(OperatorImprint {
                role: PrincipalRole::User,
                imprinted_address: None,
            });
        };
        let address = normalize_operator_address(address)?;
        self.add_at(
            authenticated_sender_inbox_id,
            &address,
            authenticated_sent_at_ns,
        )?;
        Ok(OperatorImprint {
            role: PrincipalRole::User,
            imprinted_address: Some(address),
        })
    }

    pub fn ensure_sole_operator(&mut self, inbox_id: &str, label: &str) -> Result<bool> {
        let inbox_id = normalize_operator_inbox_id(inbox_id)?;
        if let Some(active) = self
            .operators
            .iter()
            .find(|operator| operator.status == OperatorStatus::Active)
        {
            if active.inbox_id == inbox_id {
                return Ok(false);
            }
            bail!("this Tentacle already has its sole active operator");
        }
        self.add(&inbox_id, label)?;
        Ok(true)
    }

    /// Authorize an inbox immediately while fencing messages authored before the local grant.
    pub fn add(&mut self, inbox_id: &str, label: &str) -> Result<AuthorizedOperator> {
        self.add_at(inbox_id, label, &unix_nanoseconds().to_string())
    }

    pub(crate) fn add_at(
        &mut self,
        inbox_id: &str,
        label: &str,
        authorized_after_sent_at_ns: &str,
    ) -> Result<AuthorizedOperator> {
        let inbox_id = normalize_operator_inbox_id(inbox_id)?;
        let label = validate_label(label)?;
        let authorization_boundary = parse_sent_at_ns(authorized_after_sent_at_ns)?;
        let now = unix_seconds();
        let mut operators = self.operators.clone();

        if operators.iter().any(|operator| {
            operator.status == OperatorStatus::Active && operator.inbox_id != inbox_id
        }) {
            bail!("this Tentacle already has its sole active operator");
        }

        let generation = if let Some(existing) = operators
            .iter_mut()
            .find(|operator| operator.inbox_id == inbox_id)
        {
            existing.generation = existing
                .generation
                .checked_add(1)
                .context("operator generation overflow")?;
            existing.label = label;
            existing.status = OperatorStatus::Active;
            existing.activation_token_hash = None;
            existing.added_at_unix = now;
            existing.authorized_at_unix = Some(now);
            existing.authorized_after_sent_at_ns = Some(authorization_boundary.to_string());
            existing.revoked_at_unix = None;
            existing.generation
        } else {
            if operators.len() >= MAX_OPERATORS {
                bail!("operator configuration reached the {MAX_OPERATORS}-operator limit");
            }
            operators.push(OperatorRecord {
                inbox_id: inbox_id.clone(),
                label,
                status: OperatorStatus::Active,
                generation: 1,
                activation_token_hash: None,
                added_at_unix: now,
                authorized_at_unix: Some(now),
                authorized_after_sent_at_ns: Some(authorization_boundary.to_string()),
                revoked_at_unix: None,
            });
            1
        };
        operators.sort_by(|left, right| left.inbox_id.cmp(&right.inbox_id));
        self.commit(operators)?;
        Ok(AuthorizedOperator {
            inbox_id,
            generation,
        })
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
    validate_common_records(&records)?;
    if records
        .iter()
        .filter(|operator| operator.status == OperatorStatus::Active)
        .count()
        > 1
    {
        bail!("operator configuration contains more than one active operator");
    }
    for operator in &records {
        if operator.status == OperatorStatus::Pending {
            bail!("operator configuration version 3 does not permit pending roles");
        }
        if operator.activation_token_hash.is_some() {
            bail!("operator configuration version 3 must not contain activation proofs");
        }
        if operator.status == OperatorStatus::Active {
            operator
                .authorized_at_unix
                .context("active operator is missing its authorization time")?;
            let boundary = operator
                .authorized_after_sent_at_ns
                .as_deref()
                .context("active operator is missing its authorization boundary")?;
            parse_sent_at_ns(boundary)?;
        } else if let Some(boundary) = operator.authorized_after_sent_at_ns.as_deref() {
            parse_sent_at_ns(boundary)?;
        }
    }
    Ok(records)
}

fn validate_legacy_records(records: Vec<OperatorRecord>) -> Result<Vec<OperatorRecord>> {
    validate_common_records(&records)?;
    for operator in &records {
        match operator.status {
            OperatorStatus::Pending => {
                let hash = operator
                    .activation_token_hash
                    .as_deref()
                    .context("pending operator is missing an activation-token hash")?;
                if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    bail!("pending operator has an invalid activation-token hash");
                }
                if operator.authorized_after_sent_at_ns.is_some() {
                    bail!("pending operator must not have an authorization boundary");
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
            operator
                .authorized_at_unix
                .context("active operator is missing its authorization time")?;
            let boundary = operator
                .authorized_after_sent_at_ns
                .as_deref()
                .context("active operator is missing its authorization boundary")?;
            parse_sent_at_ns(boundary)?;
        } else if let Some(boundary) = operator.authorized_after_sent_at_ns.as_deref() {
            parse_sent_at_ns(boundary)?;
        }
    }
    Ok(records)
}

fn validate_common_records(records: &[OperatorRecord]) -> Result<()> {
    if records.len() > MAX_OPERATORS {
        bail!("operator configuration exceeds the {MAX_OPERATORS}-operator limit");
    }
    let mut seen = std::collections::BTreeSet::new();
    for operator in records {
        let normalized = normalize_operator_inbox_id(&operator.inbox_id)?;
        if normalized != operator.inbox_id || !seen.insert(normalized) {
            bail!("operator configuration contains a non-canonical or duplicate inbox ID");
        }
        validate_label(&operator.label)?;
        if operator.generation == 0 {
            bail!("operator generation must be positive");
        }
    }
    Ok(())
}

fn migrate_legacy_records(
    mut records: Vec<OperatorRecord>,
    authorization_boundary: u128,
) -> Vec<OperatorRecord> {
    for operator in &mut records {
        if operator.status == OperatorStatus::Pending {
            operator.status = OperatorStatus::Active;
            operator.authorized_at_unix = Some(unix_seconds());
            operator.authorized_after_sent_at_ns = Some(authorization_boundary.to_string());
        }
        operator.activation_token_hash = None;
    }
    records
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

fn normalize_operator_address(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() != 42
        || !value.starts_with("0x")
        || !value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("automatic operator imprint requires an SDK-authenticated Ethereum address");
    }
    Ok(value)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

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
    fn operator_authorization_is_immediate_exact_persistent_and_revocable() {
        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::new(root.path(), "production").unwrap();
        let authorized = store
            .add_at(&OPERATOR_ID.to_ascii_uppercase(), "Dean", "200")
            .unwrap();
        assert_eq!(authorized.inbox_id, OPERATOR_ID);
        assert_eq!(
            store.role_for(OPERATOR_ID).unwrap(),
            PrincipalRole::Operator
        );
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "200").unwrap(),
            PrincipalRole::StaleOperator
        );
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "201").unwrap(),
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
    fn active_role_rejects_messages_authored_before_its_authorization_boundary() {
        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::new(root.path(), "production").unwrap();
        store.add_at(OPERATOR_ID, "Dean", "200").unwrap();
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "199").unwrap(),
            PrincipalRole::StaleOperator
        );
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "200").unwrap(),
            PrincipalRole::StaleOperator
        );
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "201").unwrap(),
            PrincipalRole::Operator
        );
    }

    #[test]
    fn first_authenticated_evm_sender_is_atomically_imprinted_but_trigger_stays_public() {
        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::new(root.path(), "production").unwrap();
        let address = "0x1234567890abcdef1234567890abcdef12345678";

        let missing_address = store.classify_or_imprint(OPERATOR_ID, None, "100").unwrap();
        assert_eq!(missing_address.role, PrincipalRole::User);
        assert_eq!(missing_address.imprinted_address, None);

        let imprint = store
            .classify_or_imprint(OPERATOR_ID, Some(address), "101")
            .unwrap();
        assert_eq!(imprint.role, PrincipalRole::User);
        assert_eq!(imprint.imprinted_address.as_deref(), Some(address));
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "101").unwrap(),
            PrincipalRole::StaleOperator
        );
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "102").unwrap(),
            PrincipalRole::Operator
        );

        let second = store
            .classify_or_imprint(
                OTHER_OPERATOR_ID,
                Some("0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"),
                "102",
            )
            .unwrap();
        assert_eq!(second.role, PrincipalRole::User);
        assert_eq!(second.imprinted_address, None);
        assert!(store.add_at(OTHER_OPERATOR_ID, "other", "102").is_err());

        assert!(store.revoke(OPERATOR_ID).unwrap());
        let after_revocation = store
            .classify_or_imprint(
                OTHER_OPERATOR_ID,
                Some("0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"),
                "103",
            )
            .unwrap();
        assert_eq!(after_revocation.role, PrincipalRole::User);
        assert_eq!(after_revocation.imprinted_address, None);

        let reloaded = OperatorStore::new(root.path(), "production").unwrap();
        assert_eq!(reloaded.list().count(), 1);
        assert_eq!(
            reloaded.role_for_message(OPERATOR_ID, "104").unwrap(),
            PrincipalRole::RevokedOperator
        );
    }

    #[test]
    fn explicit_sole_operator_configuration_is_idempotent_and_rejects_replacement() {
        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::new(root.path(), "production").unwrap();
        assert!(store.ensure_sole_operator(OPERATOR_ID, "Dean").unwrap());
        assert!(!store.ensure_sole_operator(OPERATOR_ID, "Dean").unwrap());
        assert!(
            store
                .ensure_sole_operator(OTHER_OPERATOR_ID, "Other")
                .is_err()
        );
        assert_eq!(store.list().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn persisted_configuration_with_two_active_operators_fails_closed() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let record = |inbox_id: &str| {
            serde_json::json!({
                "inbox_id": inbox_id,
                "label": "operator",
                "status": "active",
                "generation": 1,
                "added_at_unix": 1,
                "authorized_at_unix": 1,
                "authorized_after_sent_at_ns": "1",
                "revoked_at_unix": null
            })
        };
        let path = state.join("operators.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": 3,
                "xmtp_environment": "production",
                "operators": [record(OPERATOR_ID), record(OTHER_OPERATOR_ID)]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        assert!(OperatorStore::new(root.path(), "production").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn failed_reauthorization_persistence_leaves_live_authority_unchanged() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let mut store = OperatorStore::new(root.path(), "production").unwrap();
        store.add_at(OPERATOR_ID, "Dean", "200").unwrap();
        let path = root.path().join("state/operators.json");
        let backup = root.path().join("state/operators.backup.json");
        fs::rename(&path, &backup).unwrap();
        symlink(&backup, &path).unwrap();

        assert!(store.add_at(OPERATOR_ID, "Dean updated", "300").is_err());
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "250").unwrap(),
            PrincipalRole::Operator
        );
        assert_eq!(
            store.role_for_message(OPERATOR_ID, "200").unwrap(),
            PrincipalRole::StaleOperator
        );

        fs::remove_file(&path).unwrap();
        fs::rename(&backup, &path).unwrap();
        assert_eq!(
            store
                .add_at(OPERATOR_ID, "Dean updated", "300")
                .unwrap()
                .generation,
            2
        );
    }

    #[cfg(unix)]
    #[test]
    fn version_two_pending_record_migrates_to_active_without_a_proof() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let path = state.join("operators.json");
        let legacy = serde_json::json!({
            "version": 2,
            "xmtp_environment": "production",
            "operators": [{
                "inbox_id": OPERATOR_ID,
                "label": "Dean",
                "status": "pending",
                "generation": 1,
                "activation_token_hash": "a".repeat(64),
                "added_at_unix": 200,
                "activated_at_unix": null,
                "activated_after_sent_at_ns": null,
                "revoked_at_unix": null
            }]
        });
        fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        let before_migration = unix_nanoseconds();
        let store = OperatorStore::new(root.path(), "production").unwrap();
        let after_migration = unix_nanoseconds();
        assert_eq!(
            store.role_for(OPERATOR_ID).unwrap(),
            PrincipalRole::Operator
        );
        let migrated: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(migrated["version"], 3);
        assert_eq!(migrated["operators"][0]["status"], "active");
        assert!(
            migrated["operators"][0]
                .get("activation_token_hash")
                .is_none()
        );
        let boundary = migrated["operators"][0]["authorized_after_sent_at_ns"]
            .as_str()
            .unwrap()
            .parse::<u128>()
            .unwrap();
        assert!(boundary >= before_migration && boundary <= after_migration);
        assert_eq!(
            store
                .role_for_message(OPERATOR_ID, &boundary.to_string())
                .unwrap(),
            PrincipalRole::StaleOperator
        );
        assert_eq!(
            store
                .role_for_message(OPERATOR_ID, &(boundary + 1).to_string())
                .unwrap(),
            PrincipalRole::Operator
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
