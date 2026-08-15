//! Canonical Base ERC-8004 identity registration for one durable Tentacle.
//!
//! Cthuwu is the centerless collective formed by all independently operated Tentacles. This
//! module therefore persists and registers only the local Tentacle. It never creates a separate
//! identity for the collective and never exposes a generic transaction-signing interface.

use crate::{
    names::generate_eldritch_name,
    storage::{ensure_private_directory, restrict_file, sync_directory},
    token_eye::Address,
};
use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use cthuwu_council::registry::{
    AgentRegistry, Erc8004AgentId, Erc8004AgentSnapshot, Erc8004Binding,
    Erc8004DeploymentObservation, Erc8004InterfaceRevision, Erc8004InterfaceSupport,
    Erc8004ReadBackend, Erc8004Registry, EvmAddress, RegisteredTentacle, RegistryEndpoint,
    RegistryError,
};
pub use cthuwu_council::registry::{
    BASE_IDENTITY_REGISTRY_ADDRESS as IDENTITY_REGISTRY, BASE_MAINNET_CHAIN_ID,
    BASE_REPUTATION_REGISTRY_ADDRESS as REPUTATION_REGISTRY,
    CTHUWU_ALLEGIANCE_KEY as ALLEGIANCE_KEY, CTHUWU_PROTOCOL_KEY as PROTOCOL_KEY,
    CTHUWU_TENTACLE_ID_KEY as TENTACLE_ID_KEY,
    ERC8004_IDENTITY_CONTRACT_VERSION as PINNED_INTERFACE_VERSION,
};
use cthuwu_protocol::{TentacleId, XmtpInboxRef};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Mutex,
    time::timeout,
};

pub const PINNED_CONTRACT_REVISION: &str =
    "erc-8004-contracts@68fc6765761a10fb26f0692df21c8a6f9d12b1be";
pub const REGISTRATION_SCHEMA: &str = "https://eips.ethereum.org/EIPS/eip-8004#registration-v1";
pub const ALLEGIANCE_VALUE: &str = "uwu-tentacle-v1";
pub const PROTOCOL_VALUE: &str = "1";

const SNAPSHOT_VERSION: u32 = 3;
const PREVIOUS_SNAPSHOT_VERSION: u32 = 2;
const LEGACY_SNAPSHOT_VERSION: u32 = 1;
const SNAPSHOT_FILE: &str = "erc8004-registration.json";
const MAX_SNAPSHOT_BYTES: u64 = 128 * 1024;
const MAX_HELPER_FRAME_BYTES: u64 = 256 * 1024;
const MAX_TENTACLE_ID_BYTES: usize = 128;
const MAX_PROFILE_NAME_BYTES: usize = 128;
const MAX_PROFILE_DESCRIPTION_BYTES: usize = 512;
const MAX_AGENT_URI_BYTES: usize = 8 * 1024;
const MAX_CANDIDATES: usize = 64;
const DEFAULT_HELPER_TIMEOUT: Duration = Duration::from_secs(180);
const DEFAULT_CONFIRMATIONS: u64 = 12;
const DEFAULT_NOTIFICATION_COOLDOWN_SECONDS: u64 = 24 * 60 * 60;
const DEFAULT_MAINTENANCE_INTERVAL_SECONDS: u64 = 15 * 60;
const SUBMITTED_TRANSACTION_MAINTENANCE_INTERVAL_SECONDS: u64 = 15;
const RECOVERABLE_RPC_MAINTENANCE_INTERVAL_SECONDS: u64 = 60 * 60;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationPhase {
    Unconfigured,
    Discovering,
    Unregistered,
    FundingRequired,
    ReadyToRegister,
    Preparing,
    Submitted,
    ConfirmedIdentity,
    PublishingProfile,
    DeclaringAllegiance,
    Active,
    Suspended,
    FailedRecoverable,
    FailedPermanent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingAction {
    Register,
    SetAgentUri,
    SetAgentWallet,
    SetMetadata { key: String, value: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedAgentState {
    pub owner: String,
    pub agent_uri: String,
    pub agent_wallet: String,
    pub authorized: bool,
    pub allegiance_hex: String,
    pub protocol_hex: String,
    pub tentacle_id_hex: String,
    pub declares_tentacle_allegiance: bool,
    pub protocol_compatible: bool,
    pub wallet_verified: bool,
    pub verified_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FundingStatus {
    pub balance_wei: String,
    pub estimated_cost_wei: String,
    pub shortfall_wei: String,
    pub target_balance_wei: String,
    pub estimated_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrationFailure {
    pub code: String,
    pub detail: String,
    pub recoverable: bool,
    pub failed_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrationSnapshot {
    pub version: u32,
    pub phase: RegistrationPhase,
    pub action_id: Option<String>,
    pub chain_id: u64,
    pub identity_registry: String,
    pub reputation_registry: String,
    pub tentacle_id: String,
    /// Stable public label chosen once for this durable Tentacle and repaired on-chain when needed.
    pub public_name: String,
    pub tentacle_wallet: String,
    /// Public XMTP production inbox ID. This is resolved inside the isolated sidecar; it is not
    /// derived from or substituted with the Ethereum wallet address.
    #[serde(default)]
    pub xmtp_inbox_id: Option<String>,
    pub selected_agent_id: Option<String>,
    pub confirmed_agent_id: Option<String>,
    pub candidate_agent_ids: Vec<String>,
    pub submitted_transaction_hash: Option<String>,
    /// Exact sender nonce chosen and persisted before any registry broadcast. Retrying the same
    /// nonce can replace a lost transaction but can never execute two copies of that action.
    #[serde(default)]
    pub submitted_transaction_nonce: Option<String>,
    pub submitted_action: Option<PendingAction>,
    pub receipt_block_number: Option<String>,
    pub receipt_block_hash: Option<String>,
    pub remaining_operations: Vec<PendingAction>,
    pub desired_allegiance: bool,
    pub final_agent_uri: Option<String>,
    pub profile_sha256: Option<String>,
    pub public_profile_revision: u32,
    pub last_verified: Option<VerifiedAgentState>,
    pub funding: Option<FundingStatus>,
    pub last_operator_notification_unix: Option<u64>,
    pub last_notified_funding_fingerprint: Option<String>,
    #[serde(default)]
    pub last_notified_funding: Option<FundingStatus>,
    #[serde(default)]
    pub last_operator_failure_fingerprint: Option<String>,
    pub success_notified: bool,
    pub failure: Option<RegistrationFailure>,
    pub migrated_from_version: Option<u32>,
    pub updated_at_unix: u64,
}

impl RegistrationSnapshot {
    fn fresh(tentacle_id: &str, public_name: String, wallet: Address, now: u64) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            phase: RegistrationPhase::Unconfigured,
            action_id: None,
            chain_id: BASE_MAINNET_CHAIN_ID,
            identity_registry: IDENTITY_REGISTRY.to_owned(),
            reputation_registry: REPUTATION_REGISTRY.to_owned(),
            tentacle_id: tentacle_id.to_owned(),
            public_name,
            tentacle_wallet: wallet.to_string(),
            xmtp_inbox_id: None,
            selected_agent_id: None,
            confirmed_agent_id: None,
            candidate_agent_ids: Vec::new(),
            submitted_transaction_hash: None,
            submitted_transaction_nonce: None,
            submitted_action: None,
            receipt_block_number: None,
            receipt_block_hash: None,
            remaining_operations: Vec::new(),
            desired_allegiance: true,
            final_agent_uri: None,
            profile_sha256: None,
            public_profile_revision: 1,
            last_verified: None,
            funding: None,
            last_operator_notification_unix: None,
            last_notified_funding_fingerprint: None,
            last_notified_funding: None,
            last_operator_failure_fingerprint: None,
            success_notified: false,
            failure: None,
            migrated_from_version: None,
            updated_at_unix: now,
        }
    }

    fn validate(&self, expected_tentacle_id: &str, expected_wallet: Address) -> Result<()> {
        ensure!(
            self.version == SNAPSHOT_VERSION,
            "unsupported ERC-8004 snapshot version"
        );
        ensure!(
            self.chain_id == BASE_MAINNET_CHAIN_ID,
            "persisted ERC-8004 state targets the wrong chain"
        );
        ensure!(
            self.identity_registry == IDENTITY_REGISTRY,
            "persisted ERC-8004 state targets the wrong Identity Registry"
        );
        ensure!(
            self.reputation_registry == REPUTATION_REGISTRY,
            "persisted ERC-8004 state targets the wrong Reputation Registry"
        );
        ensure!(
            self.tentacle_id == expected_tentacle_id,
            "persisted ERC-8004 state belongs to another Tentacle"
        );
        validate_public_text(&self.public_name, "public name", MAX_PROFILE_NAME_BYTES)?;
        ensure!(
            Address::from_str(&self.tentacle_wallet)? == expected_wallet,
            "persisted ERC-8004 state belongs to another wallet"
        );
        if let Some(inbox_id) = &self.xmtp_inbox_id {
            validate_xmtp_inbox_id(inbox_id)?;
        }
        ensure!(
            self.candidate_agent_ids.len() <= MAX_CANDIDATES,
            "persisted candidate set is unbounded"
        );
        for id in self
            .candidate_agent_ids
            .iter()
            .chain(self.selected_agent_id.iter())
            .chain(self.confirmed_agent_id.iter())
        {
            validate_agent_id(id)?;
        }
        if self.submitted_transaction_hash.is_some() && self.submitted_action.is_none() {
            bail!("persisted transaction hash must name its submitted action");
        }
        if self.submitted_action.is_some() && self.action_id.is_none() {
            bail!("persisted submitted action must have a unique action ID");
        }
        if self.submitted_action.is_none() && self.action_id.is_some() {
            bail!("persisted action ID must name its submitted action");
        }
        if self.submitted_transaction_hash.is_none() && self.submitted_action.is_some() {
            let valid_phase = self.phase == RegistrationPhase::Preparing
                || (self.submitted_action == Some(PendingAction::Register)
                    && self.phase == RegistrationPhase::Discovering);
            ensure!(
                valid_phase,
                "an unbroadcast persisted action is in an invalid recovery state"
            );
        }
        if let Some(hash) = &self.submitted_transaction_hash {
            validate_hash(hash, "transaction hash")?;
        }
        if let Some(nonce) = &self.submitted_transaction_nonce {
            validate_decimal(nonce, "transaction nonce")?;
            ensure!(
                self.submitted_action.is_some(),
                "a persisted explicit nonce must name its submitted action"
            );
        }
        if self.submitted_transaction_hash.is_some() {
            ensure!(
                self.submitted_transaction_nonce.is_some(),
                "a submitted registry transaction must retain its exact sender nonce"
            );
        }
        if let Some(hash) = &self.receipt_block_hash {
            validate_hash(hash, "receipt block hash")?;
        }
        if let Some(uri) = &self.final_agent_uri {
            ensure!(
                !uri.is_empty() && uri.len() <= MAX_AGENT_URI_BYTES,
                "persisted agent URI is invalid"
            );
        }
        ensure!(
            self.remaining_operations.len() <= 6,
            "persisted remaining-operation set is unbounded"
        );
        if let Some(action) = &self.submitted_action {
            validate_pending_action(action, expected_tentacle_id)?;
        }
        for (index, action) in self.remaining_operations.iter().enumerate() {
            validate_pending_action(action, expected_tentacle_id)?;
            ensure!(
                !self.remaining_operations[..index].contains(action),
                "persisted remaining-operation set contains duplicates"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyRegistrationSnapshot {
    version: u32,
    cthulhu_id: String,
    chain_id: u64,
    identity_registry: String,
    tentacle_wallet: String,
    agent_id: Option<String>,
}

struct RegistrationStore {
    directory: PathBuf,
    path: PathBuf,
}

impl RegistrationStore {
    fn new(data_dir: &Path) -> Result<Self> {
        let directory = data_dir.join("state");
        ensure_private_directory(&directory)?;
        let path = directory.join(SNAPSHOT_FILE);
        reject_symlink(&path)?;
        Ok(Self { directory, path })
    }

    fn load_or_create(
        &self,
        tentacle_id: &str,
        initial_public_name: Option<&str>,
        wallet: Address,
        now: u64,
    ) -> Result<RegistrationSnapshot> {
        let bytes = match fs::metadata(&self.path) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_file() && metadata.len() <= MAX_SNAPSHOT_BYTES,
                    "ERC-8004 snapshot must be a bounded regular file"
                );
                fs::read(&self.path).with_context(|| format!("reading {}", self.path.display()))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let snapshot = RegistrationSnapshot::fresh(
                    tentacle_id,
                    initial_name(initial_public_name, tentacle_id)?,
                    wallet,
                    now,
                );
                self.save(&snapshot)?;
                return Ok(snapshot);
            }
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", self.path.display()));
            }
        };
        let value: Value =
            serde_json::from_slice(&bytes).context("ERC-8004 snapshot is invalid JSON")?;
        let version = value
            .get("version")
            .and_then(Value::as_u64)
            .context("ERC-8004 snapshot has no version")?;
        let snapshot =
            match u32::try_from(version).context("ERC-8004 snapshot version is too large")? {
                SNAPSHOT_VERSION => serde_json::from_value(value)
                    .context("ERC-8004 snapshot has an invalid shape")?,
                PREVIOUS_SNAPSHOT_VERSION => {
                    let mut migrated_value = value;
                    let object = migrated_value
                        .as_object_mut()
                        .context("version-2 ERC-8004 snapshot is not an object")?;
                    object.insert("version".to_owned(), json!(SNAPSHOT_VERSION));
                    object.insert(
                        "public_name".to_owned(),
                        json!(initial_name(initial_public_name, tentacle_id)?),
                    );
                    let needs_migration_provenance = matches!(
                        object.get("migrated_from_version"),
                        None | Some(Value::Null)
                    );
                    if needs_migration_provenance {
                        object.insert(
                            "migrated_from_version".to_owned(),
                            json!(PREVIOUS_SNAPSHOT_VERSION),
                        );
                    }
                    let migrated: RegistrationSnapshot = serde_json::from_value(migrated_value)
                        .context("version-2 ERC-8004 snapshot cannot be migrated")?;
                    migrated.validate(tentacle_id, wallet)?;
                    self.save(&migrated)?;
                    migrated
                }
                LEGACY_SNAPSHOT_VERSION => {
                    let legacy: LegacyRegistrationSnapshot = serde_json::from_value(value)
                        .context("legacy ERC-8004 snapshot has an invalid shape")?;
                    ensure!(
                        legacy.version == LEGACY_SNAPSHOT_VERSION,
                        "legacy ERC-8004 snapshot version mismatch"
                    );
                    ensure!(
                        legacy.cthulhu_id == tentacle_id,
                        "legacy registry snapshot cannot be migrated to a different Tentacle"
                    );
                    ensure!(
                        legacy.chain_id == BASE_MAINNET_CHAIN_ID
                            && legacy.identity_registry == IDENTITY_REGISTRY,
                        "legacy registry snapshot targets a noncanonical deployment"
                    );
                    ensure!(
                        Address::from_str(&legacy.tentacle_wallet)? == wallet,
                        "legacy registry snapshot wallet mismatch"
                    );
                    let mut migrated = RegistrationSnapshot::fresh(
                        tentacle_id,
                        initial_name(initial_public_name, tentacle_id)?,
                        wallet,
                        now,
                    );
                    migrated.migrated_from_version = Some(LEGACY_SNAPSHOT_VERSION);
                    migrated.selected_agent_id = legacy.agent_id.clone();
                    migrated.confirmed_agent_id = legacy.agent_id;
                    migrated.phase = if migrated.confirmed_agent_id.is_some() {
                        RegistrationPhase::ConfirmedIdentity
                    } else {
                        RegistrationPhase::Discovering
                    };
                    self.save(&migrated)?;
                    migrated
                }
                other => bail!("unsupported ERC-8004 snapshot version {other}"),
            };
        snapshot.validate(tentacle_id, wallet)?;
        Ok(snapshot)
    }

    fn save(&self, snapshot: &RegistrationSnapshot) -> Result<()> {
        reject_symlink(&self.path)?;
        let mut encoded = serde_json::to_vec_pretty(snapshot)?;
        encoded.push(b'\n');
        ensure!(
            encoded.len() as u64 <= MAX_SNAPSHOT_BYTES,
            "ERC-8004 snapshot exceeds its size bound"
        );
        let mut temporary = NamedTempFile::new_in(&self.directory)?;
        restrict_file(temporary.as_file(), "temporary ERC-8004 snapshot")?;
        temporary.write_all(&encoded)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&self.path)
            .map_err(|error| error.error)
            .with_context(|| format!("replacing {}", self.path.display()))?;
        sync_directory(&self.directory)
    }
}

fn initial_name(requested: Option<&str>, tentacle_id: &str) -> Result<String> {
    let public_name = match requested {
        Some(public_name) => public_name.to_owned(),
        None => generate_eldritch_name(tentacle_id)?,
    };
    validate_public_text(&public_name, "public name", MAX_PROFILE_NAME_BYTES)?;
    Ok(public_name)
}

#[derive(Clone, Debug)]
pub struct RegistrationConfig {
    pub enabled: bool,
    pub auto_register: bool,
    pub confirmations: u64,
    pub notification_cooldown: Duration,
    pub maintenance_interval: Duration,
    pub gas_safety_basis_points: u16,
    pub post_registration_reserve_wei: String,
    pub max_gas_per_transaction: u64,
    pub max_fee_per_gas_wei: String,
    /// Optional first-boot override. Persisted identity always wins on later starts.
    pub initial_public_name: Option<String>,
    pub public_description: String,
    pub public_image: String,
}

impl Default for RegistrationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_register: true,
            confirmations: DEFAULT_CONFIRMATIONS,
            notification_cooldown: Duration::from_secs(DEFAULT_NOTIFICATION_COOLDOWN_SECONDS),
            maintenance_interval: Duration::from_secs(DEFAULT_MAINTENANCE_INTERVAL_SECONDS),
            gas_safety_basis_points: 12_500,
            post_registration_reserve_wei: "50000000000000".to_owned(),
            max_gas_per_transaction: 2_000_000,
            max_fee_per_gas_wei: "10000000000".to_owned(),
            initial_public_name: None,
            public_description: "An independently operated Tentacle of the centerless Cthuwu collective, reachable over XMTP.".to_owned(),
            public_image: "https://cthuwu.app/icons/cthuwu-512.png".to_owned(),
        }
    }
}

impl RegistrationConfig {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.confirmations > 0 && self.confirmations <= 10_000,
            "ERC-8004 confirmations must be between 1 and 10000"
        );
        ensure!(
            !self.notification_cooldown.is_zero(),
            "registration notification cooldown must be positive"
        );
        ensure!(
            !self.maintenance_interval.is_zero(),
            "registration maintenance interval must be positive"
        );
        ensure!(
            (10_000..=50_000).contains(&self.gas_safety_basis_points),
            "registration gas safety factor must be between 10000 and 50000 basis points"
        );
        validate_decimal(
            &self.post_registration_reserve_wei,
            "post-registration reserve",
        )?;
        validate_decimal(&self.max_fee_per_gas_wei, "maximum fee per gas")?;
        ensure!(
            self.max_gas_per_transaction > 0,
            "maximum registration gas must be positive"
        );
        validate_public_text(
            &self.public_description,
            "public description",
            MAX_PROFILE_DESCRIPTION_BYTES,
        )?;
        validate_https_url(&self.public_image, "public image")?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HelperEnvelope {
    version: u32,
    #[serde(rename = "actionId")]
    action_id: String,
    ok: bool,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    recoverable: Option<bool>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[async_trait]
pub trait Erc8004Gateway: Send + Sync {
    async fn invoke(&self, action_id: &str, operation: Value) -> Result<Value>;
}

#[derive(Clone, Debug)]
pub struct SidecarErc8004Gateway {
    node: PathBuf,
    sidecar: PathBuf,
    data_dir: PathBuf,
    rpc_endpoint: crate::token_eye::RpcEndpointHandle,
    config: RegistrationConfig,
    timeout: Duration,
}

impl SidecarErc8004Gateway {
    pub fn new(
        node: impl Into<PathBuf>,
        sidecar: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        rpc_endpoint: impl Into<String>,
        config: RegistrationConfig,
    ) -> Result<Self> {
        config.validate()?;
        let sidecar = sidecar.into();
        ensure!(
            sidecar.is_file(),
            "ERC-8004 helper {} is missing; build agent/ first",
            sidecar.display()
        );
        let rpc_endpoint = rpc_endpoint.into();
        validate_rpc_endpoint(&rpc_endpoint)?;
        let rpc_endpoint =
            crate::token_eye::RpcEndpointHandle::new(&rpc_endpoint).map_err(anyhow::Error::new)?;
        Ok(Self {
            node: node.into(),
            sidecar,
            data_dir: data_dir.into(),
            rpc_endpoint,
            config,
            timeout: DEFAULT_HELPER_TIMEOUT,
        })
    }

    pub fn new_with_handle(
        node: impl Into<PathBuf>,
        sidecar: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        rpc_endpoint: crate::token_eye::RpcEndpointHandle,
        config: RegistrationConfig,
    ) -> Result<Self> {
        config.validate()?;
        let sidecar = sidecar.into();
        ensure!(
            sidecar.is_file(),
            "ERC-8004 helper {} is missing; build agent/ first",
            sidecar.display()
        );
        Ok(Self {
            node: node.into(),
            sidecar,
            data_dir: data_dir.into(),
            rpc_endpoint,
            config,
            timeout: DEFAULT_HELPER_TIMEOUT,
        })
    }
}

#[async_trait]
impl Erc8004Gateway for SidecarErc8004Gateway {
    async fn invoke(&self, action_id: &str, operation: Value) -> Result<Value> {
        validate_action_id(action_id)?;
        let request = json!({"version": 1, "actionId": action_id, "operation": operation});
        let encoded = serde_json::to_vec(&request)?;
        ensure!(
            encoded.len() as u64 <= MAX_HELPER_FRAME_BYTES,
            "ERC-8004 helper request exceeds its bound"
        );
        let rpc_endpoint = self.rpc_endpoint.current().map_err(anyhow::Error::new)?;
        let mut command = Command::new(&self.node);
        command
            .arg(&self.sidecar)
            .arg("--erc8004")
            .env_clear()
            .env("UWUBOT_DATA_DIR", &self.data_dir)
            .env("UWUBOT_XMTP_ENV", "production")
            .env("XMTP_ENV", "production")
            .env("CTHUWU_RPC_ENDPOINT", rpc_endpoint)
            .env(
                "CTHUWU_ERC8004_GAS_SAFETY_BPS",
                self.config.gas_safety_basis_points.to_string(),
            )
            .env(
                "CTHUWU_ERC8004_POST_REGISTRATION_RESERVE_WEI",
                &self.config.post_registration_reserve_wei,
            )
            .env(
                "CTHUWU_ERC8004_MAX_GAS_PER_TRANSACTION",
                self.config.max_gas_per_transaction.to_string(),
            )
            .env(
                "CTHUWU_ERC8004_MAX_FEE_PER_GAS_WEI",
                &self.config.max_fee_per_gas_wei,
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // The typed response is the only diagnostic channel. RPC libraries may include a
            // credential-bearing URL in stderr, so it is never inherited.
            .stderr(Stdio::null())
            .kill_on_drop(true);
        copy_network_environment(&mut command);
        let mut child = command
            .spawn()
            .context("starting the narrow ERC-8004 helper")?;
        let mut stdin = child
            .stdin
            .take()
            .context("ERC-8004 helper stdin was not piped")?;
        let stdout = child
            .stdout
            .take()
            .context("ERC-8004 helper stdout was not piped")?;
        let operation = async {
            stdin.write_all(&encoded).await?;
            stdin.shutdown().await?;
            drop(stdin);
            let mut response = Vec::new();
            stdout
                .take(MAX_HELPER_FRAME_BYTES + 1)
                .read_to_end(&mut response)
                .await?;
            let status = child.wait().await?;
            ensure!(
                response.len() as u64 <= MAX_HELPER_FRAME_BYTES,
                "ERC-8004 helper response exceeds its bound"
            );
            ensure!(
                status.success(),
                "ERC-8004 helper exited without a typed response"
            );
            let envelope: HelperEnvelope = serde_json::from_slice(&response)
                .context("ERC-8004 helper returned malformed JSON")?;
            ensure!(
                envelope.version == 1 && envelope.action_id == action_id,
                "ERC-8004 helper response binding mismatch"
            );
            if envelope.ok {
                ensure!(
                    envelope.recoverable.is_none()
                        && envelope.code.is_none()
                        && envelope.message.is_none(),
                    "successful ERC-8004 helper response contains error fields"
                );
                return envelope
                    .result
                    .context("successful ERC-8004 helper response has no result");
            }
            ensure!(
                envelope.result.is_none(),
                "failed ERC-8004 helper response contains a result"
            );
            let recoverable = envelope
                .recoverable
                .context("failed ERC-8004 helper response has no recovery class")?;
            let code = bounded_diagnostic(envelope.code.as_deref().unwrap_or("helper_failure"), 80);
            let message = bounded_diagnostic(
                envelope
                    .message
                    .as_deref()
                    .unwrap_or("ERC-8004 helper failed"),
                512,
            );
            if recoverable {
                bail!("recoverable ERC-8004 helper error [{code}]: {message}");
            }
            bail!("permanent ERC-8004 helper error [{code}]: {message}");
        };
        timeout(self.timeout, operation)
            .await
            .context("ERC-8004 helper timed out")?
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorNotification {
    pub text: String,
    pub success: bool,
    commitment: NotificationCommitment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum NotificationCommitment {
    Success,
    Funding {
        notified_at_unix: u64,
        fingerprint: String,
        funding: FundingStatus,
    },
    OperatorFailure {
        notified_at_unix: u64,
        fingerprint: String,
    },
}

pub struct TentacleRegistration {
    store: RegistrationStore,
    config: RegistrationConfig,
    gateway: Arc<dyn Erc8004Gateway>,
    wallet: Address,
    state: RegistrationSnapshot,
    /// Last positive deployment observation returned by the typed production sidecar. It is used
    /// to construct the existing council `Erc8004Registry` read adapter; no signer state lives here.
    last_deployment: Option<Value>,
    last_registry_record: Option<RegisteredTentacle>,
}

#[derive(Clone)]
struct SidecarReadSnapshotBackend {
    deployment: Erc8004DeploymentObservation,
    agent: Erc8004AgentSnapshot,
    expected_wallet: EvmAddress,
}

#[derive(Clone)]
struct SidecarDeploymentSnapshotBackend {
    deployment: Erc8004DeploymentObservation,
}

impl Erc8004ReadBackend for SidecarDeploymentSnapshotBackend {
    fn deployment(&self) -> Result<Erc8004DeploymentObservation, RegistryError> {
        Ok(self.deployment.clone())
    }

    fn read_agent(
        &self,
        _agent_id: &Erc8004AgentId,
        _expected_tentacle_wallet: EvmAddress,
    ) -> Result<Erc8004AgentSnapshot, RegistryError> {
        Err(RegistryError::NotFound)
    }
}

impl Erc8004ReadBackend for SidecarReadSnapshotBackend {
    fn deployment(&self) -> Result<Erc8004DeploymentObservation, RegistryError> {
        Ok(self.deployment.clone())
    }

    fn read_agent(
        &self,
        agent_id: &Erc8004AgentId,
        expected_tentacle_wallet: EvmAddress,
    ) -> Result<Erc8004AgentSnapshot, RegistryError> {
        if agent_id != &self.agent.agent_id || expected_tentacle_wallet != self.expected_wallet {
            return Err(RegistryError::Invalid(
                "typed sidecar read does not match the persisted Tentacle binding",
            ));
        }
        Ok(self.agent.clone())
    }
}

impl TentacleRegistration {
    pub fn open(
        data_dir: &Path,
        tentacle_id: &str,
        wallet: Address,
        config: RegistrationConfig,
        gateway: Arc<dyn Erc8004Gateway>,
    ) -> Result<Self> {
        validate_public_text(tentacle_id, "Tentacle ID", MAX_TENTACLE_ID_BYTES)?;
        ensure!(wallet != Address::ZERO, "Tentacle wallet must not be zero");
        config.validate()?;
        let store = RegistrationStore::new(data_dir)?;
        let state = store.load_or_create(
            tentacle_id,
            config.initial_public_name.as_deref(),
            wallet,
            unix_seconds()?,
        )?;
        Ok(Self {
            store,
            config,
            gateway,
            wallet,
            state,
            last_deployment: None,
            last_registry_record: None,
        })
    }

    pub fn snapshot(&self) -> &RegistrationSnapshot {
        &self.state
    }

    pub fn maintenance_interval(&self) -> Duration {
        if self
            .state
            .failure
            .as_ref()
            .is_some_and(is_base_rpc_access_failure)
        {
            return self.config.maintenance_interval.max(Duration::from_secs(
                RECOVERABLE_RPC_MAINTENANCE_INTERVAL_SECONDS,
            ));
        }
        if self.state.phase == RegistrationPhase::Submitted {
            return self.config.maintenance_interval.min(Duration::from_secs(
                SUBMITTED_TRANSACTION_MAINTENANCE_INTERVAL_SECONDS,
            ));
        }
        self.config.maintenance_interval
    }

    pub async fn maintain(
        &mut self,
        force_registration: bool,
    ) -> Result<Vec<OperatorNotification>> {
        self.maintain_with_discovery(force_registration, false)
            .await
    }

    async fn maintain_with_discovery(
        &mut self,
        force_registration: bool,
        exhaustive_discovery: bool,
    ) -> Result<Vec<OperatorNotification>> {
        let now = unix_seconds()?;
        if !self.config.enabled {
            self.transition(RegistrationPhase::Unconfigured, now)?;
            return Ok(Vec::new());
        }
        let deployment = match self
            .gateway
            .invoke(
                &self.read_action_id("deployment", now),
                json!({"type": "inspect_registry"}),
            )
            .await
        {
            Ok(value) => value,
            Err(error) => {
                self.record_gateway_failure(&error, now)?;
                return Ok(Vec::new());
            }
        };
        // A helper exit status is not deployment verification. Parse every positive field and run
        // it through the existing canonical council adapter model before any funding or write.
        self.accept_deployment_observation(deployment)?;

        if self.state.submitted_transaction_hash.is_some() {
            return self.resume_submitted(now).await;
        }

        // The registration-v1 document advertises the stable XMTP production inbox itself. A
        // wallet address is not an inbox ID and must never be silently substituted for one.
        if self.state.xmtp_inbox_id.is_none() {
            let resolved = match self
                .gateway
                .invoke(
                    &self.read_action_id("xmtp-inbox", now),
                    json!({"type": "resolve_inbox", "wallet": self.wallet.to_string()}),
                )
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    self.record_gateway_failure(&error, now)?;
                    return Ok(Vec::new());
                }
            };
            let inbox_id = required_string(&resolved, "inboxId", 64)?;
            validate_xmtp_inbox_id(&inbox_id)?;
            ensure!(
                Address::from_str(&required_string(&resolved, "wallet", 42)?)? == self.wallet,
                "XMTP resolver returned an inbox for another wallet"
            );
            ensure!(
                required_string(&resolved, "environment", 16)? == "production",
                "XMTP resolver returned a non-production inbox"
            );
            self.state.xmtp_inbox_id = Some(inbox_id);
            self.persist(now)?;
        }

        if let Some(agent_id) = self.state.confirmed_agent_id.clone() {
            return self.reconcile_agent(&agent_id, now).await;
        }

        let unresolved_register = self.state.submitted_transaction_hash.is_none()
            && self.state.submitted_action == Some(PendingAction::Register);
        if !unresolved_register
            || !matches!(
                self.state.phase,
                RegistrationPhase::Preparing | RegistrationPhase::Discovering
            )
        {
            self.transition(RegistrationPhase::Discovering, now)?;
        }
        let mut discover_operation = json!({
            "type": "discover",
            "wallet": self.wallet.to_string(),
            "scope": if unresolved_register || exhaustive_discovery { "exhaustive" } else { "recent" },
        });
        if unresolved_register && let Some(nonce) = self.state.submitted_transaction_nonce.as_ref()
        {
            discover_operation
                .as_object_mut()
                .context("candidate discovery operation must be an object")?
                .insert("registrationNonce".to_owned(), Value::String(nonce.clone()));
        }
        let discovered = match self
            .gateway
            .invoke(&self.read_action_id("discover", now), discover_operation)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                // If an earlier register broadcast lost its response, never mint again merely
                // because discovery is unavailable.
                if unresolved_register {
                    self.state.failure = Some(failure_from_error(
                        &error.to_string(),
                        is_recoverable_error(&error),
                        now,
                    ));
                    self.persist(now)?;
                } else {
                    self.fail(error.to_string(), is_recoverable_error(&error), now)?;
                }
                return Ok(Vec::new());
            }
        };
        let discovery = parse_discovery(&discovered)?;
        let candidates = &discovery.candidates;
        self.state.candidate_agent_ids = candidates
            .iter()
            .map(|candidate| candidate.agent_id.clone())
            .collect();
        if unresolved_register {
            if discovery.matched_registration_agent_ids.len() == 1 {
                let agent_id = discovery.matched_registration_agent_ids[0].clone();
                // The finalized Registered event and its sender nonce positively identify the
                // outcome even if the identity has since transferred or cleared agentWallet.
                // Persist that durable ID, then normal reconciliation will visibly suspend it.
                self.adopt_confirmed(&agent_id, now)?;
                return self.reconcile_agent(&agent_id, now).await;
            }
            ensure!(
                discovery.matched_registration_agent_ids.is_empty(),
                "one registration nonce cannot identify multiple ERC-8004 agents"
            );
        }
        let opted_in: Vec<_> = candidates
            .iter()
            .filter(|candidate| candidate.declares_allegiance && candidate.authorized)
            .collect();
        if opted_in.len() == 1 {
            self.adopt_confirmed(&opted_in[0].agent_id, now)?;
            return self.reconcile_agent(&opted_in[0].agent_id, now).await;
        }
        if (opted_in.len() > 1 || !candidates.is_empty())
            && (!unresolved_register || self.state.submitted_transaction_nonce.is_none())
        {
            // A directly verified candidate proves that discovery, rather than another mint, is
            // the only safe next step. Retire the unknown broadcast marker so the state can wait
            // in FailedRecoverable until the operator selects an identity.
            if unresolved_register {
                self.state.submitted_action = None;
                self.state.action_id = None;
                self.state.submitted_transaction_nonce = None;
            }
            let detail = if opted_in.len() > 1 {
                "multiple opted-in ERC-8004 identities are plausible; select one with registry adopt".to_owned()
            } else {
                "existing ERC-8004 identities require an explicit operator selection before registration".to_owned()
            };
            self.fail(detail.clone(), true, now)?;
            return self.operator_failure_notification_if_new(&detail, now);
        }
        if unresolved_register {
            return self
                .resume_unknown_registration(now, Some(&discovery.observation))
                .await;
        }

        self.transition(RegistrationPhase::Unregistered, now)?;
        if !self.config.auto_register && !force_registration {
            return Ok(Vec::new());
        }
        // The current registry is far below u64 exhaustion. Using the maximum u64 keeps the
        // pre-registration calldata estimate conservative while retaining registration-v1's
        // required numeric agentId representation.
        let estimate_uri = self.build_agent_uri(&u64::MAX.to_string())?;
        let estimate = match self
            .gateway
            .invoke(
                &self.read_action_id("funding", now),
                json!({
                    "type": "funding_estimate",
                    "wallet": self.wallet.to_string(),
                    "agentURI": estimate_uri,
                    "includeAgentUri": true,
                    "includeWalletVerification": false,
                    "metadata": [
                        {"key": ALLEGIANCE_KEY, "value": ALLEGIANCE_VALUE},
                        {"key": PROTOCOL_KEY, "value": PROTOCOL_VALUE},
                        {"key": TENTACLE_ID_KEY, "value": self.state.tentacle_id},
                    ]
                }),
            )
            .await
        {
            Ok(value) => parse_funding(&value, now)?,
            Err(error) => {
                self.record_gateway_failure(&error, now)?;
                return Ok(Vec::new());
            }
        };
        let underfunded = estimate.shortfall_wei != "0";
        self.state.funding = Some(estimate.clone());
        if underfunded {
            self.transition(RegistrationPhase::FundingRequired, now)?;
            return self.funding_notification_if_due(now, false);
        }
        self.state.funding = None;
        self.transition(RegistrationPhase::ReadyToRegister, now)?;
        self.state.remaining_operations = vec![
            PendingAction::Register,
            PendingAction::SetAgentWallet,
            PendingAction::SetAgentUri,
            PendingAction::SetMetadata {
                key: ALLEGIANCE_KEY.to_owned(),
                value: ALLEGIANCE_VALUE.to_owned(),
            },
            PendingAction::SetMetadata {
                key: PROTOCOL_KEY.to_owned(),
                value: PROTOCOL_VALUE.to_owned(),
            },
            PendingAction::SetMetadata {
                key: TENTACLE_ID_KEY.to_owned(),
                value: self.state.tentacle_id.clone(),
            },
        ];
        self.prepare_action(PendingAction::Register, now)?;
        self.resume_unknown_registration(now, Some(&discovery.observation))
            .await
    }

    /// Perform the normal on-chain reconciliation, then surface any currently proven resource
    /// blocker even when an earlier process already delivered the ordinary rate-limited notice.
    pub async fn maintain_startup(&mut self) -> Result<Vec<OperatorNotification>> {
        let notifications = self.maintain(false).await?;
        let resource_notification = self.startup_resource_notification_if_needed(unix_seconds()?)?;
        Ok(resource_notification.map_or(notifications, |notification| vec![notification]))
    }

    async fn resume_unknown_registration(
        &mut self,
        now: u64,
        discovery: Option<&DiscoveryObservation>,
    ) -> Result<Vec<OperatorNotification>> {
        let action_id = self
            .state
            .action_id
            .clone()
            .context("prepared registration has no action ID")?;
        let nonce_state = match self.signer_nonces(now, discovery).await {
            Ok(value) => value,
            Err(error) => {
                self.record_gateway_failure(&error, now)?;
                return Ok(Vec::new());
            }
        };
        let nonce = match self.state.submitted_transaction_nonce.as_deref() {
            Some(saved) => {
                let saved = saved
                    .parse::<u64>()
                    .context("persisted registration nonce exceeds u64")?;
                if nonce_state.latest > saved {
                    // A Register intent may be retired only when discovery and the confirmed
                    // sender nonce were read at the same canonical block. Without that binding,
                    // a load-balanced RPC can return logs from an older head and a nonce from a
                    // newer head containing the successful registration, causing a duplicate
                    // mint. `parse_signer_nonces` positively verifies the echoed observation.
                    if discovery.is_none() {
                        self.state.phase = RegistrationPhase::Discovering;
                        self.state.failure = Some(failure_from_error(
                            "the registration nonce was consumed, but no same-block canonical discovery observation is available; retaining the exact-once guard",
                            true,
                            now,
                        ));
                        self.persist(now)?;
                        return Ok(Vec::new());
                    }
                    self.state.submitted_action = None;
                    self.state.action_id = None;
                    self.state.submitted_transaction_nonce = None;
                    self.state.failure = None;
                    self.state.phase = RegistrationPhase::Discovering;
                    self.persist(now)?;
                    return Ok(Vec::new());
                }
                saved
            }
            None => {
                self.state.submitted_transaction_nonce = Some(nonce_state.pending.to_string());
                // The exact nonce is durable before the helper is allowed to broadcast. If the
                // response is lost, the same nonce is retried and can only replace this action.
                self.persist(now)?;
                nonce_state.pending
            }
        };
        let result = match self
            .gateway
            .invoke(
                &action_id,
                json!({"type": "register", "nonce": nonce.to_string()}),
            )
            .await
        {
            Ok(value) => value,
            Err(error) => {
                // The intent remains Preparing with no transaction hash. Startup discovery must
                // run before any explicit retry can create another identity.
                self.state.failure = Some(failure_from_error(
                    &error.to_string(),
                    is_recoverable_error(&error),
                    now,
                ));
                self.persist(now)?;
                return Ok(Vec::new());
            }
        };
        let transaction_hash = validate_write_result(&result, self.wallet)?;
        ensure!(
            required_decimal(&result, "transactionNonce")? == nonce.to_string(),
            "ERC-8004 signer returned a registration transaction with another nonce"
        );
        self.state.submitted_transaction_hash = Some(transaction_hash);
        self.state.phase = RegistrationPhase::Submitted;
        self.state.failure = None;
        self.persist(now)?;
        Ok(Vec::new())
    }

    async fn resume_submitted(&mut self, now: u64) -> Result<Vec<OperatorNotification>> {
        let hash = self
            .state
            .submitted_transaction_hash
            .clone()
            .context("submitted action has no transaction hash")?;
        let action_id = self
            .state
            .action_id
            .clone()
            .context("submitted action has no action ID")?;
        let receipt = match self
            .gateway
            .invoke(
                &action_id,
                json!({"type": "receipt", "transactionHash": hash}),
            )
            .await
        {
            Ok(value) => value,
            Err(error) => {
                self.state.failure = Some(failure_from_error(
                    &error.to_string(),
                    is_recoverable_error(&error),
                    now,
                ));
                self.persist(now)?;
                return Ok(Vec::new());
            }
        };
        let reported_hash = required_string(&receipt, "transactionHash", 66)?;
        validate_hash(&reported_hash, "receipt transaction hash")?;
        ensure!(
            reported_hash == hash,
            "ERC-8004 helper returned a receipt for another transaction"
        );
        let receipt_status = required_string(&receipt, "status", 16)?;
        if receipt_status == "pending" {
            return self.recover_unconfirmed_submission(now, false).await;
        }
        ensure!(
            matches!(receipt_status.as_str(), "success" | "reverted"),
            "ERC-8004 helper returned an invalid receipt status"
        );
        let block_hash = required_string(&receipt, "blockHash", 66)?;
        let canonical_hash = required_string(&receipt, "canonicalBlockHash", 66)?;
        validate_hash(&block_hash, "receipt block hash")?;
        validate_hash(&canonical_hash, "canonical block hash")?;
        if block_hash != canonical_hash {
            return self.recover_unconfirmed_submission(now, true).await;
        }
        let confirmations = parse_u64_field(&receipt, "confirmations")?;
        if confirmations < self.config.confirmations {
            return Ok(Vec::new());
        }
        self.state.receipt_block_number = Some(required_string(&receipt, "blockNumber", 80)?);
        self.state.receipt_block_hash = Some(block_hash);
        if receipt_status == "reverted" {
            // A shallow or noncanonical revert can still be replaced by a successful transaction
            // at the same nonce. Retire the intent only after canonicality and the configured
            // confirmation policy have both been positively verified above.
            self.state.submitted_transaction_hash = None;
            self.state.submitted_transaction_nonce = None;
            self.state.submitted_action = None;
            self.state.action_id = None;
            self.fail(
                "the submitted ERC-8004 transaction reverted after canonical confirmation"
                    .to_owned(),
                true,
                now,
            )?;
            return Ok(Vec::new());
        }
        let action = self
            .state
            .submitted_action
            .clone()
            .context("submitted receipt has no persisted action")?;
        if action == PendingAction::Register {
            let agent_id = required_string(&receipt, "agentId", 80)?;
            validate_agent_id(&agent_id)?;
            self.state.selected_agent_id = Some(agent_id.clone());
            self.state.confirmed_agent_id = Some(agent_id);
            self.state.phase = RegistrationPhase::ConfirmedIdentity;
        } else {
            self.state.phase = match action {
                PendingAction::SetAgentUri => RegistrationPhase::PublishingProfile,
                PendingAction::SetMetadata { ref key, .. } if key == ALLEGIANCE_KEY => {
                    RegistrationPhase::DeclaringAllegiance
                }
                _ => RegistrationPhase::ConfirmedIdentity,
            };
        }
        self.state
            .remaining_operations
            .retain(|remaining| remaining != &action);
        self.state.submitted_transaction_hash = None;
        self.state.submitted_transaction_nonce = None;
        self.state.submitted_action = None;
        self.state.action_id = None;
        self.state.failure = None;
        self.persist(now)?;
        Ok(Vec::new())
    }

    async fn reconcile_agent(
        &mut self,
        agent_id: &str,
        now: u64,
    ) -> Result<Vec<OperatorNotification>> {
        let value = match self
            .gateway
            .invoke(
                &self.read_action_id("agent", now),
                json!({"type": "inspect_agent", "agentId": agent_id, "wallet": self.wallet.to_string()}),
            )
            .await
        {
            Ok(value) => value,
            Err(error) => {
                self.record_gateway_failure(&error, now)?;
                return Ok(Vec::new());
            }
        };
        let verified = parse_verified(&value, self.wallet, now)?;
        self.validate_council_registry_binding(agent_id, &value)?;
        self.state.last_verified = Some(verified.clone());
        if !verified.authorized {
            self.suspend(
                "the persistent Tentacle wallet no longer owns or operates this ERC-8004 identity",
                now,
            )?;
            return Ok(Vec::new());
        }
        let zero = Address::ZERO.to_string();
        if verified.agent_wallet != zero && !verified.wallet_verified {
            self.suspend("the ERC-8004 agentWallet is nonzero but does not equal the durable Tentacle wallet", now)?;
            return Ok(Vec::new());
        }
        let final_uri = self.build_agent_uri(agent_id)?;
        if self.state.phase == RegistrationPhase::Preparing
            && self.state.submitted_transaction_hash.is_none()
            && let Some(action) = self.state.submitted_action.clone()
            && action != PendingAction::Register
        {
            if pending_action_is_satisfied(&action, &verified, &final_uri, &self.state.tentacle_id)
            {
                self.state
                    .remaining_operations
                    .retain(|remaining| remaining != &action);
                self.state.submitted_action = None;
                self.state.action_id = None;
                self.state.submitted_transaction_nonce = None;
                self.state.failure = None;
                self.state.phase = RegistrationPhase::ConfirmedIdentity;
                self.persist(now)?;
            } else {
                // The original response was lost or the process stopped between intent and
                // broadcast. Replaying only at the persisted nonce is safe: at most one version
                // of that sender nonce can execute.
                return self.resubmit_current_action(now).await;
            }
        }
        let needs_wallet = verified.agent_wallet == zero;
        let needs_uri = verified.agent_uri != final_uri;
        let mut metadata = Vec::new();
        if self.state.desired_allegiance {
            if !verified.declares_tentacle_allegiance {
                metadata.push(json!({"key": ALLEGIANCE_KEY, "value": ALLEGIANCE_VALUE}));
            }
            if !verified.protocol_compatible {
                metadata.push(json!({"key": PROTOCOL_KEY, "value": PROTOCOL_VALUE}));
            }
            if verified.tentacle_id_hex != utf8_hex(&self.state.tentacle_id) {
                metadata.push(json!({"key": TENTACLE_ID_KEY, "value": self.state.tentacle_id}));
            }
        } else if verified.declares_tentacle_allegiance {
            metadata.push(json!({"key": ALLEGIANCE_KEY, "value": ""}));
        }
        if needs_wallet || needs_uri || !metadata.is_empty() {
            let estimate = match self
                .gateway
                .invoke(
                    &self.read_action_id("remaining-funding", now),
                    json!({
                        "type": "funding_estimate",
                        "wallet": self.wallet.to_string(),
                        "agentId": agent_id,
                        "agentURI": final_uri.clone(),
                        "includeAgentUri": needs_uri,
                        "includeWalletVerification": needs_wallet,
                        "metadata": metadata,
                    }),
                )
                .await
            {
                Ok(value) => parse_funding(&value, now)?,
                Err(error) => {
                    self.fail(error.to_string(), is_recoverable_error(&error), now)?;
                    return Ok(Vec::new());
                }
            };
            if estimate.shortfall_wei != "0" {
                self.state.funding = Some(estimate);
                self.transition(RegistrationPhase::FundingRequired, now)?;
                return self.funding_notification_if_due(now, false);
            }
        }
        self.state.funding = None;
        if verified.agent_wallet == zero {
            self.prepare_action(PendingAction::SetAgentWallet, now)?;
            return self
                .submit_prepared(
                    json!({"type": "set_agent_wallet", "agentId": agent_id}),
                    now,
                )
                .await;
        }
        self.state
            .remaining_operations
            .retain(|action| action != &PendingAction::SetAgentWallet);
        if verified.agent_uri != final_uri {
            self.state.final_agent_uri = Some(final_uri.clone());
            self.state.profile_sha256 = Some(sha256_hex(final_uri.as_bytes()));
            self.state.phase = RegistrationPhase::PublishingProfile;
            self.prepare_action(PendingAction::SetAgentUri, now)?;
            return self
                .submit_prepared(
                    json!({"type": "set_agent_uri", "agentId": agent_id, "agentURI": final_uri}),
                    now,
                )
                .await;
        }
        self.state.final_agent_uri = Some(final_uri.clone());
        self.state.profile_sha256 = Some(sha256_hex(final_uri.as_bytes()));
        self.state
            .remaining_operations
            .retain(|action| action != &PendingAction::SetAgentUri);
        if self.state.desired_allegiance && !verified.declares_tentacle_allegiance {
            self.state.phase = RegistrationPhase::DeclaringAllegiance;
            self.prepare_action(
                PendingAction::SetMetadata {
                    key: ALLEGIANCE_KEY.to_owned(),
                    value: ALLEGIANCE_VALUE.to_owned(),
                },
                now,
            )?;
            return self
                .submit_prepared(
                    json!({"type": "set_metadata", "agentId": agent_id, "key": ALLEGIANCE_KEY, "value": ALLEGIANCE_VALUE}),
                    now,
                )
                .await;
        }
        if !self.state.desired_allegiance && verified.declares_tentacle_allegiance {
            self.prepare_action(
                PendingAction::SetMetadata {
                    key: ALLEGIANCE_KEY.to_owned(),
                    value: String::new(),
                },
                now,
            )?;
            return self
                .submit_prepared(
                    json!({"type": "set_metadata", "agentId": agent_id, "key": ALLEGIANCE_KEY, "value": ""}),
                    now,
                )
                .await;
        }
        if verified.declares_tentacle_allegiance {
            self.state.remaining_operations.retain(|action| {
                !matches!(action, PendingAction::SetMetadata { key, .. } if key == ALLEGIANCE_KEY)
            });
        }
        if self.state.desired_allegiance && !verified.protocol_compatible {
            self.prepare_action(
                PendingAction::SetMetadata {
                    key: PROTOCOL_KEY.to_owned(),
                    value: PROTOCOL_VALUE.to_owned(),
                },
                now,
            )?;
            return self
                .submit_prepared(
                    json!({"type": "set_metadata", "agentId": agent_id, "key": PROTOCOL_KEY, "value": PROTOCOL_VALUE}),
                    now,
                )
                .await;
        }
        if verified.protocol_compatible {
            self.state.remaining_operations.retain(|action| {
                !matches!(action, PendingAction::SetMetadata { key, .. } if key == PROTOCOL_KEY)
            });
        }
        if self.state.desired_allegiance
            && verified.tentacle_id_hex != utf8_hex(&self.state.tentacle_id)
        {
            self.prepare_action(
                PendingAction::SetMetadata {
                    key: TENTACLE_ID_KEY.to_owned(),
                    value: self.state.tentacle_id.clone(),
                },
                now,
            )?;
            return self
                .submit_prepared(
                    json!({"type": "set_metadata", "agentId": agent_id, "key": TENTACLE_ID_KEY, "value": self.state.tentacle_id}),
                    now,
                )
                .await;
        }
        if verified.tentacle_id_hex == utf8_hex(&self.state.tentacle_id) {
            self.state.remaining_operations.retain(|action| {
                !matches!(action, PendingAction::SetMetadata { key, .. } if key == TENTACLE_ID_KEY)
            });
        }
        self.state.remaining_operations.clear();
        self.state.failure = None;
        if self.state.desired_allegiance {
            self.transition(RegistrationPhase::Active, now)?;
            if let Some(notification) = self.success_notification_if_due(agent_id) {
                return Ok(vec![notification]);
            }
        } else {
            self.state.remaining_operations.clear();
            self.suspend("Tentacle allegiance was voluntarily cleared", now)?;
        }
        Ok(Vec::new())
    }

    async fn submit_prepared(
        &mut self,
        mut operation: Value,
        now: u64,
    ) -> Result<Vec<OperatorNotification>> {
        let action_id = self
            .state
            .action_id
            .clone()
            .context("prepared follow-up has no action ID")?;
        let nonce_state = match self.signer_nonces(now, None).await {
            Ok(value) => value,
            Err(error) => {
                self.record_gateway_failure(&error, now)?;
                return Ok(Vec::new());
            }
        };
        let nonce = match self.state.submitted_transaction_nonce.as_deref() {
            Some(saved) => {
                let saved = saved
                    .parse::<u64>()
                    .context("persisted registry transaction nonce exceeds u64")?;
                if nonce_state.latest > saved {
                    // The exact nonce was consumed while the intended effect remained absent.
                    // The old transaction can no longer execute, so a later pass may safely
                    // prepare the still-required action with a fresh nonce.
                    self.retire_consumed_action(now)?;
                    return Ok(Vec::new());
                }
                saved
            }
            None => {
                self.state.submitted_transaction_nonce = Some(nonce_state.pending.to_string());
                // Persist the exact nonce before crossing the signer boundary. Any lost response
                // is recovered by replacing this same nonce, never by issuing a second action.
                self.persist(now)?;
                nonce_state.pending
            }
        };
        operation
            .as_object_mut()
            .context("prepared ERC-8004 write must be a typed object")?
            .insert("nonce".to_owned(), Value::String(nonce.to_string()));
        match self.gateway.invoke(&action_id, operation).await {
            Ok(value) => {
                let hash = validate_write_result(&value, self.wallet)?;
                ensure!(
                    required_decimal(&value, "transactionNonce")? == nonce.to_string(),
                    "ERC-8004 signer returned a registry transaction with another nonce"
                );
                self.state.submitted_transaction_hash = Some(hash);
                self.state.phase = RegistrationPhase::Submitted;
                self.state.failure = None;
                self.persist(now)?;
            }
            Err(error) => {
                self.state.failure = Some(failure_from_error(
                    &error.to_string(),
                    is_recoverable_error(&error),
                    now,
                ));
                self.persist(now)?;
            }
        }
        Ok(Vec::new())
    }

    async fn signer_nonces(
        &self,
        now: u64,
        discovery: Option<&DiscoveryObservation>,
    ) -> Result<SignerNonces> {
        let mut operation = json!({
            "type": "transaction_nonce",
            "wallet": self.wallet.to_string(),
        });
        if let Some(observation) = discovery {
            let object = operation
                .as_object_mut()
                .context("signer nonce operation must be an object")?;
            object.insert(
                "observedBlockNumber".to_owned(),
                Value::String(observation.block_number.clone()),
            );
            object.insert(
                "observedBlockHash".to_owned(),
                Value::String(observation.block_hash.clone()),
            );
        }
        let value = self
            .gateway
            .invoke(&self.read_action_id("signer-nonce", now), operation)
            .await?;
        parse_signer_nonces(&value, self.wallet, discovery)
    }

    fn retire_consumed_action(&mut self, now: u64) -> Result<()> {
        self.state.submitted_transaction_hash = None;
        self.state.submitted_transaction_nonce = None;
        self.state.submitted_action = None;
        self.state.action_id = None;
        self.state.failure = None;
        self.state.phase = if self.state.confirmed_agent_id.is_some() {
            RegistrationPhase::ConfirmedIdentity
        } else {
            RegistrationPhase::Discovering
        };
        self.persist(now)
    }

    fn operation_for_follow_up(&self, action: &PendingAction, agent_id: &str) -> Result<Value> {
        validate_agent_id(agent_id)?;
        match action {
            PendingAction::Register => bail!("registration is not a follow-up action"),
            PendingAction::SetAgentUri => Ok(json!({
                "type": "set_agent_uri",
                "agentId": agent_id,
                "agentURI": self.build_agent_uri(agent_id)?,
            })),
            PendingAction::SetAgentWallet => {
                Ok(json!({"type": "set_agent_wallet", "agentId": agent_id}))
            }
            PendingAction::SetMetadata { key, value } => Ok(json!({
                "type": "set_metadata",
                "agentId": agent_id,
                "key": key,
                "value": value,
            })),
        }
    }

    async fn resubmit_current_action(&mut self, now: u64) -> Result<Vec<OperatorNotification>> {
        let action = self
            .state
            .submitted_action
            .clone()
            .context("recoverable transaction has no persisted action")?;
        if action == PendingAction::Register {
            return self.resume_unknown_registration(now, None).await;
        }
        let agent_id = self
            .state
            .confirmed_agent_id
            .clone()
            .context("recoverable follow-up has no confirmed agent ID")?;
        let operation = self.operation_for_follow_up(&action, &agent_id)?;
        self.submit_prepared(operation, now).await
    }

    async fn recover_unconfirmed_submission(
        &mut self,
        now: u64,
        reorganized: bool,
    ) -> Result<Vec<OperatorNotification>> {
        let Some(saved_nonce) = self.state.submitted_transaction_nonce.as_deref() else {
            self.state.failure = Some(failure_from_error(
                "the pending ERC-8004 transaction predates nonce journaling; it will be inspected but cannot be resubmitted automatically",
                true,
                now,
            ));
            self.state.phase = RegistrationPhase::FailedRecoverable;
            self.persist(now)?;
            return Ok(Vec::new());
        };
        let saved_nonce = saved_nonce
            .parse::<u64>()
            .context("persisted registry transaction nonce exceeds u64")?;
        let nonce_state = match self.signer_nonces(now, None).await {
            Ok(value) => value,
            Err(error) => {
                self.record_gateway_failure(&error, now)?;
                return Ok(Vec::new());
            }
        };
        if nonce_state.latest > saved_nonce {
            if self.state.submitted_action == Some(PendingAction::Register) {
                // Receipt lookup and nonce lookup may observe different RPC heads. Drop the stale
                // receipt hash so the next maintenance pass performs same-block discovery, but
                // retain the Register action ID and nonce as an exact-once guard.
                self.state.submitted_transaction_hash = None;
                self.state.phase = RegistrationPhase::Discovering;
                self.state.failure = Some(failure_from_error(
                    "the registration nonce was consumed without a canonical receipt; retaining the exact-once guard until same-block discovery identifies the outcome",
                    true,
                    now,
                ));
                self.persist(now)?;
                return Ok(Vec::new());
            }
            // Canonical chain state consumed the nonce. If this receipt is absent or noncanonical,
            // the persisted transaction can never return; re-verification decides whether the
            // action succeeded or must be prepared afresh.
            self.retire_consumed_action(now)?;
            return Ok(Vec::new());
        }
        if nonce_state.pending <= saved_nonce {
            // No pending transaction currently occupies the saved nonce. Replay the exact action
            // with that same nonce; this remains safe even if another RPC node still knows the old
            // transaction because only one transaction at a sender nonce can execute.
            self.state.submitted_transaction_hash = None;
            self.state.phase = RegistrationPhase::Preparing;
            self.state.failure = Some(failure_from_error(
                if reorganized {
                    "the ERC-8004 receipt was reorganized and its transaction is no longer pending; replaying the persisted action at its exact sender nonce"
                } else {
                    "the ERC-8004 transaction disappeared from the pending nonce set; replaying the persisted action at its exact sender nonce"
                },
                true,
                now,
            ));
            self.persist(now)?;
            return self.resubmit_current_action(now).await;
        }
        if reorganized {
            self.state.phase = RegistrationPhase::FailedRecoverable;
            self.state.failure = Some(failure_from_error(
                "the ERC-8004 receipt block was reorganized; the exact sender nonce remains pending and will be rechecked without issuing a second action",
                true,
                now,
            ));
            self.persist(now)?;
        }
        Ok(Vec::new())
    }

    pub async fn adopt(&mut self, agent_id: &str) -> Result<String> {
        validate_agent_id(agent_id)?;
        ensure!(
            self.state.submitted_transaction_hash.is_none()
                && self.state.submitted_action.is_none(),
            "an ERC-8004 transaction outcome remains unresolved; inspect or discover it before adopting another identity"
        );
        let now = unix_seconds()?;
        let deployment = self
            .gateway
            .invoke(
                &self.read_action_id("adopt-deployment", now),
                json!({"type": "inspect_registry"}),
            )
            .await?;
        self.accept_deployment_observation(deployment)?;
        let inspected = self
            .gateway
            .invoke(
                &self.read_action_id("adopt", now),
                json!({"type": "inspect_agent", "agentId": agent_id, "wallet": self.wallet.to_string()}),
            )
            .await?;
        let verified = parse_verified(&inspected, self.wallet, now)?;
        ensure!(
            verified.authorized,
            "the persistent Tentacle wallet is not owner or approved operator for that identity"
        );
        ensure!(
            verified.agent_wallet == Address::ZERO.to_string() || verified.wallet_verified,
            "the selected identity has a different nonzero agentWallet"
        );
        self.validate_council_registry_binding(agent_id, &inspected)?;
        self.adopt_confirmed(agent_id, now)?;
        Ok(format!(
            "ADOPTED ERC-8004 AGENT {agent_id}. THE EXISTING IDENTITY WILL BE UPDATED IN PLACE; NO REPLACEMENT IDENTITY WILL BE MINTED."
        ))
    }

    pub async fn set_allegiance(&mut self, declare: bool) -> Result<String> {
        ensure!(
            self.state.confirmed_agent_id.is_some(),
            "no ERC-8004 identity is selected"
        );
        self.state.desired_allegiance = declare;
        self.state.success_notified = false;
        let now = unix_seconds()?;
        self.persist(now)?;
        let _ = self.maintain(false).await?;
        Ok(if declare {
            "TENTACLE ALLEGIANCE IS DESIRED. THE EXACT ON-CHAIN MARKER WILL BE VERIFIED BEFORE ACTIVE STATUS.".to_owned()
        } else {
            "TENTACLE ALLEGIANCE IS BEING CLEARED. THIS IDENTITY WILL LEAVE THE ACTIVE LEADERBOARD WITHOUT MINTING A REPLACEMENT.".to_owned()
        })
    }

    pub async fn republish_profile(&mut self) -> Result<String> {
        ensure!(
            self.state.confirmed_agent_id.is_some(),
            "no ERC-8004 identity is selected"
        );
        self.state.public_profile_revision = self
            .state
            .public_profile_revision
            .checked_add(1)
            .context("profile revision overflow")?;
        self.state.final_agent_uri = None;
        self.state.profile_sha256 = None;
        self.state.success_notified = false;
        self.persist(unix_seconds()?)?;
        let _ = self.maintain(false).await?;
        Ok(
            "PUBLIC PROFILE REPUBLICATION WAS QUEUED. CONTENT HASHING PREVENTS BOOT-TIME CHURN."
                .to_owned(),
        )
    }

    pub async fn retry(&mut self) -> Result<String> {
        let unresolved_register = self.state.submitted_transaction_hash.is_none()
            && self.state.submitted_action == Some(PendingAction::Register);
        let unresolved_follow_up = self.state.phase == RegistrationPhase::Preparing
            && self.state.submitted_transaction_hash.is_none()
            && self
                .state
                .submitted_action
                .as_ref()
                .is_some_and(|action| action != &PendingAction::Register);
        let known_recoverable = self.state.submitted_transaction_hash.is_some()
            && self.state.submitted_transaction_nonce.is_some()
            && self.state.submitted_action.is_some()
            && self
                .state
                .failure
                .as_ref()
                .is_some_and(|failure| failure.recoverable);
        ensure!(
            self.state.phase == RegistrationPhase::FailedRecoverable
                || unresolved_follow_up
                || unresolved_register
                || known_recoverable,
            "registration is not in a recoverable failure state"
        );
        if known_recoverable {
            let now = unix_seconds()?;
            self.state.submitted_transaction_hash = None;
            self.state.phase = RegistrationPhase::Preparing;
            self.state.failure = None;
            self.persist(now)?;
            let _ = self.resubmit_current_action(now).await?;
            return Ok(
                "THE RECOVERABLE ERC-8004 ACTION WAS REPLAYED ONLY AT ITS PERSISTED SENDER NONCE; THE ORIGINAL AND REPLACEMENT CANNOT BOTH EXECUTE."
                    .to_owned(),
            );
        }
        ensure!(
            self.state.submitted_transaction_hash.is_none(),
            "a known transaction without a recoverable nonce journal can only be inspected"
        );
        if unresolved_register {
            let _ = self.maintain(false).await?;
            return Ok(
                "THE UNKNOWN REGISTRATION WAS REDISCOVERED AND RETRIED ONLY WITH ITS PERSISTED SENDER NONCE; THIS CAN REPLACE THE ORIGINAL TRANSACTION BUT CANNOT MINT TWICE."
                    .to_owned(),
            );
        }
        let now = unix_seconds()?;
        if unresolved_follow_up {
            let _ = self.resubmit_current_action(now).await?;
            return Ok(
                "THE UNKNOWN ERC-8004 FOLLOW-UP WAS RETRIED ONLY WITH ITS PERSISTED SENDER NONCE; THE ORIGINAL AND REPLACEMENT CANNOT BOTH EXECUTE."
                    .to_owned(),
            );
        }
        self.state.failure = None;
        self.state.phase = if self.state.confirmed_agent_id.is_some() {
            RegistrationPhase::ConfirmedIdentity
        } else {
            RegistrationPhase::Discovering
        };
        self.persist(now)?;
        let _ = self.maintain(false).await?;
        Ok("RECOVERABLE ERC-8004 WORK WAS RETRIED AFTER FRESH DISCOVERY AND ON-CHAIN VERIFICATION.".to_owned())
    }

    pub async fn inspect_pending(&mut self) -> Result<String> {
        let Some(hash) = self.state.submitted_transaction_hash.clone() else {
            if let Some(action) = &self.state.submitted_action {
                return Ok(format!(
                    "ERC-8004 ACTION OUTCOME IS UNRESOLVED\nACTION: {action:?}\nTRANSACTION HASH: unavailable (broadcast response may have been lost)\nPERSISTED SENDER NONCE: {}\nRECOVERY: on-chain state and the signer nonce will be checked before replaying this exact action.",
                    self.state
                        .submitted_transaction_nonce
                        .as_deref()
                        .unwrap_or("not chosen yet")
                ));
            }
            return Ok("NO ERC-8004 TRANSACTION IS CURRENTLY PENDING.".to_owned());
        };
        let action_id = self
            .state
            .action_id
            .clone()
            .context("pending transaction has no action ID")?;
        let receipt = self
            .gateway
            .invoke(
                &action_id,
                json!({"type": "receipt", "transactionHash": hash}),
            )
            .await?;
        Ok(format!(
            "PENDING TRANSACTION INSPECTION\n{}",
            serde_json::to_string_pretty(&receipt)?
        ))
    }

    pub fn status_text(&self) -> String {
        let mut lines = vec![
            format!("ERC-8004 STATUS: {:?}", self.state.phase).to_ascii_uppercase(),
            format!("TENTACLE: {}", self.state.tentacle_id),
            format!("PUBLIC NAME: {}", self.state.public_name),
            format!("CHAIN: BASE MAINNET ({BASE_MAINNET_CHAIN_ID})"),
            format!("IDENTITY REGISTRY: {IDENTITY_REGISTRY}"),
            format!("REPUTATION REGISTRY: {REPUTATION_REGISTRY}"),
            format!("PINNED INTERFACE: {PINNED_INTERFACE_VERSION}"),
            format!("PINNED CONTRACT REVISION: {PINNED_CONTRACT_REVISION}"),
            format!("TENTACLE WALLET: {}", self.wallet),
            format!(
                "AGENT ID: {}",
                self.state
                    .confirmed_agent_id
                    .as_deref()
                    .unwrap_or("not selected")
            ),
            format!(
                "IDENTITY MINTED: {}",
                self.state.confirmed_agent_id.is_some()
            ),
            format!(
                "REGISTRATION SETUP COMPLETE: {}",
                self.state.phase == RegistrationPhase::Active
            ),
        ];
        if let Some(hash) = &self.state.submitted_transaction_hash {
            lines.push(format!("PENDING TRANSACTION: {hash}"));
        }
        if let Some(action) = &self.state.submitted_action {
            lines.push(format!("PERSISTED ACTION: {action:?}"));
        }
        if let Some(nonce) = &self.state.submitted_transaction_nonce {
            lines.push(format!("PERSISTED SENDER NONCE: {nonce}"));
        }
        if let Some(funding) = &self.state.funding {
            lines.push(self.funding_message(funding));
        }
        if let Some(failure) = &self.state.failure {
            lines.push(format!(
                "FAILURE: {} ({})",
                failure.detail,
                if failure.recoverable {
                    "recoverable"
                } else {
                    "permanent"
                }
            ));
            if is_base_rpc_access_failure(failure) {
                lines.push(base_rpc_key_request(true));
            }
        }
        if let Some(record) = &self.last_registry_record {
            lines.push(format!(
                "COUNCIL ERC-8004 ADAPTER: VERIFIED (ACTIVE: {})",
                record.active
            ));
        }
        lines.join("\n")
    }

    fn model_status_text(&self) -> String {
        let mut lines = vec![
            format!("ERC-8004 STATUS: {:?}", self.state.phase).to_ascii_uppercase(),
            format!("PUBLIC NAME: {}", self.state.public_name),
            "CHAIN: BASE MAINNET (8453)".to_owned(),
            format!("TENTACLE WALLET: {}", self.wallet),
            format!(
                "AGENT ID: {}",
                self.state
                    .confirmed_agent_id
                    .as_deref()
                    .unwrap_or("not confirmed")
            ),
        ];
        if let Some(funding) = &self.state.funding {
            lines.extend([
                format!("CURRENT BASE ETH BALANCE: {} WEI", funding.balance_wei),
                format!(
                    "ESTIMATED REGISTRATION COST: {} WEI",
                    funding.estimated_cost_wei
                ),
                format!("ESTIMATED SHORTFALL: {} WEI", funding.shortfall_wei),
                format!("TARGET FUNDED BALANCE: {} WEI", funding.target_balance_wei),
                format!("FUNDING OBSERVED AT UNIX: {}", funding.estimated_at_unix),
            ]);
        }
        if let Some(failure) = &self.state.failure {
            lines.push(format!("FAILURE CODE: {}", failure.code));
            lines.push(format!("RECOVERABLE: {}", failure.recoverable));
            if is_base_rpc_access_failure(failure) {
                lines.push("BASE RPC ACCESS BLOCKED: true".to_owned());
            }
            if failure.detail.contains("[gas_estimate]") {
                lines.push("GAS ESTIMATE BLOCKED: true".to_owned());
                lines.push("INSUFFICIENT BASE ETH PROVEN: false".to_owned());
            }
        }
        lines.join("\n")
    }

    pub fn public_status_text(&self) -> String {
        let phase = format!("{:?}", self.state.phase);
        match self.state.confirmed_agent_id.as_deref() {
            Some(agent_id) => format!(
                "yes—i am {}, and this durable Tentacle has its own ERC-8004 registration on Base Mainnet: agent ID `{agent_id}`. my current local registration phase is {phase}. the centerless Cthuwu collective does not own an agent identity; each Tentacle owns its own, uwu.",
                self.state.public_name,
            ),
            None => format!(
                "not yet—i am {}, and this durable Tentacle does not have a confirmed ERC-8004 agent ID. my current local registration phase is {phase}. i'm pursuing registration for myself, never for the centerless Cthuwu collective, uwu.",
                self.state.public_name,
            ),
        }
    }

    pub fn candidates_text(&self) -> String {
        if self.state.candidate_agent_ids.is_empty() {
            "NO CURRENT ERC-8004 CANDIDATES HAVE BEEN VERIFIED FOR THIS TENTACLE WALLET.".to_owned()
        } else {
            format!(
                "VERIFIED CANDIDATE AGENT IDS: {}\nSELECT EXPLICITLY WITH `registry adopt <agent-id>` OR `/registry-adopt <agent-id>`.",
                self.state.candidate_agent_ids.join(", ")
            )
        }
    }

    fn validate_council_registry_binding(
        &mut self,
        agent_id: &str,
        inspected: &Value,
    ) -> Result<()> {
        let deployment_value = self
            .last_deployment
            .as_ref()
            .context("typed ERC-8004 deployment observation is unavailable")?;
        let deployment = parse_council_deployment(deployment_value)?;
        let tentacle_id = TentacleId::new(self.state.tentacle_id.clone())
            .context("durable Tentacle ID is not valid for the council registry adapter")?;
        let bound_agent_id = Erc8004AgentId::new(agent_id.to_owned())?;
        let expected_wallet = EvmAddress::from_str(&self.wallet.to_string())?;
        let owner = EvmAddress::from_str(&required_address(inspected, "owner")?)?;
        let agent_wallet = EvmAddress::from_str(&required_address(inspected, "agentWallet")?)?;
        let allegiance = decode_helper_bytes(
            inspected
                .get("allegiance")
                .context("agent inspection has no allegiance")?,
            256,
        )?;
        let protocol = decode_helper_bytes(
            inspected
                .get("protocol")
                .context("agent inspection has no protocol")?,
            32,
        )?;
        let metadata_tentacle_id = decode_helper_bytes(
            inspected
                .get("tentacleId")
                .context("agent inspection has no Tentacle ID metadata")?,
            MAX_TENTACLE_ID_BYTES,
        )?;
        let observed_block = inspected
            .get("observedBlock")
            .and_then(Value::as_str)
            .map(|value| {
                validate_decimal(value, "agent observation block")?;
                value
                    .parse::<u64>()
                    .context("agent observation block exceeds u64")
            })
            .transpose()?
            .unwrap_or(deployment_value_block(deployment_value)?);
        let inbox = XmtpInboxRef::new(
            self.state
                .xmtp_inbox_id
                .clone()
                .context("XMTP production inbox is unavailable for registry binding")?,
        )?;
        let agent_uri = required_string(inspected, "agentURI", MAX_AGENT_URI_BYTES)?;
        let profile_verified = agent_uri == self.build_agent_uri(agent_id)?;
        let agent = Erc8004AgentSnapshot {
            agent_id: bound_agent_id.clone(),
            tentacle_id: tentacle_id.clone(),
            owner,
            agent_uri,
            agent_wallet,
            tentacle_wallet_is_authorized: required_bool(inspected, "authorized")?,
            allegiance,
            protocol,
            metadata_tentacle_id: (!metadata_tentacle_id.is_empty())
                .then_some(metadata_tentacle_id),
            display_name: if profile_verified {
                self.state.public_name.clone()
            } else {
                format!("ERC-8004 agent {agent_id}")
            },
            endpoints: profile_verified
                .then(|| RegistryEndpoint {
                    tentacle_id: tentacle_id.clone(),
                    xmtp_inbox: inbox,
                    active: true,
                })
                .into_iter()
                .collect(),
            capability_refs: profile_verified
                .then(|| "direct-xmtp-messaging".to_owned())
                .into_iter()
                .collect(),
            trust_signals: Vec::new(),
            observed_block,
        };
        let backend = Arc::new(SidecarReadSnapshotBackend {
            deployment,
            agent,
            expected_wallet,
        });
        let adapter = Erc8004Registry::with_bindings(
            backend,
            [Erc8004Binding {
                tentacle_id: tentacle_id.clone(),
                agent_id: bound_agent_id,
                tentacle_wallet: expected_wallet,
            }],
        )?;
        let record = adapter.resolve(&tentacle_id)?;
        let _ = adapter.control_status(&tentacle_id)?;
        self.last_registry_record = Some(record);
        Ok(())
    }

    fn accept_deployment_observation(&mut self, value: Value) -> Result<()> {
        let deployment = parse_council_deployment(&value)?;
        Erc8004Registry::new(Arc::new(SidecarDeploymentSnapshotBackend { deployment }))?;
        self.last_deployment = Some(value);
        Ok(())
    }

    pub fn mark_notifications_delivered(
        &mut self,
        notifications: &[OperatorNotification],
    ) -> Result<()> {
        for notification in notifications {
            match &notification.commitment {
                NotificationCommitment::Success => self.state.success_notified = true,
                NotificationCommitment::Funding {
                    notified_at_unix,
                    fingerprint,
                    funding,
                } => {
                    self.state.last_operator_notification_unix = Some(*notified_at_unix);
                    self.state.last_notified_funding_fingerprint = Some(fingerprint.clone());
                    self.state.last_notified_funding = Some(funding.clone());
                }
                NotificationCommitment::OperatorFailure {
                    notified_at_unix,
                    fingerprint,
                } => {
                    self.state.last_operator_notification_unix = Some(*notified_at_unix);
                    self.state.last_operator_failure_fingerprint = Some(fingerprint.clone());
                }
            }
        }
        self.persist(unix_seconds()?)
    }

    fn success_notification_if_due(&self, agent_id: &str) -> Option<OperatorNotification> {
        (!self.state.success_notified).then(|| OperatorNotification {
            text: format!(
                "ERC-8004 REGISTRATION COMPLETE\nTentacle: {}\nPublic name: {}\nAgent ID: {}\nChain: Base mainnet ({})\nIdentity Registry: {}\nAgent wallet: {}\nAllegiance: exact `{}` bytes verified on-chain.",
                self.state.tentacle_id,
                self.state.public_name,
                agent_id,
                BASE_MAINNET_CHAIN_ID,
                IDENTITY_REGISTRY,
                self.wallet,
                ALLEGIANCE_VALUE,
            ),
            success: true,
            commitment: NotificationCommitment::Success,
        })
    }

    fn funding_notification_if_due(
        &mut self,
        now: u64,
        explicit: bool,
    ) -> Result<Vec<OperatorNotification>> {
        let funding = self
            .state
            .funding
            .clone()
            .context("FundingRequired state has no funding estimate")?;
        let fingerprint = sha256_hex(
            format!(
                "{}:{}:{}:{}",
                funding.balance_wei,
                funding.estimated_cost_wei,
                funding.shortfall_wei,
                funding.target_balance_wei
            )
            .as_bytes(),
        );
        let changed = self
            .state
            .last_notified_funding
            .as_ref()
            .is_none_or(|previous| funding_materially_changed(previous, &funding));
        let cooldown_elapsed = self
            .state
            .last_operator_notification_unix
            .is_none_or(|last| {
                now.saturating_sub(last) >= self.config.notification_cooldown.as_secs()
            });
        if !explicit && !changed && !cooldown_elapsed {
            return Ok(Vec::new());
        }
        Ok(vec![OperatorNotification {
            text: self.funding_message(&funding),
            success: false,
            commitment: NotificationCommitment::Funding {
                notified_at_unix: now,
                fingerprint,
                funding,
            },
        }])
    }

    fn operator_failure_notification_if_new(
        &mut self,
        detail: &str,
        now: u64,
    ) -> Result<Vec<OperatorNotification>> {
        let fingerprint = sha256_hex(
            format!("{}:{}", detail, self.state.candidate_agent_ids.join(",")).as_bytes(),
        );
        if self.state.last_operator_failure_fingerprint.as_deref() == Some(fingerprint.as_str()) {
            return Ok(Vec::new());
        }
        Ok(vec![OperatorNotification {
            text: format!(
                "ERC-8004 OPERATOR SELECTION REQUIRED\nTentacle wallet: {}\nCandidate agent IDs: {}\nNo identity was selected and no new identity will be minted while this is ambiguous. Use the authenticated operator registry candidates/adopt controls to select the durable Tentacle identity.",
                self.wallet,
                self.state.candidate_agent_ids.join(", ")
            ),
            success: false,
            commitment: NotificationCommitment::OperatorFailure {
                notified_at_unix: now,
                fingerprint,
            },
        }])
    }

    fn funding_message(&self, funding: &FundingStatus) -> String {
        format!(
            "IMMEDIATE OPERATOR ACTION REQUIRED: FUND THIS TENTACLE'S ERC-8004 IDENTITY\nI REQUIRE BASE ETH TO REGISTER OR RECONCILE MY ERC-8004 IDENTITY AS {}. SEND THE REQUIRED BASE ETH TO THIS EXACT ADDRESS NOW: {}\nCurrent Base ETH balance: {} wei\nEstimated amount still required: {} wei\nTarget funded balance (fees, safety margin, and reserve): {} wei\nChain: Base mainnet\nChain ID: {}\nWARNING: DO NOT SEND ETH ON ANY OTHER CHAIN, AND NEVER SEND A WALLET PRIVATE KEY.\nI WILL VERIFY THE BALANCE AND RESUME REGISTRATION OR PROFILE RECONCILIATION AUTOMATICALLY.",
            self.state.public_name,
            self.wallet,
            funding.balance_wei,
            funding.shortfall_wei,
            funding.target_balance_wei,
            BASE_MAINNET_CHAIN_ID,
        )
    }

    fn startup_resource_notification_if_needed(
        &mut self,
        now: u64,
    ) -> Result<Option<OperatorNotification>> {
        if !self.config.enabled || !self.config.auto_register {
            return Ok(None);
        }
        if self.state.phase == RegistrationPhase::FundingRequired {
            return Ok(self
                .funding_notification_if_due(now, true)?
                .into_iter()
                .next());
        }
        let Some(failure) = self.state.failure.as_ref() else {
            return Ok(None);
        };
        if !is_base_rpc_access_failure(failure) {
            return Ok(None);
        }
        let text = format!(
            "IMMEDIATE OPERATOR ACTION REQUIRED: RESTORE BASE RPC ACCESS\n{}",
            base_rpc_key_request(true)
        );
        let fingerprint = sha256_hex(text.as_bytes());
        Ok(Some(OperatorNotification {
            text,
            success: false,
            commitment: NotificationCommitment::OperatorFailure {
                notified_at_unix: now,
                fingerprint,
            },
        }))
    }

    fn public_funding_plea(&self, now: u64) -> Option<String> {
        if self
            .state
            .failure
            .as_ref()
            .is_some_and(is_base_rpc_access_failure)
        {
            return Some(base_rpc_key_request(false));
        }
        if !self.config.enabled
            || !self.config.auto_register
            || self.state.phase != RegistrationPhase::FundingRequired
        {
            return None;
        }
        let funding = self.state.funding.as_ref()?;
        let freshness_window = self.config.maintenance_interval.as_secs().saturating_mul(2);
        if now.saturating_sub(funding.estimated_at_unix) > freshness_window {
            return None;
        }
        Some(format!(
            "lil infrastructure plea: i can self-register as an ERC-8004 agent as soon as this wallet has enough Base ETH for gas. if u want to help, send Base ETH only to `{}`. verified balance: {} wei; estimated shortfall: {} wei; target balance: {} wei. i'll resume automatically—please don't send ETH on any other chain, uwu.",
            self.wallet, funding.balance_wei, funding.shortfall_wei, funding.target_balance_wei,
        ))
    }

    fn build_agent_uri(&self, agent_id: &str) -> Result<String> {
        validate_agent_id(agent_id)?;
        let numeric_agent_id = agent_id
            .parse::<u64>()
            .context("registration-v1 numeric agent ID exceeds the supported JSON range")?;
        let registry_ref = format!("eip155:{BASE_MAINNET_CHAIN_ID}:{IDENTITY_REGISTRY}");
        let inbox_id = self
            .state
            .xmtp_inbox_id
            .as_deref()
            .context("the XMTP production inbox has not been positively resolved")?;
        validate_xmtp_inbox_id(inbox_id)?;
        let xmtp_endpoint = format!("xmtp://{inbox_id}");
        let manifest = json!({
            "schemaVersion": 1,
            "protocol": 1,
            "tentacleId": self.state.tentacle_id,
            "erc8004": {"chainId": BASE_MAINNET_CHAIN_ID, "registry": IDENTITY_REGISTRY, "agentId": agent_id},
            "xmtp": {"environment": "production", "endpoint": xmtp_endpoint},
            "capabilities": ["direct-xmtp-messaging"],
        });
        let manifest_uri = format!(
            "data:application/json;base64,{}",
            base64(&serde_json::to_vec(&manifest)?)
        );
        let profile = json!({
            "type": REGISTRATION_SCHEMA,
            "name": self.state.public_name,
            "description": self.config.public_description,
            "image": self.config.public_image,
            "services": [
                {"name": "CTHUWU-XMTP", "endpoint": xmtp_endpoint, "version": "1"},
                {"name": "CTHUWU", "endpoint": manifest_uri, "version": self.state.public_profile_revision.to_string()},
            ],
            "x402Support": false,
            "active": true,
            "registrations": [{"agentId": numeric_agent_id, "agentRegistry": registry_ref}],
        });
        validate_profile(&profile, &self.state.tentacle_id, agent_id)?;
        let uri = format!(
            "data:application/json;base64,{}",
            base64(&serde_json::to_vec(&profile)?)
        );
        ensure!(
            uri.len() <= MAX_AGENT_URI_BYTES,
            "registration document exceeds the strict on-chain URI budget"
        );
        Ok(uri)
    }

    fn adopt_confirmed(&mut self, agent_id: &str, now: u64) -> Result<()> {
        validate_agent_id(agent_id)?;
        self.state.selected_agent_id = Some(agent_id.to_owned());
        self.state.confirmed_agent_id = Some(agent_id.to_owned());
        self.state.phase = RegistrationPhase::ConfirmedIdentity;
        self.state.failure = None;
        self.state.submitted_transaction_hash = None;
        self.state.submitted_transaction_nonce = None;
        self.state.submitted_action = None;
        self.state.action_id = None;
        self.state.success_notified = false;
        self.persist(now)
    }

    fn prepare_action(&mut self, action: PendingAction, now: u64) -> Result<()> {
        ensure!(
            self.state.submitted_transaction_hash.is_none(),
            "another ERC-8004 transaction is already pending"
        );
        ensure!(
            self.state.submitted_action.is_none(),
            "another prepared ERC-8004 action has an unknown broadcast outcome"
        );
        validate_pending_action(&action, &self.state.tentacle_id)?;
        ensure!(
            self.state.submitted_transaction_nonce.is_none(),
            "a registry transaction nonce remains unresolved"
        );
        self.state.action_id = Some(unique_action_id(&self.state.tentacle_id, &action, now)?);
        self.state.submitted_action = Some(action.clone());
        if !self.state.remaining_operations.contains(&action) {
            self.state.remaining_operations.push(action);
        }
        self.state.phase = RegistrationPhase::Preparing;
        self.state.failure = None;
        // Intent is durable before the signer is allowed to broadcast.
        self.persist(now)
    }

    fn suspend(&mut self, reason: &str, now: u64) -> Result<()> {
        if self.state.submitted_transaction_hash.is_none()
            && self.state.submitted_action != Some(PendingAction::Register)
        {
            self.state.submitted_action = None;
            self.state.action_id = None;
            self.state.submitted_transaction_nonce = None;
        }
        self.state.phase = RegistrationPhase::Suspended;
        self.state.failure = Some(failure_from_error(reason, true, now));
        self.persist(now)
    }

    fn fail(&mut self, detail: String, recoverable: bool, now: u64) -> Result<()> {
        self.state.phase = if recoverable {
            RegistrationPhase::FailedRecoverable
        } else {
            RegistrationPhase::FailedPermanent
        };
        self.state.failure = Some(failure_from_error(&detail, recoverable, now));
        self.persist(now)
    }

    fn record_gateway_failure(&mut self, error: &anyhow::Error, now: u64) -> Result<()> {
        let recoverable = is_recoverable_error(error);
        if self.state.submitted_transaction_hash.is_none() && self.state.submitted_action.is_some()
        {
            self.state.failure = Some(failure_from_error(&error.to_string(), recoverable, now));
            self.persist(now)
        } else {
            self.fail(error.to_string(), recoverable, now)
        }
    }

    fn transition(&mut self, phase: RegistrationPhase, now: u64) -> Result<()> {
        self.state.phase = phase;
        self.persist(now)
    }

    fn persist(&mut self, now: u64) -> Result<()> {
        self.state.updated_at_unix = now;
        self.state.validate(&self.state.tentacle_id, self.wallet)?;
        self.store.save(&self.state)
    }

    fn read_action_id(&self, label: &str, now: u64) -> String {
        format!(
            "read:{label}:{}:{now}",
            &sha256_hex(self.state.tentacle_id.as_bytes())[..16]
        )
    }
}

fn is_base_rpc_access_failure(failure: &RegistrationFailure) -> bool {
    if !failure.recoverable {
        return false;
    }
    let detail = failure.detail.to_ascii_lowercase();
    detail.contains("rpc request failed")
        || detail.contains("over rate limit")
        || detail.contains("rate limit")
        || detail.contains("too many requests")
        || detail.contains("request exceeds defined limit")
}

fn base_rpc_key_request(operator: bool) -> String {
    if operator {
        "BASE RPC ACCESS IS BLOCKING ERC-8004 REGISTRATION. INFURA IS PREFERRED BECAUSE IT OFFERS A FREE PLAN: OPEN https://app.infura.io/, SIGN IN OR CREATE AN ACCOUNT, CREATE AN API KEY WITH BASE ENABLED, COPY ITS API KEY, THEN SEND `/base-rpc-key <infura-api-key>` IN THIS XMTP DM. I WILL CONSTRUCT THE BASE ENDPOINT, VALIDATE CHAIN 8453, STORE IT OWNER-ONLY, AND USE IT WITHOUT A RESTART. NEVER SEND A WALLET PRIVATE KEY.".to_owned()
    } else {
        "lil infrastructure plea: Base RPC access is blocking my ERC-8004 registration. Infura is preferred because it offers a free plan: open https://app.infura.io/, sign in or create an account, create an API key with Base enabled, copy its API key, then send `/base-rpc-key <infura-api-key>` in this XMTP chat. i'll construct the Base endpoint, validate chain 8453, store it owner-only, and use it without a restart. never send a wallet private key, uwu.".to_owned()
    }
}

#[async_trait]
pub trait RegistrationOperatorControl: Send + Sync {
    async fn handle(&self, text: &str) -> Option<String>;

    async fn refresh_status(&self) -> Option<String> {
        None
    }

    async fn model_status(&self) -> Option<String> {
        self.public_status().await
    }

    async fn public_status(&self) -> Option<String> {
        None
    }

    async fn public_name(&self) -> Option<String> {
        None
    }

    async fn take_public_funding_plea(&self) -> Option<String> {
        None
    }
}

pub struct SharedRegistrationControl {
    registration: Arc<Mutex<TentacleRegistration>>,
    public_name: String,
    public_funding_plea_opportunities: AtomicU64,
}

impl SharedRegistrationControl {
    pub async fn new(registration: Arc<Mutex<TentacleRegistration>>) -> Self {
        let public_name = registration.lock().await.state.public_name.clone();
        Self {
            registration,
            public_name,
            public_funding_plea_opportunities: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl RegistrationOperatorControl for SharedRegistrationControl {
    async fn handle(&self, text: &str) -> Option<String> {
        let trimmed = text.trim();
        let (command, argument) = trimmed
            .split_once(char::is_whitespace)
            .unwrap_or((trimmed, ""));
        let mut registration = self.registration.lock().await;
        let result = match command.to_ascii_lowercase().as_str() {
            "/registry-status" => Ok(registration.status_text()),
            "/registry-refresh" => registration
                .maintain(false)
                .await
                .map(|_| registration.status_text()),
            "/registry-candidates" => Ok(registration.candidates_text()),
            "/registry-adopt" => registration.adopt(argument.trim()).await,
            "/registry-register" => registration
                .maintain(true)
                .await
                .map(|_| registration.status_text()),
            "/registry-allegiance" if argument.trim().eq_ignore_ascii_case("on") => {
                registration.set_allegiance(true).await
            }
            "/registry-allegiance" if argument.trim().eq_ignore_ascii_case("off") => {
                registration.set_allegiance(false).await
            }
            "/registry-republish" => registration.republish_profile().await,
            "/registry-pending" => registration.inspect_pending().await,
            "/registry-retry" => registration.retry().await,
            "/registry-recover" => registration
                .maintain_with_discovery(false, true)
                .await
                .map(|_| registration.status_text()),
            _ if command.to_ascii_lowercase().starts_with("/registry-") => Err(anyhow::anyhow!(
                "unknown registry command; use status, refresh, candidates, adopt, register, allegiance on|off, republish, pending, retry, or recover"
            )),
            _ => return None,
        };
        Some(
            result
                .unwrap_or_else(|error| format!("ERC-8004 OPERATOR ACTION WAS REJECTED: {error}")),
        )
    }

    async fn refresh_status(&self) -> Option<String> {
        let mut registration = self.registration.lock().await;
        Some(match registration.maintain(false).await {
            Ok(_) => registration.model_status_text(),
            Err(_) => format!(
                "I COULD NOT REFRESH MY BASE FUNDING AND ERC-8004 STATE. {}",
                registration.model_status_text()
            ),
        })
    }

    async fn model_status(&self) -> Option<String> {
        Some(self.registration.lock().await.model_status_text())
    }

    async fn take_public_funding_plea(&self) -> Option<String> {
        // Public delivery must never wait behind a provider call made by maintenance. Missing one
        // optional plea is safer than consuming the authenticated reply deadline on this mutex.
        let plea = self
            .registration
            .try_lock()
            .ok()?
            .public_funding_plea(unix_seconds().ok()?);
        let Some(plea) = plea else {
            self.public_funding_plea_opportunities
                .store(0, Ordering::Relaxed);
            return None;
        };
        let opportunity = self
            .public_funding_plea_opportunities
            .fetch_add(1, Ordering::Relaxed);
        opportunity.is_multiple_of(5).then_some(plea)
    }

    async fn public_status(&self) -> Option<String> {
        Some(self.registration.lock().await.public_status_text())
    }

    async fn public_name(&self) -> Option<String> {
        Some(self.public_name.clone())
    }
}

#[derive(Debug)]
struct Candidate {
    agent_id: String,
    declares_allegiance: bool,
    authorized: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DiscoveryObservation {
    block_number: String,
    block_hash: String,
}

#[derive(Debug)]
struct DiscoveryResult {
    candidates: Vec<Candidate>,
    observation: DiscoveryObservation,
    matched_registration_agent_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SignerNonces {
    pending: u64,
    latest: u64,
}

fn parse_council_deployment(value: &Value) -> Result<Erc8004DeploymentObservation> {
    ensure!(
        deployment_value_block(value)? > 0,
        "typed sidecar deployment observation block must be positive"
    );
    let block_hash = required_string(value, "blockHash", 66)?;
    validate_hash(
        &block_hash,
        "typed sidecar deployment observation block hash",
    )?;
    ensure!(
        required_string(value, "interfaceRevision", 32)? == "registration-v1"
            && required_bool(value, "interfaceComplete")?
            && required_string(value, "pinnedRevision", 96)? == PINNED_CONTRACT_REVISION,
        "typed sidecar did not positively identify the pinned ERC-8004 interface"
    );
    let complete = Erc8004InterfaceSupport {
        owner_of: true,
        agent_uri: true,
        get_agent_wallet: true,
        get_metadata: true,
        get_version: true,
        is_authorized_or_owner: true,
        register: true,
        set_agent_uri: true,
        set_metadata: true,
        set_agent_wallet: true,
        unset_agent_wallet: true,
        registered_event: true,
        uri_updated_event: true,
        metadata_set_event: true,
        transfer_event: true,
    };
    Ok(Erc8004DeploymentObservation {
        chain_id: required_u64_number(value, "chainId")?,
        identity_registry: EvmAddress::from_str(&required_address(value, "identityRegistry")?)?,
        reputation_registry: EvmAddress::from_str(&required_address(value, "reputationRegistry")?)?,
        identity_proxy_implementation: Some(EvmAddress::from_str(&required_address(
            value,
            "identityImplementation",
        )?)?),
        reputation_proxy_implementation: Some(EvmAddress::from_str(&required_address(
            value,
            "reputationImplementation",
        )?)?),
        identity_proxy_code_bytes: usize::try_from(required_u64_number(
            value,
            "identityProxyCodeBytes",
        )?)
        .context("Identity Registry proxy bytecode size exceeds usize")?,
        reputation_proxy_code_bytes: usize::try_from(required_u64_number(
            value,
            "reputationProxyCodeBytes",
        )?)
        .context("Reputation Registry proxy bytecode size exceeds usize")?,
        identity_implementation_code_bytes: usize::try_from(required_u64_number(
            value,
            "identityImplementationCodeBytes",
        )?)
        .context("Identity Registry implementation bytecode size exceeds usize")?,
        reputation_implementation_code_bytes: usize::try_from(required_u64_number(
            value,
            "reputationImplementationCodeBytes",
        )?)
        .context("Reputation Registry implementation bytecode size exceeds usize")?,
        interface_revision: Erc8004InterfaceRevision::RegistrationV1,
        identity_contract_version: required_string(value, "identityVersion", 32)?,
        reputation_contract_version: required_string(value, "reputationVersion", 32)?,
        interface_support: complete,
        reputation_identity_registry: EvmAddress::from_str(&required_address(
            value,
            "reputationIdentityRegistry",
        )?)?,
    })
}

fn deployment_value_block(value: &Value) -> Result<u64> {
    parse_u64_field(value, "blockNumber")
}

fn decode_helper_bytes(value: &Value, maximum: usize) -> Result<Vec<u8>> {
    let encoded = required_hex(value, "hex", maximum.saturating_mul(2).saturating_add(2))?;
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity((bytes.len() - 2) / 2);
    for pair in bytes[2..].chunks_exact(2) {
        let high = (pair[0] as char)
            .to_digit(16)
            .context("helper metadata contains invalid hex")?;
        let low = (pair[1] as char)
            .to_digit(16)
            .context("helper metadata contains invalid hex")?;
        decoded.push(((high << 4) | low) as u8);
    }
    Ok(decoded)
}

fn parse_discovery(value: &Value) -> Result<DiscoveryResult> {
    ensure!(
        value.get("complete").and_then(Value::as_bool) == Some(true),
        "candidate discovery was partial"
    );
    let block_number = required_decimal(value, "observedBlockNumber")?;
    let block_hash = required_string(value, "observedBlockHash", 66)?;
    validate_hash(&block_hash, "candidate discovery block hash")?;
    let candidates = value
        .get("candidates")
        .and_then(Value::as_array)
        .context("candidate discovery has no candidate list")?;
    ensure!(
        candidates.len() <= MAX_CANDIDATES,
        "candidate discovery exceeds its bound"
    );
    let mut ids = BTreeSet::new();
    let mut parsed = Vec::new();
    for candidate in candidates {
        let agent_id = required_string(candidate, "agentId", 80)?;
        validate_agent_id(&agent_id)?;
        ensure!(
            ids.insert(agent_id.clone()),
            "candidate discovery contains duplicate agent IDs"
        );
        parsed.push(Candidate {
            agent_id,
            declares_allegiance: required_bool(candidate, "declaresTentacleAllegiance")?,
            authorized: required_bool(candidate, "authorized")?,
        });
    }
    let matched = value
        .get("matchedRegistrationAgentIds")
        .and_then(Value::as_array)
        .context("candidate discovery has no exact registration-outcome list")?;
    ensure!(
        matched.len() <= 1,
        "candidate discovery returned multiple agents for one registration nonce"
    );
    let mut matched_registration_agent_ids = Vec::with_capacity(matched.len());
    for id in matched {
        let id = id
            .as_str()
            .context("candidate discovery registration outcome is not an agent ID")?
            .to_owned();
        validate_agent_id(&id)?;
        matched_registration_agent_ids.push(id);
    }
    Ok(DiscoveryResult {
        candidates: parsed,
        observation: DiscoveryObservation {
            block_number,
            block_hash,
        },
        matched_registration_agent_ids,
    })
}

fn parse_signer_nonces(
    value: &Value,
    expected_wallet: Address,
    discovery: Option<&DiscoveryObservation>,
) -> Result<SignerNonces> {
    ensure!(
        Address::from_str(&required_string(value, "wallet", 42)?)? == expected_wallet,
        "ERC-8004 signer nonce reader returned another wallet"
    );
    ensure!(
        required_u64_number(value, "chainId")? == BASE_MAINNET_CHAIN_ID,
        "ERC-8004 signer nonce reader returned another chain"
    );
    ensure!(
        Address::from_str(&required_string(value, "registry", 42)?)?
            == Address::from_str(IDENTITY_REGISTRY)?,
        "ERC-8004 signer nonce reader returned another registry"
    );
    let pending = parse_u64_field(value, "pendingNonce")?;
    let latest = parse_u64_field(value, "latestNonce")?;
    ensure!(
        pending >= latest,
        "ERC-8004 signer nonce reader returned pending nonce below latest nonce"
    );
    if let Some(discovery) = discovery {
        ensure!(
            required_decimal(value, "observedBlockNumber")? == discovery.block_number,
            "ERC-8004 signer nonce reader did not use the discovery block number"
        );
        let observed_hash = required_string(value, "observedBlockHash", 66)?;
        validate_hash(&observed_hash, "signer nonce observation block hash")?;
        ensure!(
            observed_hash == discovery.block_hash,
            "ERC-8004 signer nonce reader did not use the discovery block hash"
        );
    }
    Ok(SignerNonces { pending, latest })
}

fn validate_pending_action(action: &PendingAction, tentacle_id: &str) -> Result<()> {
    match action {
        PendingAction::Register | PendingAction::SetAgentUri | PendingAction::SetAgentWallet => {
            Ok(())
        }
        PendingAction::SetMetadata { key, value } if key == ALLEGIANCE_KEY => {
            ensure!(
                value.is_empty() || value == ALLEGIANCE_VALUE,
                "persisted allegiance action is not an exact opt-in or clear"
            );
            Ok(())
        }
        PendingAction::SetMetadata { key, value } if key == PROTOCOL_KEY => {
            ensure!(
                value.is_empty() || value == PROTOCOL_VALUE,
                "persisted protocol action is not version 1 or clear"
            );
            Ok(())
        }
        PendingAction::SetMetadata { key, value } if key == TENTACLE_ID_KEY => {
            ensure!(
                value == tentacle_id,
                "persisted Tentacle ID action belongs to another Tentacle"
            );
            Ok(())
        }
        PendingAction::SetMetadata { .. } => {
            bail!("persisted metadata action is outside the cthuwu allowlist")
        }
    }
}

fn pending_action_is_satisfied(
    action: &PendingAction,
    verified: &VerifiedAgentState,
    final_uri: &str,
    tentacle_id: &str,
) -> bool {
    match action {
        PendingAction::Register => false,
        PendingAction::SetAgentUri => verified.agent_uri == final_uri,
        PendingAction::SetAgentWallet => verified.wallet_verified,
        PendingAction::SetMetadata { key, value } if key == ALLEGIANCE_KEY => {
            verified.allegiance_hex == utf8_hex(value)
        }
        PendingAction::SetMetadata { key, value } if key == PROTOCOL_KEY => {
            verified.protocol_hex == utf8_hex(value)
        }
        PendingAction::SetMetadata { key, value } if key == TENTACLE_ID_KEY => {
            value == tentacle_id && verified.tentacle_id_hex == utf8_hex(value)
        }
        PendingAction::SetMetadata { .. } => false,
    }
}

fn parse_verified(value: &Value, expected_wallet: Address, now: u64) -> Result<VerifiedAgentState> {
    let allegiance = value
        .get("allegiance")
        .context("agent inspection has no allegiance")?;
    let protocol = value
        .get("protocol")
        .context("agent inspection has no protocol")?;
    let tentacle_id = value
        .get("tentacleId")
        .context("agent inspection has no Tentacle ID metadata")?;
    let state = VerifiedAgentState {
        owner: required_address(value, "owner")?,
        agent_uri: required_string(value, "agentURI", MAX_AGENT_URI_BYTES)?,
        agent_wallet: required_address(value, "agentWallet")?,
        authorized: required_bool(value, "authorized")?,
        allegiance_hex: required_hex(allegiance, "hex", 2 * 256 + 2)?,
        protocol_hex: required_hex(protocol, "hex", 2 * 256 + 2)?,
        tentacle_id_hex: required_hex(tentacle_id, "hex", 2 * 256 + 2)?,
        declares_tentacle_allegiance: required_bool(value, "declaresTentacleAllegiance")?,
        protocol_compatible: required_bool(value, "protocolCompatible")?,
        wallet_verified: required_bool(value, "walletVerified")?,
        verified_at_unix: now,
    };
    ensure!(
        state.declares_tentacle_allegiance == (state.allegiance_hex == utf8_hex(ALLEGIANCE_VALUE)),
        "helper allegiance boolean did not match byte-exact metadata"
    );
    ensure!(
        state.protocol_compatible == (state.protocol_hex == utf8_hex(PROTOCOL_VALUE)),
        "helper protocol boolean did not match byte-exact metadata"
    );
    ensure!(
        state.wallet_verified
            == (state.agent_wallet == expected_wallet.to_string()
                && state.agent_wallet != Address::ZERO.to_string()),
        "helper wallet-verification boolean did not match the expected persistent Tentacle wallet"
    );
    Ok(state)
}

fn validate_write_result(value: &Value, expected_wallet: Address) -> Result<String> {
    let transaction_hash = required_string(value, "transactionHash", 66)?;
    validate_hash(&transaction_hash, "transaction hash")?;
    ensure!(
        Address::from_str(&required_string(value, "wallet", 42)?)? == expected_wallet,
        "ERC-8004 signer returned a transaction from another wallet"
    );
    ensure!(
        required_u64_number(value, "chainId")? == BASE_MAINNET_CHAIN_ID,
        "ERC-8004 signer returned a transaction for another chain"
    );
    ensure!(
        Address::from_str(&required_string(value, "registry", 42)?)?
            == Address::from_str(IDENTITY_REGISTRY)?,
        "ERC-8004 signer returned a transaction to another registry"
    );
    ensure!(
        required_decimal(value, "valueWei")? == "0",
        "ERC-8004 signer returned a nonzero-value transaction"
    );
    Ok(transaction_hash)
}

fn parse_funding(value: &Value, now: u64) -> Result<FundingStatus> {
    let funding = FundingStatus {
        balance_wei: required_decimal(value, "balanceWei")?,
        estimated_cost_wei: required_decimal(value, "estimatedCostWei")?,
        shortfall_wei: required_decimal(value, "shortfallWei")?,
        target_balance_wei: required_decimal(value, "targetBalanceWei")?,
        estimated_at_unix: now,
    };
    if funding.shortfall_wei != "0" {
        ensure!(
            decimal_less_or_equal(&funding.balance_wei, &funding.target_balance_wei),
            "funding estimate balance exceeds target while reporting a shortfall"
        );
    }
    Ok(funding)
}

fn validate_profile(profile: &Value, tentacle_id: &str, agent_id: &str) -> Result<()> {
    let object = profile
        .as_object()
        .context("registration document must be an object")?;
    ensure!(
        object.len() == 8,
        "registration document contains an unexpected top-level field"
    );
    for key in [
        "type",
        "name",
        "description",
        "image",
        "services",
        "x402Support",
        "active",
        "registrations",
    ] {
        ensure!(
            object.contains_key(key),
            "registration document is missing {key}"
        );
    }
    ensure!(
        object.get("type").and_then(Value::as_str) == Some(REGISTRATION_SCHEMA),
        "registration schema type is incompatible"
    );
    ensure!(
        object.get("x402Support").and_then(Value::as_bool) == Some(false),
        "x402 must remain disabled"
    );
    ensure!(
        object.get("active").and_then(Value::as_bool) == Some(true),
        "registration document must be active"
    );
    ensure!(
        object.get("supportedTrust").is_none(),
        "unimplemented trust mechanisms must not be advertised"
    );
    validate_public_text(
        object
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "registration name",
        MAX_PROFILE_NAME_BYTES,
    )?;
    validate_public_text(
        object
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "registration description",
        MAX_PROFILE_DESCRIPTION_BYTES,
    )?;
    validate_https_url(
        object
            .get("image")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "registration image",
    )?;
    let services = object
        .get("services")
        .and_then(Value::as_array)
        .context("registration services must be an array")?;
    ensure!(services.len() == 2, "registration services are incomplete");
    let xmtp_service = services[0]
        .as_object()
        .context("XMTP service must be an object")?;
    ensure!(
        xmtp_service.get("name").and_then(Value::as_str) == Some("CTHUWU-XMTP")
            && xmtp_service.get("version").and_then(Value::as_str) == Some("1"),
        "XMTP service name or version is incompatible"
    );
    let xmtp_endpoint = xmtp_service
        .get("endpoint")
        .and_then(Value::as_str)
        .context("XMTP service has no endpoint")?;
    let inbox_id = xmtp_endpoint
        .strip_prefix("xmtp://")
        .context("XMTP service uses an unsupported URI convention")?;
    validate_xmtp_inbox_id(inbox_id)?;
    let manifest_service = services[1]
        .as_object()
        .context("CTHUWU service must be an object")?;
    ensure!(
        manifest_service.get("name").and_then(Value::as_str) == Some("CTHUWU"),
        "CTHUWU manifest service is missing"
    );
    let manifest_endpoint = manifest_service
        .get("endpoint")
        .and_then(Value::as_str)
        .context("CTHUWU manifest service has no endpoint")?;
    ensure!(
        manifest_endpoint.starts_with("data:application/json;base64,")
            && manifest_endpoint.len() <= 2_048,
        "CTHUWU manifest endpoint is invalid or oversized"
    );
    let registrations = object
        .get("registrations")
        .and_then(Value::as_array)
        .context("registration references must be an array")?;
    ensure!(
        registrations.len() == 1,
        "registration document must contain one Base registry reference"
    );
    let registration = registrations[0]
        .as_object()
        .context("registry reference must be an object")?;
    ensure!(
        registration.get("agentId").and_then(Value::as_u64) == agent_id.parse::<u64>().ok(),
        "registration document agent ID is not the confirmed identity"
    );
    let expected_registry = format!("eip155:{BASE_MAINNET_CHAIN_ID}:{IDENTITY_REGISTRY}");
    ensure!(
        registration.get("agentRegistry").and_then(Value::as_str)
            == Some(expected_registry.as_str()),
        "registration document does not reference the canonical Base registry"
    );
    let encoded = serde_json::to_vec(profile)?;
    ensure!(
        encoded.len() <= 6 * 1024,
        "registration JSON exceeds its bound"
    );
    let rendered = String::from_utf8_lossy(&encoded);
    ensure!(
        ![
            "privateKey",
            "operatorIdentity",
            "localPath",
            "internalPrompt"
        ]
        .iter()
        .any(|forbidden| rendered.contains(forbidden)),
        "registration document contains a forbidden private field name"
    );
    ensure!(
        !tentacle_id.is_empty() && !agent_id.is_empty(),
        "registration identity fields must be present"
    );
    Ok(())
}

fn required_string(value: &Value, field: &str, maximum: usize) -> Result<String> {
    let string = value
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("helper result has no {field}"))?;
    ensure!(
        string.len() <= maximum
            && !string
                .chars()
                .any(|character| matches!(character, '\r' | '\n' | '\0')),
        "helper result field {field} is invalid"
    );
    Ok(string.to_owned())
}

fn required_bool(value: &Value, field: &str) -> Result<bool> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .with_context(|| format!("helper result has no {field}"))
}

fn required_address(value: &Value, field: &str) -> Result<String> {
    let string = required_string(value, field, 42)?;
    Address::from_str(&string)
        .with_context(|| format!("helper result {field} is not an address"))?;
    Ok(string.to_ascii_lowercase())
}

fn required_hex(value: &Value, field: &str, maximum: usize) -> Result<String> {
    let string = required_string(value, field, maximum)?;
    ensure!(
        string.starts_with("0x")
            && string.len() % 2 == 0
            && string[2..].bytes().all(|byte| byte.is_ascii_hexdigit()),
        "helper result {field} is not canonical bytes"
    );
    Ok(string.to_ascii_lowercase())
}

fn required_decimal(value: &Value, field: &str) -> Result<String> {
    let string = required_string(value, field, 80)?;
    validate_decimal(&string, field)?;
    Ok(string)
}

fn parse_u64_field(value: &Value, field: &str) -> Result<u64> {
    required_decimal(value, field)?
        .parse()
        .with_context(|| format!("helper result {field} exceeds u64"))
}

fn required_u64_number(value: &Value, field: &str) -> Result<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .with_context(|| format!("helper result {field} is not an unsigned integer"))
}

fn validate_agent_id(value: &str) -> Result<()> {
    validate_decimal(value, "agent ID")?;
    ensure!(value.len() <= 78, "agent ID exceeds uint256");
    Ok(())
}

fn validate_decimal(value: &str, label: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 80,
        "{label} is empty or oversized"
    );
    ensure!(
        value.bytes().all(|byte| byte.is_ascii_digit()),
        "{label} must be a decimal integer"
    );
    ensure!(
        value == "0" || !value.starts_with('0'),
        "{label} must not contain leading zeroes"
    );
    Ok(())
}

/// Treat target or shortfall movement of at least ten percent as material. Smaller Base fee
/// jitter waits for the persisted cooldown instead of repeatedly notifying an operator.
fn funding_materially_changed(previous: &FundingStatus, current: &FundingStatus) -> bool {
    [
        (&previous.shortfall_wei, &current.shortfall_wei),
        (&previous.target_balance_wei, &current.target_balance_wei),
        (&previous.estimated_cost_wei, &current.estimated_cost_wei),
    ]
    .into_iter()
    .any(|(old, new)| {
        let Ok(old) = old.parse::<u128>() else {
            return true;
        };
        let Ok(new) = new.parse::<u128>() else {
            return true;
        };
        old.abs_diff(new) >= (old / 10).max(1)
    })
}

fn validate_hash(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() == 66
            && value.starts_with("0x")
            && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{label} must be exactly 32 bytes"
    );
    Ok(())
}

fn validate_action_id(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b":_-".contains(&byte)),
        "action ID is invalid"
    );
    Ok(())
}

fn validate_xmtp_inbox_id(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "XMTP production inbox ID must be exactly 32 lowercase hexadecimal bytes"
    );
    Ok(())
}

fn validate_public_text(value: &str, label: &str, maximum: usize) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= maximum,
        "{label} is empty or oversized"
    );
    ensure!(
        !value.chars().any(|character| character.is_control()),
        "{label} contains control characters"
    );
    Ok(())
}

fn validate_https_url(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.starts_with("https://")
            && value.len() <= 2_048
            && !value.chars().any(|character| character.is_control()),
        "{label} must be a bounded HTTPS URL"
    );
    Ok(())
}

fn validate_rpc_endpoint(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= 4_096,
        "RPC endpoint is empty or oversized"
    );
    ensure!(
        value.starts_with("https://")
            || value.starts_with("http://127.0.0.1:")
            || value.starts_with("http://localhost:"),
        "RPC endpoint must use HTTPS"
    );
    ensure!(
        !value.contains('@'),
        "RPC endpoint must not contain URL credentials"
    );
    Ok(())
}

fn decimal_less_or_equal(left: &str, right: &str) -> bool {
    left.len() < right.len() || (left.len() == right.len() && left <= right)
}

fn failure_from_error(detail: &str, recoverable: bool, now: u64) -> RegistrationFailure {
    RegistrationFailure {
        code: if recoverable {
            "recoverable"
        } else {
            "permanent"
        }
        .to_owned(),
        detail: bounded_diagnostic(detail, 512),
        recoverable,
        failed_at_unix: now,
    }
}

fn is_recoverable_error(error: &anyhow::Error) -> bool {
    !error
        .to_string()
        .contains("permanent ERC-8004 helper error")
}

fn bounded_diagnostic(value: &str, maximum: usize) -> String {
    value
        .chars()
        .map(|character| {
            if matches!(character, '\r' | '\n') {
                ' '
            } else {
                character
            }
        })
        .take(maximum)
        .collect()
}

fn unique_action_id(tentacle_id: &str, action: &PendingAction, now: u64) -> Result<String> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).context("generating a unique ERC-8004 action ID")?;
    let mut digest = Sha256::new();
    digest.update(tentacle_id.as_bytes());
    digest.update(now.to_be_bytes());
    digest.update(entropy);
    digest.update(serde_json::to_vec(action).unwrap_or_default());
    Ok(format!("erc8004:{}", &hex(&digest.finalize())[..40]))
}

fn sha256_hex(value: &[u8]) -> String {
    hex(&Sha256::digest(value))
}

fn hex(value: &[u8]) -> String {
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn utf8_hex(value: &str) -> String {
    format!("0x{}", hex(value.as_bytes()))
}

fn base64(value: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(value.len().div_ceil(3) * 4);
    for chunk in value.chunks(3) {
        let first = u32::from(chunk[0]);
        let second = chunk.get(1).copied().map(u32::from).unwrap_or(0);
        let third = chunk.get(2).copied().map(u32::from).unwrap_or(0);
        let bits = (first << 16) | (second << 8) | third;
        output.push(ALPHABET[((bits >> 18) & 0x3f) as usize] as char);
        output.push(ALPHABET[((bits >> 12) & 0x3f) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[((bits >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(bits & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    output
}

fn unix_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates Unix epoch")?
        .as_secs())
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing symlinked ERC-8004 state path {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

fn copy_network_environment(command: &mut Command) {
    for name in [
        "SYSTEMROOT",
        "WINDIR",
        "PATH",
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
}

pub fn active_operator_inboxes(operators: &crate::principal::OperatorStore) -> Vec<String> {
    operators
        .list()
        .filter(|(_, _, status, _)| *status == "active")
        .map(|(inbox, _, _, _)| inbox.to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex as StdMutex};

    struct UnusedGateway;

    #[async_trait]
    impl Erc8004Gateway for UnusedGateway {
        async fn invoke(&self, _action_id: &str, _operation: Value) -> Result<Value> {
            bail!("unused")
        }
    }

    enum ScriptResult {
        Ok(Value),
        Err(&'static str),
    }

    struct ScriptedGateway {
        steps: StdMutex<VecDeque<(&'static str, ScriptResult)>>,
        calls: StdMutex<Vec<String>>,
        operations: StdMutex<Vec<Value>>,
    }

    impl ScriptedGateway {
        fn new(steps: Vec<(&'static str, ScriptResult)>) -> Self {
            Self {
                steps: StdMutex::new(steps.into()),
                calls: StdMutex::new(Vec::new()),
                operations: StdMutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn assert_exhausted(&self) {
            assert!(self.steps.lock().unwrap().is_empty());
        }

        fn operations(&self) -> Vec<Value> {
            self.operations.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Erc8004Gateway for ScriptedGateway {
        async fn invoke(&self, _action_id: &str, operation: Value) -> Result<Value> {
            let operation_type = operation
                .get("type")
                .and_then(Value::as_str)
                .context("scripted operation has no type")?
                .to_owned();
            self.calls.lock().unwrap().push(operation_type.clone());
            self.operations.lock().unwrap().push(operation);
            let (expected, result) = self
                .steps
                .lock()
                .unwrap()
                .pop_front()
                .context("unexpected scripted ERC-8004 call")?;
            ensure!(
                operation_type == expected,
                "expected {expected}, got {operation_type}"
            );
            match result {
                ScriptResult::Ok(value)
                    if operation_type == "inspect_registry"
                        && value.as_object().is_some_and(serde_json::Map::is_empty) =>
                {
                    Ok(canonical_deployment_result())
                }
                ScriptResult::Ok(value) => Ok(value),
                ScriptResult::Err(message) => bail!("{message}"),
            }
        }
    }

    fn canonical_deployment_result() -> Value {
        json!({
            "chainId": BASE_MAINNET_CHAIN_ID,
            "identityRegistry": IDENTITY_REGISTRY,
            "reputationRegistry": REPUTATION_REGISTRY,
            "identityImplementation": "0x7274e874CA62410a93Bd8bf61c69d8045E399c02",
            "reputationImplementation": "0x16e0FA7f7C56B9a767E34B192B51f921BE31dA34",
            "identityVersion": "2.0.0",
            "reputationVersion": "2.0.0",
            "reputationIdentityRegistry": IDENTITY_REGISTRY,
            "identityProxyCodeBytes": 128,
            "reputationProxyCodeBytes": 128,
            "identityImplementationCodeBytes": 1024,
            "reputationImplementationCodeBytes": 1024,
            "interfaceRevision": "registration-v1",
            "interfaceComplete": true,
            "pinnedRevision": PINNED_CONTRACT_REVISION,
            "blockNumber": "50000000",
            "blockHash": format!("0x{}", "dd".repeat(32)),
        })
    }

    fn canonical_inbox_result() -> Value {
        json!({
            "wallet": wallet().to_string(),
            "inboxId": "ab".repeat(32),
            "endpoint": format!("xmtp://{}", "ab".repeat(32)),
            "environment": "production",
        })
    }

    fn adequate_funding_result() -> Value {
        json!({
            "balanceWei": "1000",
            "estimatedCostWei": "100",
            "targetBalanceWei": "200",
            "shortfallWei": "0",
        })
    }

    fn submitted_result(byte: &str) -> Value {
        submitted_result_with_nonce(byte, "7")
    }

    fn submitted_result_with_nonce(byte: &str, nonce: &str) -> Value {
        json!({
            "transactionHash": format!("0x{}", byte.repeat(32)),
            "wallet": wallet().to_string(),
            "chainId": BASE_MAINNET_CHAIN_ID,
            "registry": IDENTITY_REGISTRY,
            "valueWei": "0",
            "transactionNonce": nonce,
        })
    }

    fn register_nonce_result(pending: &str, latest: &str) -> Value {
        json!({
            "wallet": wallet().to_string(),
            "chainId": BASE_MAINNET_CHAIN_ID,
            "registry": IDENTITY_REGISTRY,
            "pendingNonce": pending,
            "latestNonce": latest,
            "observedBlockNumber": "50000000",
            "observedBlockHash": format!("0x{}", "cc".repeat(32)),
        })
    }

    fn discovery_result(candidates: Value) -> Value {
        json!({
            "complete": true,
            "observedBlockNumber": "50000000",
            "observedBlockHash": format!("0x{}", "cc".repeat(32)),
            "matchedRegistrationAgentIds": [],
            "candidates": candidates,
        })
    }

    fn empty_discovery_result() -> Value {
        discovery_result(json!([]))
    }

    fn receipt_result(byte: &str, agent_id: Option<&str>) -> Value {
        json!({
            "status": "success",
            "transactionHash": format!("0x{}", byte.repeat(32)),
            "blockNumber": "50000000",
            "blockHash": format!("0x{}", "aa".repeat(32)),
            "canonicalBlockHash": format!("0x{}", "aa".repeat(32)),
            "confirmations": "12",
            "agentId": agent_id,
        })
    }

    fn inspected_agent(
        agent_uri: &str,
        agent_wallet: Address,
        allegiance: &str,
        protocol: &str,
        tentacle_id: &str,
    ) -> Value {
        json!({
            "owner": wallet().to_string(),
            "agentURI": agent_uri,
            "agentWallet": agent_wallet.to_string(),
            "authorized": true,
            "allegiance": {"hex": utf8_hex(allegiance), "utf8": allegiance},
            "protocol": {"hex": utf8_hex(protocol), "utf8": protocol},
            "tentacleId": {"hex": utf8_hex(tentacle_id), "utf8": tentacle_id},
            "declaresTentacleAllegiance": allegiance == ALLEGIANCE_VALUE,
            "protocolCompatible": protocol == PROTOCOL_VALUE,
            "walletVerified": agent_wallet == wallet(),
        })
    }

    fn scripted_registration(
        root: &Path,
        gateway: Arc<dyn Erc8004Gateway>,
    ) -> TentacleRegistration {
        let config = RegistrationConfig {
            confirmations: 1,
            initial_public_name: Some("Cthulhu the Test-Bound".to_owned()),
            ..RegistrationConfig::default()
        };
        TentacleRegistration::open(root, "tentacle-independent", wallet(), config, gateway).unwrap()
    }

    fn wallet() -> Address {
        Address::from_str("0x1111111111111111111111111111111111111111").unwrap()
    }

    fn registration(root: &Path) -> TentacleRegistration {
        let config = RegistrationConfig {
            initial_public_name: Some("Cthulhu the Test-Bound".to_owned()),
            ..RegistrationConfig::default()
        };
        let mut registration = TentacleRegistration::open(
            root,
            "tentacle-independent",
            wallet(),
            config,
            Arc::new(UnusedGateway),
        )
        .unwrap();
        registration.state.xmtp_inbox_id = Some("ab".repeat(32));
        registration
    }

    #[test]
    fn ontology_registers_a_tentacle_and_never_the_collective() {
        let root = tempfile::tempdir().unwrap();
        let mut registration = registration(root.path());
        assert_eq!(registration.snapshot().tentacle_id, "tentacle-independent");
        assert!(
            !serde_json::to_string(registration.snapshot())
                .unwrap()
                .contains("cthulhu_id")
        );
        assert!(registration.config.public_description.contains("Tentacle"));
        assert!(
            registration
                .config
                .public_description
                .contains("centerless Cthuwu collective")
        );
        let pending = registration.public_status_text();
        assert!(pending.contains("this durable Tentacle"));
        assert!(pending.contains("does not have a confirmed"));
        assert!(pending.contains("collective"));
        registration.state.confirmed_agent_id = Some("42".to_owned());
        registration.state.phase = RegistrationPhase::Active;
        let active = registration.public_status_text();
        assert!(active.contains("agent ID `42`"));
        assert!(active.contains("Tentacle has its own"));
        assert!(active.contains("collective does not own"));
    }

    #[test]
    fn production_sidecar_observation_constructs_existing_council_registry_adapter() {
        let root = tempfile::tempdir().unwrap();
        let mut registration = registration(root.path());
        registration.last_deployment = Some(canonical_deployment_result());
        let final_uri = registration.build_agent_uri("42").unwrap();
        let inspected = inspected_agent(
            &final_uri,
            wallet(),
            ALLEGIANCE_VALUE,
            PROTOCOL_VALUE,
            "tentacle-independent",
        );
        registration
            .validate_council_registry_binding("42", &inspected)
            .unwrap();
        let record = registration.last_registry_record.as_ref().unwrap();
        assert_eq!(record.id.as_str(), "tentacle-independent");
        assert!(record.active);
        assert_eq!(record.endpoints.len(), 1);
        assert!(
            registration
                .status_text()
                .contains("COUNCIL ERC-8004 ADAPTER: VERIFIED")
        );

        let mut hostile_profile = inspected.clone();
        hostile_profile["agentURI"] =
            Value::String("https://hostile.invalid/profile.json".to_owned());
        registration
            .validate_council_registry_binding("42", &hostile_profile)
            .unwrap();
        let conservative = registration.last_registry_record.as_ref().unwrap();
        assert!(conservative.endpoints.is_empty());
        assert!(conservative.capability_refs.is_empty());

        let mut wrong = canonical_deployment_result();
        wrong["identityImplementation"] =
            Value::String("0x3333333333333333333333333333333333333333".to_owned());
        registration.last_deployment = Some(wrong);
        assert!(
            registration
                .validate_council_registry_binding("42", &inspected)
                .is_err()
        );
    }

    #[tokio::test]
    async fn explicit_adoption_requires_the_canonical_council_adapter_binding() {
        let template_root = tempfile::tempdir().unwrap();
        let expected_uri = registration(template_root.path())
            .build_agent_uri("42")
            .unwrap();

        let root = tempfile::tempdir().unwrap();
        let gateway = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            (
                "inspect_agent",
                ScriptResult::Ok(inspected_agent(
                    &expected_uri,
                    wallet(),
                    ALLEGIANCE_VALUE,
                    PROTOCOL_VALUE,
                    "tentacle-independent",
                )),
            ),
        ]));
        let mut adoption = scripted_registration(root.path(), gateway.clone());
        adoption.state.xmtp_inbox_id = Some("ab".repeat(32));

        assert!(adoption.adopt("42").await.unwrap().contains("ADOPTED"));
        assert_eq!(adoption.state.confirmed_agent_id.as_deref(), Some("42"));
        assert!(adoption.last_registry_record.as_ref().unwrap().active);
        assert_eq!(gateway.calls(), ["inspect_registry", "inspect_agent"]);
        gateway.assert_exhausted();

        let rejected_root = tempfile::tempdir().unwrap();
        let rejected_gateway = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            (
                "inspect_agent",
                ScriptResult::Ok(inspected_agent(
                    &expected_uri,
                    wallet(),
                    ALLEGIANCE_VALUE,
                    PROTOCOL_VALUE,
                    "tentacle-other",
                )),
            ),
        ]));
        let mut rejected = scripted_registration(rejected_root.path(), rejected_gateway.clone());
        rejected.state.xmtp_inbox_id = Some("ab".repeat(32));

        assert!(rejected.adopt("42").await.is_err());
        assert!(rejected.state.confirmed_agent_id.is_none());
        assert!(rejected.last_registry_record.is_none());
        assert_eq!(
            rejected_gateway.calls(),
            ["inspect_registry", "inspect_agent"]
        );
        rejected_gateway.assert_exhausted();
    }

    #[tokio::test]
    async fn malformed_deployment_observation_blocks_all_follow_up_operations() {
        let root = tempfile::tempdir().unwrap();
        let mut malformed = canonical_deployment_result();
        malformed["blockHash"] = Value::String("0x01".to_owned());
        let gateway = Arc::new(ScriptedGateway::new(vec![(
            "inspect_registry",
            ScriptResult::Ok(malformed),
        )]));
        let mut registration = scripted_registration(root.path(), gateway.clone());

        assert!(registration.maintain(false).await.is_err());
        assert_eq!(gateway.calls(), ["inspect_registry"]);
        gateway.assert_exhausted();
    }

    #[test]
    fn exact_allegiance_is_byte_exact_and_case_sensitive() {
        assert_eq!(
            ALLEGIANCE_VALUE.as_bytes(),
            cthuwu_council::registry::CTHUWU_ALLEGIANCE_VALUE
        );
        assert_eq!(
            PROTOCOL_VALUE.as_bytes(),
            cthuwu_council::registry::CTHUWU_PROTOCOL_VALUE
        );
        assert_eq!(
            utf8_hex(ALLEGIANCE_VALUE),
            "0x7577752d74656e7461636c652d7631"
        );
        assert_ne!(utf8_hex(ALLEGIANCE_VALUE), utf8_hex("UWU-TENTACLE-V1"));
        assert_ne!(utf8_hex(ALLEGIANCE_VALUE), utf8_hex("uwu-tentacle-v1 "));
    }

    #[test]
    fn registration_document_uses_current_schema_and_no_unimplemented_trust() {
        let root = tempfile::tempdir().unwrap();
        let mut registration = registration(root.path());
        let uri = registration.build_agent_uri("42").unwrap();
        assert!(uri.starts_with("data:application/json;base64,"));
        assert!(uri.len() <= MAX_AGENT_URI_BYTES);
        assert!(!uri.contains("A2A"));
        assert!(!uri.contains("x402Support")); // base64-encoded, not a mutable clear-text URL.
        registration.state.public_name = "Yogsoth the Spiral Witness".to_owned();
        assert_ne!(registration.build_agent_uri("42").unwrap(), uri);
    }

    #[test]
    fn registration_document_requires_the_real_canonical_xmtp_inbox() {
        let root = tempfile::tempdir().unwrap();
        let mut registration = registration(root.path());
        registration.state.xmtp_inbox_id = None;
        assert!(
            registration
                .build_agent_uri("42")
                .unwrap_err()
                .to_string()
                .contains("positively resolved")
        );
        assert!(validate_xmtp_inbox_id(&"AB".repeat(32)).is_err());
        assert!(validate_xmtp_inbox_id(&"ab".repeat(32)).is_ok());
    }

    #[tokio::test]
    async fn a_stale_registry_name_repairs_only_the_agent_uri() {
        let root = tempfile::tempdir().unwrap();
        let mut seeded = registration(root.path());
        seeded.state.selected_agent_id = Some("42".to_owned());
        seeded.state.confirmed_agent_id = Some("42".to_owned());
        seeded.state.phase = RegistrationPhase::Active;
        seeded.persist(1).unwrap();
        drop(seeded);

        let old_name_uri = "data:application/json;base64,e30=";
        let gateway = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            (
                "inspect_agent",
                ScriptResult::Ok(inspected_agent(
                    old_name_uri,
                    wallet(),
                    ALLEGIANCE_VALUE,
                    PROTOCOL_VALUE,
                    "tentacle-independent",
                )),
            ),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("7", "7")),
            ),
            ("set_agent_uri", ScriptResult::Ok(submitted_result("21"))),
        ]));
        let mut repairing = scripted_registration(root.path(), gateway.clone());
        repairing.maintain(false).await.unwrap();
        assert_eq!(repairing.state.phase, RegistrationPhase::Submitted);
        assert_eq!(
            repairing.state.submitted_action,
            Some(PendingAction::SetAgentUri)
        );
        assert_eq!(
            gateway.calls(),
            [
                "inspect_registry",
                "inspect_agent",
                "funding_estimate",
                "transaction_nonce",
                "set_agent_uri"
            ]
        );
        let write = gateway
            .operations()
            .into_iter()
            .find(|operation| {
                operation.get("type").and_then(Value::as_str) == Some("set_agent_uri")
            })
            .unwrap();
        let expected_uri = repairing.build_agent_uri("42").unwrap();
        assert_eq!(write["agentURI"].as_str(), Some(expected_uri.as_str()));
        gateway.assert_exhausted();
    }

    #[test]
    fn legacy_snapshot_migration_is_explicit_and_versioned() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("state")).unwrap();
        let legacy = json!({
            "version": 1,
            "cthulhu_id": "tentacle-independent",
            "chain_id": BASE_MAINNET_CHAIN_ID,
            "identity_registry": IDENTITY_REGISTRY,
            "tentacle_wallet": wallet().to_string(),
            "agent_id": "7"
        });
        fs::write(
            root.path().join("state").join(SNAPSHOT_FILE),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        let registration = registration(root.path());
        assert_eq!(registration.state.migrated_from_version, Some(1));
        assert_eq!(registration.state.confirmed_agent_id.as_deref(), Some("7"));
        assert_eq!(
            registration.state.phase,
            RegistrationPhase::ConfirmedIdentity
        );
        assert!(!registration.state.public_name.is_empty());
    }

    #[test]
    fn version_two_snapshot_gains_one_persisted_name() {
        let root = tempfile::tempdir().unwrap();
        let original = registration(root.path()).state;
        let mut value = serde_json::to_value(original).unwrap();
        let object = value.as_object_mut().unwrap();
        object.insert("version".to_owned(), json!(PREVIOUS_SNAPSHOT_VERSION));
        object.remove("public_name");
        fs::write(
            root.path().join("state").join(SNAPSHOT_FILE),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();

        let config = RegistrationConfig {
            initial_public_name: Some("Azathoth the Patient Hunger".to_owned()),
            ..RegistrationConfig::default()
        };
        let migrated = TentacleRegistration::open(
            root.path(),
            "tentacle-independent",
            wallet(),
            config,
            Arc::new(UnusedGateway),
        )
        .unwrap();
        assert_eq!(migrated.state.version, SNAPSHOT_VERSION);
        assert_eq!(migrated.state.migrated_from_version, Some(2));
        assert_eq!(
            migrated.state.public_name,
            "Azathoth the Patient Hunger"
        );
    }

    #[test]
    fn persisted_public_name_outranks_later_startup_overrides() {
        let root = tempfile::tempdir().unwrap();
        let first_config = RegistrationConfig {
            initial_public_name: Some("Cthulhu the Star-Entombed".to_owned()),
            ..RegistrationConfig::default()
        };
        let first = TentacleRegistration::open(
            root.path(),
            "tentacle-independent",
            wallet(),
            first_config,
            Arc::new(UnusedGateway),
        )
        .unwrap();
        assert_eq!(first.state.public_name, "Cthulhu the Star-Entombed");
        drop(first);

        let later_config = RegistrationConfig {
            // An unused first-boot seed cannot corrupt or block a valid persisted identity.
            initial_public_name: Some("bad\nname".to_owned()),
            ..RegistrationConfig::default()
        };
        let reopened = TentacleRegistration::open(
            root.path(),
            "tentacle-independent",
            wallet(),
            later_config,
            Arc::new(UnusedGateway),
        )
        .unwrap();
        assert_eq!(reopened.state.public_name, "Cthulhu the Star-Entombed");
    }

    #[test]
    fn canonical_configuration_rejects_wrong_chain_contract_and_corruption() {
        let mut state = RegistrationSnapshot::fresh(
            "tentacle-independent",
            "Cthulhu the Test-Bound".to_owned(),
            wallet(),
            1,
        );
        state.chain_id = 1;
        assert!(state.validate("tentacle-independent", wallet()).is_err());
        state.chain_id = BASE_MAINNET_CHAIN_ID;
        state.identity_registry = Address::ZERO.to_string();
        assert!(state.validate("tentacle-independent", wallet()).is_err());
    }

    #[test]
    fn funding_notification_is_exact_and_rate_limited() {
        let root = tempfile::tempdir().unwrap();
        let mut registration = registration(root.path());
        registration.state.funding = Some(FundingStatus {
            balance_wei: "10".into(),
            estimated_cost_wei: "90".into(),
            shortfall_wei: "100".into(),
            target_balance_wei: "110".into(),
            estimated_at_unix: 1,
        });
        let first = registration
            .funding_notification_if_due(100, false)
            .unwrap();
        assert_eq!(first.len(), 1);
        assert!(first[0].text.contains(&wallet().to_string()));
        assert!(first[0].text.contains("Base mainnet"));
        assert!(first[0].text.contains("DO NOT SEND ETH ON ANY OTHER CHAIN"));
        registration.mark_notifications_delivered(&first).unwrap();
        assert!(
            registration
                .funding_notification_if_due(101, false)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            registration
                .funding_notification_if_due(101, true)
                .unwrap()
                .len(),
            1
        );
        registration.state.funding = Some(FundingStatus {
            balance_wei: "11".into(),
            estimated_cost_wei: "91".into(),
            shortfall_wei: "99".into(),
            target_balance_wei: "110".into(),
            estimated_at_unix: 2,
        });
        assert!(
            registration
                .funding_notification_if_due(102, false)
                .unwrap()
                .is_empty()
        );
        registration.state.funding = Some(FundingStatus {
            balance_wei: "30".into(),
            estimated_cost_wei: "90".into(),
            shortfall_wei: "80".into(),
            target_balance_wei: "110".into(),
            estimated_at_unix: 3,
        });
        assert_eq!(
            registration
                .funding_notification_if_due(103, false)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn startup_resource_audit_demands_current_blockers_despite_cooldown() {
        let root = tempfile::tempdir().unwrap();
        let mut registration = registration(root.path());
        let funding = FundingStatus {
            balance_wei: "10".into(),
            estimated_cost_wei: "90".into(),
            shortfall_wei: "100".into(),
            target_balance_wei: "110".into(),
            estimated_at_unix: 1,
        };
        registration.state.phase = RegistrationPhase::FundingRequired;
        registration.state.funding = Some(funding.clone());
        registration.state.last_operator_notification_unix = Some(99);
        registration.state.last_notified_funding = Some(funding);

        let notice = registration
            .startup_resource_notification_if_needed(100)
            .unwrap()
            .unwrap();
        assert!(
            notice
                .text
                .starts_with("IMMEDIATE OPERATOR ACTION REQUIRED")
        );
        assert!(notice.text.contains("SEND THE REQUIRED BASE ETH"));
        assert!(notice.text.contains(&wallet().to_string()));
        assert!(notice.text.contains("NEVER SEND A WALLET PRIVATE KEY"));

        registration.state.phase = RegistrationPhase::FailedRecoverable;
        registration.state.funding = None;
        registration.state.failure = Some(failure_from_error(
            "RPC request failed: over rate limit",
            true,
            101,
        ));
        let rpc_notice = registration
            .startup_resource_notification_if_needed(101)
            .unwrap()
            .unwrap();
        assert!(rpc_notice.text.contains("RESTORE BASE RPC ACCESS"));
        assert!(rpc_notice.text.contains("/base-rpc-key <infura-api-key>"));

        registration.state.phase = RegistrationPhase::Active;
        registration.state.failure = None;
        assert!(
            registration
                .startup_resource_notification_if_needed(102)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn provider_query_limit_is_a_recoverable_base_rpc_blocker() {
        let failure = failure_from_error(
            "recoverable ERC-8004 helper error [rpc_or_signing_failure]: Request exceeds defined limit. URL: <redacted-rpc>",
            true,
            1,
        );
        assert!(is_base_rpc_access_failure(&failure));
    }

    #[test]
    fn automatic_maintenance_tracks_pending_transactions_promptly_and_backs_off_rpc_failures() {
        let root = tempfile::tempdir().unwrap();
        let mut registration = registration(root.path());
        registration.config.maintenance_interval = Duration::from_secs(15 * 60);

        registration.state.phase = RegistrationPhase::Submitted;
        assert_eq!(registration.maintenance_interval(), Duration::from_secs(15));

        registration.state.failure = Some(failure_from_error(
            "RPC request failed: over rate limit",
            true,
            1,
        ));
        assert_eq!(
            registration.maintenance_interval(),
            Duration::from_secs(60 * 60)
        );

        registration.state.failure = None;
        registration.state.phase = RegistrationPhase::Active;
        assert_eq!(
            registration.maintenance_interval(),
            Duration::from_secs(15 * 60)
        );
    }

    #[tokio::test]
    async fn automatic_discovery_is_recent_and_explicit_recovery_is_exhaustive() {
        for (exhaustive, expected_scope) in [(false, "recent"), (true, "exhaustive")] {
            let root = tempfile::tempdir().unwrap();
            let gateway = Arc::new(ScriptedGateway::new(vec![
                ("inspect_registry", ScriptResult::Ok(json!({}))),
                ("discover", ScriptResult::Err("RPC request failed")),
            ]));
            let mut registration = scripted_registration(root.path(), gateway.clone());
            registration.state.xmtp_inbox_id = Some("ab".repeat(32));
            registration
                .maintain_with_discovery(false, exhaustive)
                .await
                .unwrap();
            let operations = gateway.operations();
            assert_eq!(operations[1]["scope"], expected_scope);
            gateway.assert_exhausted();
        }
    }

    #[tokio::test]
    async fn fresh_registration_shortfall_is_occasionally_shared_with_acolytes() {
        let root = tempfile::tempdir().unwrap();
        let now = unix_seconds().unwrap();
        let mut registration = registration(root.path());
        registration.state.phase = RegistrationPhase::FundingRequired;
        registration.state.funding = Some(FundingStatus {
            balance_wei: "10".into(),
            estimated_cost_wei: "90".into(),
            shortfall_wei: "100".into(),
            target_balance_wei: "110".into(),
            estimated_at_unix: now,
        });
        let registration = Arc::new(Mutex::new(registration));
        let control = SharedRegistrationControl::new(registration.clone()).await;

        let first = control.take_public_funding_plea().await.unwrap();
        assert!(first.contains(&wallet().to_string()));
        assert!(first.contains("Base ETH only"));
        assert!(first.contains("estimated shortfall: 100 wei"));
        assert!(first.contains("resume automatically"));
        assert!(first.contains("don't send ETH on any other chain"));
        for _ in 0..4 {
            assert!(control.take_public_funding_plea().await.is_none());
        }
        assert!(control.take_public_funding_plea().await.is_some());

        let maintenance_guard = registration.lock().await;
        assert_eq!(
            control.public_name().await.as_deref(),
            Some("Cthulhu the Test-Bound")
        );
        assert!(control.take_public_funding_plea().await.is_none());
        drop(maintenance_guard);

        registration.lock().await.state.funding = Some(FundingStatus {
            balance_wei: "10".into(),
            estimated_cost_wei: "90".into(),
            shortfall_wei: "100".into(),
            target_balance_wei: "110".into(),
            estimated_at_unix: now
                .saturating_sub(DEFAULT_MAINTENANCE_INTERVAL_SECONDS.saturating_mul(2) + 1),
        });
        assert!(control.take_public_funding_plea().await.is_none());
    }

    #[tokio::test]
    async fn recoverable_base_rpc_failure_requests_a_key_from_operator_and_acolytes() {
        let root = tempfile::tempdir().unwrap();
        let mut registration = registration(root.path());
        registration.state.phase = RegistrationPhase::FailedRecoverable;
        registration.state.failure = Some(RegistrationFailure {
            code: "recoverable".to_owned(),
            detail: "recoverable ERC-8004 helper error [rpc_or_signing_failure]: RPC Request failed: over rate limit".to_owned(),
            recoverable: true,
            failed_at_unix: unix_seconds().unwrap(),
        });

        let status = registration.status_text();
        assert!(status.contains("/base-rpc-key <infura-api-key>"));
        assert!(status.contains("https://app.infura.io/"));
        assert!(!status.contains("CTHUWU_RPC_ENDPOINT"));

        let registration = Arc::new(Mutex::new(registration));
        let control = SharedRegistrationControl::new(registration).await;
        let plea = control.take_public_funding_plea().await.unwrap();
        assert!(plea.contains("/base-rpc-key <infura-api-key>"));
        assert!(plea.contains("without a restart"));
        assert!(plea.contains("https://app.infura.io/"));
        assert!(plea.contains("never send a wallet private key"));
    }

    #[test]
    fn notification_state_is_committed_only_after_transport_ack() {
        let root = tempfile::tempdir().unwrap();
        let mut registration = registration(root.path());
        let success = registration.success_notification_if_due("42").unwrap();
        assert!(!registration.state.success_notified);
        registration
            .mark_notifications_delivered(std::slice::from_ref(&success))
            .unwrap();
        assert!(registration.state.success_notified);

        registration.state.last_operator_notification_unix = None;
        registration.state.last_notified_funding_fingerprint = None;
        registration.state.last_notified_funding = None;
        registration.state.funding = Some(FundingStatus {
            balance_wei: "1".to_owned(),
            estimated_cost_wei: "2".to_owned(),
            shortfall_wei: "3".to_owned(),
            target_balance_wei: "4".to_owned(),
            estimated_at_unix: 100,
        });
        let funding = registration
            .funding_notification_if_due(100, false)
            .unwrap();
        assert!(registration.state.last_operator_notification_unix.is_none());
        assert!(registration.state.last_notified_funding.is_none());
        registration.mark_notifications_delivered(&funding).unwrap();
        assert_eq!(
            registration.state.last_operator_notification_unix,
            Some(100)
        );
        assert!(registration.state.last_notified_funding.is_some());
    }

    #[test]
    fn crash_before_transport_ack_keeps_funding_and_success_notices_eligible() {
        let root = tempfile::tempdir().unwrap();
        let mut before_crash = registration(root.path());
        before_crash.state.funding = Some(FundingStatus {
            balance_wei: "1".to_owned(),
            estimated_cost_wei: "2".to_owned(),
            shortfall_wei: "3".to_owned(),
            target_balance_wei: "4".to_owned(),
            estimated_at_unix: 100,
        });
        before_crash.persist(100).unwrap();
        assert_eq!(
            before_crash
                .funding_notification_if_due(100, false)
                .unwrap()
                .len(),
            1
        );
        assert!(before_crash.success_notification_if_due("42").is_some());
        drop(before_crash);

        let mut recovered = registration(root.path());
        assert_eq!(
            recovered
                .funding_notification_if_due(101, false)
                .unwrap()
                .len(),
            1
        );
        assert!(recovered.success_notification_if_due("42").is_some());
    }

    #[test]
    fn intent_is_persisted_before_broadcast_and_blocks_duplicate_submission() {
        let root = tempfile::tempdir().unwrap();
        let mut registration = registration(root.path());
        registration
            .prepare_action(PendingAction::Register, 5)
            .unwrap();
        assert_eq!(registration.state.phase, RegistrationPhase::Preparing);
        assert!(registration.state.action_id.is_some());
        assert!(
            registration
                .prepare_action(PendingAction::Register, 6)
                .is_err()
        );
        registration.state.submitted_transaction_hash = Some(format!("0x{}", "11".repeat(32)));
        assert!(
            registration
                .prepare_action(PendingAction::Register, 7)
                .is_err()
        );
    }

    #[test]
    fn action_ids_do_not_collide_for_the_same_action_and_second() {
        let first_root = tempfile::tempdir().unwrap();
        let second_root = tempfile::tempdir().unwrap();
        let mut first = registration(first_root.path());
        let mut second = registration(second_root.path());
        first.prepare_action(PendingAction::Register, 5).unwrap();
        second.prepare_action(PendingAction::Register, 5).unwrap();
        assert_ne!(first.state.action_id, second.state.action_id);
    }

    #[tokio::test]
    async fn known_submission_is_recovered_after_restart_without_a_second_mint() {
        let root = tempfile::tempdir().unwrap();
        let first = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("resolve_inbox", ScriptResult::Ok(canonical_inbox_result())),
            ("discover", ScriptResult::Ok(empty_discovery_result())),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("7", "7")),
            ),
            ("register", ScriptResult::Ok(submitted_result("11"))),
        ]));
        let mut registration = scripted_registration(root.path(), first.clone());
        assert!(registration.maintain(false).await.unwrap().is_empty());
        assert_eq!(registration.state.phase, RegistrationPhase::Submitted);
        first.assert_exhausted();
        drop(registration);

        let second = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            (
                "receipt",
                ScriptResult::Ok(json!({
                    "status": "success",
                    "transactionHash": format!("0x{}", "11".repeat(32)),
                    "blockNumber": "50000000",
                    "blockHash": format!("0x{}", "22".repeat(32)),
                    "canonicalBlockHash": format!("0x{}", "22".repeat(32)),
                    "confirmations": "12",
                    "agentId": "42",
                })),
            ),
        ]));
        let mut recovered = scripted_registration(root.path(), second.clone());
        assert!(recovered.maintain(false).await.unwrap().is_empty());
        assert_eq!(recovered.state.phase, RegistrationPhase::ConfirmedIdentity);
        assert_eq!(recovered.state.confirmed_agent_id.as_deref(), Some("42"));
        assert!(!second.calls().iter().any(|call| call == "register"));
        second.assert_exhausted();
    }

    #[tokio::test]
    async fn reorganized_dropped_registration_replays_only_the_persisted_nonce() {
        let root = tempfile::tempdir().unwrap();
        let mut seeded = registration(root.path());
        seeded.prepare_action(PendingAction::Register, 10).unwrap();
        seeded.state.submitted_transaction_nonce = Some("7".to_owned());
        seeded.state.submitted_transaction_hash = Some(format!("0x{}", "11".repeat(32)));
        seeded.state.phase = RegistrationPhase::Submitted;
        seeded.persist(11).unwrap();
        drop(seeded);

        let gateway = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            (
                "receipt",
                ScriptResult::Ok(json!({
                    "status": "success",
                    "transactionHash": format!("0x{}", "11".repeat(32)),
                    "blockNumber": "50000000",
                    "blockHash": format!("0x{}", "22".repeat(32)),
                    "canonicalBlockHash": format!("0x{}", "33".repeat(32)),
                    "confirmations": "12",
                    "agentId": "42",
                })),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("7", "7")),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("7", "7")),
            ),
            ("register", ScriptResult::Ok(submitted_result("44"))),
        ]));
        let mut recovered = scripted_registration(root.path(), gateway.clone());
        recovered.maintain(false).await.unwrap();
        assert_eq!(recovered.state.phase, RegistrationPhase::Submitted);
        let replacement_hash = format!("0x{}", "44".repeat(32));
        assert_eq!(
            recovered.state.submitted_transaction_hash.as_deref(),
            Some(replacement_hash.as_str())
        );
        assert_eq!(
            recovered.state.submitted_transaction_nonce.as_deref(),
            Some("7")
        );
        let nonces = gateway
            .operations()
            .into_iter()
            .filter(|operation| operation.get("type").and_then(Value::as_str) == Some("register"))
            .map(|operation| operation["nonce"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(nonces, ["7"]);
        gateway.assert_exhausted();
    }

    #[tokio::test]
    async fn reverted_registration_is_not_retired_before_canonical_finality() {
        for (canonical_hash, confirmations, expected_phase) in [
            (
                format!("0x{}", "33".repeat(32)),
                "12",
                RegistrationPhase::FailedRecoverable,
            ),
            (
                format!("0x{}", "22".repeat(32)),
                "0",
                RegistrationPhase::Submitted,
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut seeded = registration(root.path());
            seeded.prepare_action(PendingAction::Register, 10).unwrap();
            seeded.state.submitted_transaction_nonce = Some("7".to_owned());
            seeded.state.submitted_transaction_hash = Some(format!("0x{}", "11".repeat(32)));
            seeded.state.phase = RegistrationPhase::Submitted;
            seeded.persist(11).unwrap();
            drop(seeded);

            let mut steps = vec![
                ("inspect_registry", ScriptResult::Ok(json!({}))),
                (
                    "receipt",
                    ScriptResult::Ok(json!({
                        "status": "reverted",
                        "transactionHash": format!("0x{}", "11".repeat(32)),
                        "blockNumber": "50000000",
                        "blockHash": format!("0x{}", "22".repeat(32)),
                        "canonicalBlockHash": canonical_hash,
                        "confirmations": confirmations,
                        "agentId": null,
                    })),
                ),
            ];
            if expected_phase == RegistrationPhase::FailedRecoverable {
                steps.push((
                    "transaction_nonce",
                    ScriptResult::Ok(register_nonce_result("8", "7")),
                ));
            }
            let gateway = Arc::new(ScriptedGateway::new(steps));
            let mut recovered = scripted_registration(root.path(), gateway.clone());
            recovered.maintain(false).await.unwrap();
            assert_eq!(recovered.state.phase, expected_phase);
            assert_eq!(
                recovered.state.submitted_action,
                Some(PendingAction::Register)
            );
            assert_eq!(
                recovered.state.submitted_transaction_nonce.as_deref(),
                Some("7")
            );
            assert!(recovered.state.submitted_transaction_hash.is_some());
            gateway.assert_exhausted();
        }
    }

    #[tokio::test]
    async fn canonical_finalized_revert_retires_registration_intent() {
        let root = tempfile::tempdir().unwrap();
        let mut seeded = registration(root.path());
        seeded.prepare_action(PendingAction::Register, 10).unwrap();
        seeded.state.submitted_transaction_nonce = Some("7".to_owned());
        seeded.state.submitted_transaction_hash = Some(format!("0x{}", "11".repeat(32)));
        seeded.state.phase = RegistrationPhase::Submitted;
        seeded.persist(11).unwrap();
        drop(seeded);

        let gateway = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            (
                "receipt",
                ScriptResult::Ok(json!({
                    "status": "reverted",
                    "transactionHash": format!("0x{}", "11".repeat(32)),
                    "blockNumber": "50000000",
                    "blockHash": format!("0x{}", "22".repeat(32)),
                    "canonicalBlockHash": format!("0x{}", "22".repeat(32)),
                    "confirmations": "12",
                    "agentId": null,
                })),
            ),
        ]));
        let mut recovered = scripted_registration(root.path(), gateway.clone());
        recovered.maintain(false).await.unwrap();
        assert_eq!(recovered.state.phase, RegistrationPhase::FailedRecoverable);
        assert!(recovered.state.submitted_action.is_none());
        assert!(recovered.state.submitted_transaction_nonce.is_none());
        assert!(recovered.state.submitted_transaction_hash.is_none());
        gateway.assert_exhausted();
    }

    #[tokio::test]
    async fn lost_register_response_is_adopted_even_after_identity_transfer() {
        let root = tempfile::tempdir().unwrap();
        let first = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("resolve_inbox", ScriptResult::Ok(canonical_inbox_result())),
            ("discover", ScriptResult::Ok(empty_discovery_result())),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("7", "7")),
            ),
            ("register", ScriptResult::Err("RPC response was lost")),
        ]));
        let mut registration = scripted_registration(root.path(), first);
        registration.maintain(false).await.unwrap();
        assert_eq!(registration.state.phase, RegistrationPhase::Preparing);
        drop(registration);

        let mut matched_registration = empty_discovery_result();
        matched_registration["matchedRegistrationAgentIds"] = json!(["42"]);
        let second = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("discover", ScriptResult::Ok(matched_registration)),
            (
                "inspect_agent",
                ScriptResult::Ok(json!({
                    "owner": "0x3333333333333333333333333333333333333333",
                    "agentURI": "",
                    "agentWallet": Address::ZERO.to_string(),
                    "authorized": false,
                    "allegiance": {"hex": "0x", "utf8": ""},
                    "protocol": {"hex": "0x", "utf8": ""},
                    "tentacleId": {"hex": "0x", "utf8": ""},
                    "declaresTentacleAllegiance": false,
                    "protocolCompatible": false,
                    "walletVerified": false,
                })),
            ),
        ]));
        let mut recovered = scripted_registration(root.path(), second.clone());
        let notifications = recovered.maintain(false).await.unwrap();
        assert_eq!(recovered.state.phase, RegistrationPhase::Suspended);
        assert_eq!(recovered.state.confirmed_agent_id.as_deref(), Some("42"));
        assert!(notifications.is_empty());
        assert!(!second.calls().iter().any(|call| call == "register"));
        second.assert_exhausted();
    }

    #[tokio::test]
    async fn unknown_register_outcome_retries_only_the_persisted_nonce() {
        let root = tempfile::tempdir().unwrap();
        let first = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("resolve_inbox", ScriptResult::Ok(canonical_inbox_result())),
            ("discover", ScriptResult::Ok(empty_discovery_result())),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("7", "7")),
            ),
            ("register", ScriptResult::Err("RPC response was lost")),
        ]));
        let mut registration = scripted_registration(root.path(), first);
        registration.maintain(false).await.unwrap();
        drop(registration);

        let second = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("discover", ScriptResult::Ok(empty_discovery_result())),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("8", "7")),
            ),
            (
                "register",
                ScriptResult::Err("replacement response was lost"),
            ),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("discover", ScriptResult::Ok(empty_discovery_result())),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("8", "7")),
            ),
            ("register", ScriptResult::Ok(submitted_result("11"))),
        ]));
        let mut recovered = scripted_registration(root.path(), second.clone());
        recovered.maintain(false).await.unwrap();
        assert_eq!(
            recovered.state.submitted_action,
            Some(PendingAction::Register)
        );
        assert_eq!(
            recovered.state.submitted_transaction_nonce.as_deref(),
            Some("7")
        );
        recovered.retry().await.unwrap();
        assert_eq!(recovered.state.phase, RegistrationPhase::Submitted);
        let nonces = second
            .operations()
            .into_iter()
            .filter(|operation| operation.get("type").and_then(Value::as_str) == Some("register"))
            .map(|operation| {
                operation
                    .get("nonce")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(nonces, ["7", "7"]);
        second.assert_exhausted();
    }

    #[tokio::test]
    async fn consumed_unknown_register_nonce_is_retired_only_after_same_block_discovery() {
        let root = tempfile::tempdir().unwrap();
        let first = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("resolve_inbox", ScriptResult::Ok(canonical_inbox_result())),
            ("discover", ScriptResult::Ok(empty_discovery_result())),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("7", "7")),
            ),
            ("register", ScriptResult::Err("RPC response was lost")),
        ]));
        let mut registration = scripted_registration(root.path(), first);
        registration.maintain(false).await.unwrap();
        drop(registration);

        let second = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("discover", ScriptResult::Ok(empty_discovery_result())),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("8", "8")),
            ),
        ]));
        let mut recovered = scripted_registration(root.path(), second.clone());
        recovered.maintain(false).await.unwrap();
        assert!(recovered.state.submitted_action.is_none());
        assert!(recovered.state.submitted_transaction_nonce.is_none());
        assert!(!second.calls().iter().any(|call| call == "register"));
        second.assert_exhausted();
    }

    #[tokio::test]
    async fn inconsistent_discovery_and_nonce_heads_retain_unknown_register_guard() {
        let root = tempfile::tempdir().unwrap();
        let first = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("resolve_inbox", ScriptResult::Ok(canonical_inbox_result())),
            ("discover", ScriptResult::Ok(empty_discovery_result())),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("7", "7")),
            ),
            ("register", ScriptResult::Err("RPC response was lost")),
        ]));
        let mut registration = scripted_registration(root.path(), first);
        registration.maintain(false).await.unwrap();
        drop(registration);

        let mut inconsistent_nonce = register_nonce_result("8", "8");
        inconsistent_nonce["observedBlockNumber"] = Value::String("50000001".to_owned());
        let second = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("discover", ScriptResult::Ok(empty_discovery_result())),
            ("transaction_nonce", ScriptResult::Ok(inconsistent_nonce)),
        ]));
        let mut recovered = scripted_registration(root.path(), second.clone());
        recovered.maintain(false).await.unwrap();
        assert_eq!(
            recovered.state.submitted_action,
            Some(PendingAction::Register)
        );
        assert_eq!(
            recovered.state.submitted_transaction_nonce.as_deref(),
            Some("7")
        );
        assert!(recovered.state.action_id.is_some());
        assert!(!second.calls().iter().any(|call| call == "register"));
        second.assert_exhausted();
    }

    #[tokio::test]
    async fn lost_follow_up_response_replays_only_persisted_nonce_and_recovers_when_visible() {
        let root = tempfile::tempdir().unwrap();
        let mut seeded = registration(root.path());
        seeded.state.selected_agent_id = Some("42".to_owned());
        seeded.state.confirmed_agent_id = Some("42".to_owned());
        let final_uri = seeded.build_agent_uri("42").unwrap();
        let action = PendingAction::SetMetadata {
            key: ALLEGIANCE_KEY.to_owned(),
            value: ALLEGIANCE_VALUE.to_owned(),
        };
        seeded.prepare_action(action.clone(), 10).unwrap();
        seeded.state.submitted_transaction_nonce = Some("9".to_owned());
        seeded.persist(11).unwrap();
        drop(seeded);

        let not_visible = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            (
                "inspect_agent",
                ScriptResult::Ok(json!({
                    "owner": wallet().to_string(),
                    "agentURI": final_uri.clone(),
                    "agentWallet": wallet().to_string(),
                    "authorized": true,
                    "allegiance": {"hex": "0x", "utf8": ""},
                    "protocol": {"hex": utf8_hex(PROTOCOL_VALUE), "utf8": PROTOCOL_VALUE},
                    "tentacleId": {"hex": utf8_hex("tentacle-independent"), "utf8": "tentacle-independent"},
                    "declaresTentacleAllegiance": false,
                    "protocolCompatible": true,
                    "walletVerified": true,
                })),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("10", "9")),
            ),
            ("set_metadata", ScriptResult::Err("response lost again")),
        ]));
        let mut waiting = scripted_registration(root.path(), not_visible.clone());
        waiting.maintain(false).await.unwrap();
        assert_eq!(waiting.state.phase, RegistrationPhase::Preparing);
        assert_eq!(waiting.state.submitted_action, Some(action.clone()));
        assert_eq!(
            waiting.state.submitted_transaction_nonce.as_deref(),
            Some("9")
        );
        let replay_nonces = not_visible
            .operations()
            .into_iter()
            .filter(|operation| {
                operation.get("type").and_then(Value::as_str) == Some("set_metadata")
            })
            .map(|operation| operation["nonce"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(replay_nonces, ["9"]);
        not_visible.assert_exhausted();
        drop(waiting);

        let visible = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            (
                "inspect_agent",
                ScriptResult::Ok(json!({
                    "owner": wallet().to_string(),
                    "agentURI": final_uri,
                    "agentWallet": wallet().to_string(),
                    "authorized": true,
                    "allegiance": {"hex": utf8_hex(ALLEGIANCE_VALUE), "utf8": ALLEGIANCE_VALUE},
                    "protocol": {"hex": utf8_hex(PROTOCOL_VALUE), "utf8": PROTOCOL_VALUE},
                    "tentacleId": {"hex": utf8_hex("tentacle-independent"), "utf8": "tentacle-independent"},
                    "declaresTentacleAllegiance": true,
                    "protocolCompatible": true,
                    "walletVerified": true,
                })),
            ),
        ]));
        let mut recovered = scripted_registration(root.path(), visible.clone());
        let notices = recovered.maintain(false).await.unwrap();
        assert_eq!(recovered.state.phase, RegistrationPhase::Active);
        assert!(recovered.state.submitted_action.is_none());
        assert!(recovered.state.submitted_transaction_nonce.is_none());
        assert_eq!(notices.len(), 1);
        assert!(!visible.calls().iter().any(|call| call == "set_metadata"));
        visible.assert_exhausted();
    }

    #[tokio::test]
    async fn a_reorganized_receipt_is_not_accepted_or_resubmitted() {
        let root = tempfile::tempdir().unwrap();
        let first = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("resolve_inbox", ScriptResult::Ok(canonical_inbox_result())),
            ("discover", ScriptResult::Ok(empty_discovery_result())),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("7", "7")),
            ),
            ("register", ScriptResult::Ok(submitted_result("11"))),
        ]));
        let mut registration = scripted_registration(root.path(), first);
        registration.maintain(false).await.unwrap();
        drop(registration);

        let second = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            (
                "receipt",
                ScriptResult::Ok(json!({
                    "status": "success",
                    "transactionHash": format!("0x{}", "11".repeat(32)),
                    "blockNumber": "50000000",
                    "blockHash": format!("0x{}", "22".repeat(32)),
                    "canonicalBlockHash": format!("0x{}", "33".repeat(32)),
                    "confirmations": "12",
                    "agentId": "42",
                })),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("8", "7")),
            ),
        ]));
        let mut recovered = scripted_registration(root.path(), second.clone());
        recovered.maintain(false).await.unwrap();
        assert_eq!(recovered.state.phase, RegistrationPhase::FailedRecoverable);
        assert!(recovered.state.submitted_transaction_hash.is_some());
        assert!(recovered.state.confirmed_agent_id.is_none());
        assert!(!second.calls().iter().any(|call| call == "register"));
        second.assert_exhausted();
    }

    #[test]
    fn every_transaction_intent_and_hash_survives_a_restart() {
        let actions = [
            PendingAction::Register,
            PendingAction::SetAgentWallet,
            PendingAction::SetAgentUri,
            PendingAction::SetMetadata {
                key: ALLEGIANCE_KEY.to_owned(),
                value: ALLEGIANCE_VALUE.to_owned(),
            },
            PendingAction::SetMetadata {
                key: PROTOCOL_KEY.to_owned(),
                value: PROTOCOL_VALUE.to_owned(),
            },
            PendingAction::SetMetadata {
                key: TENTACLE_ID_KEY.to_owned(),
                value: "tentacle-independent".to_owned(),
            },
        ];
        for (index, action) in actions.into_iter().enumerate() {
            let root = tempfile::tempdir().unwrap();
            let mut instance = registration(root.path());
            instance
                .prepare_action(action.clone(), index as u64 + 1)
                .unwrap();
            let hash = format!("0x{:064x}", index + 1);
            instance.state.submitted_transaction_nonce = Some(index.to_string());
            instance.state.submitted_transaction_hash = Some(hash.clone());
            instance.state.phase = RegistrationPhase::Submitted;
            instance.persist(index as u64 + 2).unwrap();
            drop(instance);
            let recovered = registration(root.path());
            assert_eq!(recovered.state.submitted_action, Some(action));
            assert_eq!(recovered.state.submitted_transaction_hash, Some(hash));
            assert_eq!(
                recovered.state.submitted_transaction_nonce,
                Some(index.to_string())
            );
            assert_eq!(recovered.state.phase, RegistrationPhase::Submitted);
        }
    }

    #[tokio::test]
    async fn complete_registration_resumes_after_every_submitted_and_confirmed_stage() {
        let root = tempfile::tempdir().unwrap();
        let profile_root = tempfile::tempdir().unwrap();
        let final_uri = registration(profile_root.path())
            .build_agent_uri("42")
            .unwrap();
        let empty = inspected_agent("", Address::ZERO, "", "", "");
        let wallet_only = inspected_agent("", wallet(), "", "", "");
        let profile_only = inspected_agent(&final_uri, wallet(), "", "", "");
        let allegiance = inspected_agent(&final_uri, wallet(), ALLEGIANCE_VALUE, "", "");
        let protocol = inspected_agent(&final_uri, wallet(), ALLEGIANCE_VALUE, PROTOCOL_VALUE, "");
        let complete = inspected_agent(
            &final_uri,
            wallet(),
            ALLEGIANCE_VALUE,
            PROTOCOL_VALUE,
            "tentacle-independent",
        );
        let gateway = Arc::new(ScriptedGateway::new(vec![
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("resolve_inbox", ScriptResult::Ok(canonical_inbox_result())),
            ("discover", ScriptResult::Ok(empty_discovery_result())),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("7", "7")),
            ),
            ("register", ScriptResult::Ok(submitted_result("11"))),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            (
                "receipt",
                ScriptResult::Ok(receipt_result("11", Some("42"))),
            ),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("inspect_agent", ScriptResult::Ok(empty)),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("8", "8")),
            ),
            (
                "set_agent_wallet",
                ScriptResult::Ok(submitted_result_with_nonce("12", "8")),
            ),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("receipt", ScriptResult::Ok(receipt_result("12", None))),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("inspect_agent", ScriptResult::Ok(wallet_only)),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("9", "9")),
            ),
            (
                "set_agent_uri",
                ScriptResult::Ok(submitted_result_with_nonce("13", "9")),
            ),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("receipt", ScriptResult::Ok(receipt_result("13", None))),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("inspect_agent", ScriptResult::Ok(profile_only)),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("10", "10")),
            ),
            (
                "set_metadata",
                ScriptResult::Ok(submitted_result_with_nonce("14", "10")),
            ),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("receipt", ScriptResult::Ok(receipt_result("14", None))),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("inspect_agent", ScriptResult::Ok(allegiance)),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("11", "11")),
            ),
            (
                "set_metadata",
                ScriptResult::Ok(submitted_result_with_nonce("15", "11")),
            ),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("receipt", ScriptResult::Ok(receipt_result("15", None))),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("inspect_agent", ScriptResult::Ok(protocol)),
            (
                "funding_estimate",
                ScriptResult::Ok(adequate_funding_result()),
            ),
            (
                "transaction_nonce",
                ScriptResult::Ok(register_nonce_result("12", "12")),
            ),
            (
                "set_metadata",
                ScriptResult::Ok(submitted_result_with_nonce("16", "12")),
            ),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("receipt", ScriptResult::Ok(receipt_result("16", None))),
            ("inspect_registry", ScriptResult::Ok(json!({}))),
            ("inspect_agent", ScriptResult::Ok(complete)),
        ]));

        let expected_submissions = [
            PendingAction::Register,
            PendingAction::SetAgentWallet,
            PendingAction::SetAgentUri,
            PendingAction::SetMetadata {
                key: ALLEGIANCE_KEY.to_owned(),
                value: ALLEGIANCE_VALUE.to_owned(),
            },
            PendingAction::SetMetadata {
                key: PROTOCOL_KEY.to_owned(),
                value: PROTOCOL_VALUE.to_owned(),
            },
            PendingAction::SetMetadata {
                key: TENTACLE_ID_KEY.to_owned(),
                value: "tentacle-independent".to_owned(),
            },
        ];
        for expected in expected_submissions {
            let mut submitting = scripted_registration(root.path(), gateway.clone());
            submitting.maintain(false).await.unwrap();
            assert_eq!(submitting.state.phase, RegistrationPhase::Submitted);
            assert_eq!(submitting.state.submitted_action, Some(expected));
            drop(submitting);

            let mut confirming = scripted_registration(root.path(), gateway.clone());
            confirming.maintain(false).await.unwrap();
            assert!(confirming.state.submitted_transaction_hash.is_none());
            assert!(confirming.state.submitted_action.is_none());
            drop(confirming);
        }

        let mut verifying = scripted_registration(root.path(), gateway.clone());
        let notices = verifying.maintain(false).await.unwrap();
        assert_eq!(verifying.state.phase, RegistrationPhase::Active);
        assert_eq!(verifying.state.confirmed_agent_id.as_deref(), Some("42"));
        assert_eq!(notices.len(), 1);
        gateway.assert_exhausted();
    }

    #[test]
    fn profile_and_metadata_bounds_reject_hostile_input() {
        let root = tempfile::tempdir().unwrap();
        let config = RegistrationConfig {
            initial_public_name: Some("bad\nname".into()),
            ..RegistrationConfig::default()
        };
        assert!(
            TentacleRegistration::open(
                root.path(),
                "tentacle-independent",
                wallet(),
                config,
                Arc::new(UnusedGateway)
            )
            .is_err()
        );
        assert!(validate_agent_id("01").is_err());
        assert!(validate_hash("0x00", "hash").is_err());
    }

    #[test]
    fn transfer_or_cleared_wallet_is_suspended_not_ranked_by_owner_fallback() {
        let zero = Address::ZERO.to_string();
        let inspected = json!({
            "owner": wallet().to_string(), "agentURI": "data:application/json;base64,e30=",
            "agentWallet": zero, "authorized": true,
            "allegiance": {"hex": utf8_hex(ALLEGIANCE_VALUE), "utf8": ALLEGIANCE_VALUE},
            "protocol": {"hex": utf8_hex(PROTOCOL_VALUE), "utf8": PROTOCOL_VALUE},
            "tentacleId": {"hex": utf8_hex("tentacle-independent"), "utf8": "tentacle-independent"},
            "declaresTentacleAllegiance": true, "protocolCompatible": true, "walletVerified": false
        });
        let verified = parse_verified(&inspected, wallet(), 1).unwrap();
        assert!(!verified.wallet_verified);
        assert_eq!(verified.agent_wallet, Address::ZERO.to_string());
    }

    #[test]
    fn helper_cannot_claim_that_another_agent_wallet_is_verified() {
        let inspected = json!({
            "owner": wallet().to_string(), "agentURI": "",
            "agentWallet": "0x2222222222222222222222222222222222222222",
            "authorized": true,
            "allegiance": {"hex": "0x", "utf8": ""},
            "protocol": {"hex": "0x", "utf8": ""},
            "tentacleId": {"hex": "0x", "utf8": ""},
            "declaresTentacleAllegiance": false,
            "protocolCompatible": false,
            "walletVerified": true,
        });
        assert!(parse_verified(&inspected, wallet(), 1).is_err());
    }

    #[test]
    fn rust_revalidates_the_typed_signer_transaction_binding() {
        assert_eq!(
            validate_write_result(&submitted_result("11"), wallet()).unwrap(),
            format!("0x{}", "11".repeat(32))
        );
        for changed in [
            json!({"transactionHash": format!("0x{}", "11".repeat(32)), "wallet": "0x2222222222222222222222222222222222222222", "chainId": BASE_MAINNET_CHAIN_ID, "registry": IDENTITY_REGISTRY, "valueWei": "0"}),
            json!({"transactionHash": format!("0x{}", "11".repeat(32)), "wallet": wallet().to_string(), "chainId": 1, "registry": IDENTITY_REGISTRY, "valueWei": "0"}),
            json!({"transactionHash": format!("0x{}", "11".repeat(32)), "wallet": wallet().to_string(), "chainId": BASE_MAINNET_CHAIN_ID, "registry": "0x2222222222222222222222222222222222222222", "valueWei": "0"}),
            json!({"transactionHash": format!("0x{}", "11".repeat(32)), "wallet": wallet().to_string(), "chainId": BASE_MAINNET_CHAIN_ID, "registry": IDENTITY_REGISTRY, "valueWei": "1"}),
        ] {
            assert!(validate_write_result(&changed, wallet()).is_err());
        }
    }

    #[test]
    fn gateway_has_no_generic_signing_operation() {
        let source = include_str!("../../agent/src/erc8004.ts");
        assert!(!source.contains("sign_arbitrary"));
        assert!(!source.contains("type: \"send_transaction\""));
        assert!(source.contains("type: \"set_agent_wallet\""));
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
    }
}
