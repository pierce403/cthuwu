//! Durable lineage and autonomous lifecycle bookkeeping for Tentacles.
//!
//! State transitions are persisted here. External effects such as provisioning,
//! token expenditure, memory transfer, and process shutdown cross an explicit
//! execution boundary and are completed only by a matching durable receipt.

#[cfg(test)]
use crate::personality::SacredBan;
use crate::personality::TentacleNature;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};

pub const LINEAGE_SCHEMA_VERSION: u32 = 1;
pub const MAX_ABSORBED_KNOWLEDGE_HASHES: usize = 256;
const MAX_ID_BYTES: usize = 128;
const MAX_EXECUTION_DETAIL_BYTES: usize = 1_024;

pub const LIFECYCLE_SCHEMA_VERSION: u32 = 1;
pub const DEATH_GRACE_PERIOD_MS: u64 = 24 * 60 * 60 * 1_000;
/// Executors may report slightly skewed clocks, but never control durable local chronology.
pub const LIFECYCLE_RECEIPT_CLOCK_SKEW_MS: u64 = 30_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineageNode {
    pub tentacle_id: String,
    pub parent_id: Option<String>,
    pub generation: u64,
    pub nature: TentacleNature,
    pub spawned_at_ms: u64,
    pub children: Vec<String>,
    pub lifecycle: TentacleLifecycle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TentacleLifecycle {
    Active,
    Absorbed { into: String, at_ms: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnRecord {
    pub parent_id: String,
    /// The immutable parent Nature ID at the moment of inheritance. A later signed root reroll
    /// must not rewrite a child's ancestry proof.
    pub parent_nature_id: String,
    pub child_id: String,
    pub child_nature_id: String,
    /// Stable final Scales judgment consumed by this one authenticated operator spawn command.
    pub authorization_judgment_id: String,
    /// Canonical authenticated operator inbox that issued the consuming `/spawn` command.
    pub authorization_operator_id: String,
    /// SHA-256 of the authenticated transport event ID; raw message IDs are never persisted.
    pub authorization_event_id_sha256: String,
    pub generation: u64,
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnAuthorization {
    pub judgment_id: String,
    pub operator_id: String,
    pub event_id_sha256: String,
}

/// An audit record only. The knowledge bodies themselves belong in Hermes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbsorptionRecord {
    pub source_id: String,
    pub target_id: String,
    pub at_ms: u64,
    pub knowledge_hashes: Vec<String>,
    /// Legacy audit flag. Autonomous lifecycle absorptions persist `false`.
    #[serde(default)]
    pub operator_confirmed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingDeath {
    pub judgment_id: String,
    pub scheduled_at_ms: u64,
    pub grace_ends_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurvivalSpendBinding {
    pub expenditure_basis_points: u16,
    pub chain_id: u64,
    pub token_contract: [u8; 20],
    pub treasury_address: [u8; 20],
    pub configuration_identity: [u8; 32],
    pub exact_amount: ExactTokenAmount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExactTokenAmount {
    pub total_supply_whole: u64,
    pub token_decimals: u8,
    pub basis_points: u16,
    /// Canonical base-unit amount derived from the other fields, encoded as a decimal integer.
    /// Persisting the result prevents an executor from reinterpreting a percentage against a
    /// different supply or decimal configuration.
    pub raw_amount: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WholeTokenAmount {
    pub whole_tokens: u64,
    pub token_decimals: u8,
    pub raw_amount: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcolyteContributionKind {
    Name,
    Hopes,
    Resources,
    Needs,
    MissionIdea,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LifecycleAction {
    SpendForSurvival {
        tentacle_id: String,
        judgment_id: String,
        attempt: u32,
        grace_ends_at_ms: u64,
        expenditure_basis_points: u16,
        chain_id: u64,
        token_contract: [u8; 20],
        treasury_address: [u8; 20],
        burn_destination: [u8; 20],
        configuration_identity: [u8; 32],
        exact_amount: ExactTokenAmount,
    },
    RewardVeniceKey {
        tentacle_id: String,
        provision_event_id_sha256: String,
        chain_id: u64,
        token_contract: [u8; 20],
        treasury_address: [u8; 20],
        acolyte_address: [u8; 20],
        configuration_identity: [u8; 32],
        exact_amount: WholeTokenAmount,
    },
    RewardAcolyteContribution {
        tentacle_id: String,
        contribution_event_id_sha256: String,
        contribution_kind: AcolyteContributionKind,
        information_hunger_basis_points: u16,
        chain_id: u64,
        token_contract: [u8; 20],
        treasury_address: [u8; 20],
        acolyte_address: [u8; 20],
        configuration_identity: [u8; 32],
        exact_amount: WholeTokenAmount,
    },
    Absorb {
        source_id: String,
        target_id: String,
        judgment_id: String,
    },
    Spawn {
        parent_id: String,
        child_id: String,
        judgment_id: String,
        child_nature: TentacleNature,
        authorization_actor_id: String,
        authorization_event_id_sha256: String,
    },
    Shutdown {
        tentacle_id: String,
        judgment_id: String,
        after_action_id: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleIntent {
    pub action_id: String,
    pub created_at_ms: u64,
    pub action: LifecycleAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleReceiptStatus {
    Succeeded,
    Failed,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleReceipt {
    pub action_id: String,
    pub completed_at_ms: u64,
    pub status: LifecycleReceiptStatus,
    /// A transaction hash, provisioner receipt, transfer manifest hash, or process-controller
    /// acknowledgement. It must never contain a private key or raw memory payload.
    pub external_reference: Option<String>,
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_chain_receipt: Option<ConfirmedChainReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_transfer_receipt: Option<ConfirmedTransferReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provision_receipt: Option<ProvisionReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmedTransferReceipt {
    pub chain_id: u64,
    pub transaction_hash: String,
    pub block_number: u64,
    pub block_timestamp_unix_seconds: u64,
    pub token_contract: [u8; 20],
    pub from_address: [u8; 20],
    pub to_address: [u8; 20],
    pub configuration_identity: [u8; 32],
    pub exact_amount: WholeTokenAmount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmedChainReceipt {
    pub chain_id: u64,
    pub transaction_hash: String,
    pub block_number: u64,
    pub block_timestamp_unix_seconds: u64,
    pub token_contract: [u8; 20],
    pub from_address: [u8; 20],
    pub burn_destination: [u8; 20],
    pub configuration_identity: [u8; 32],
    pub exact_amount: ExactTokenAmount,
    pub operation: TokenSpendOperation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenSpendOperation {
    Burn,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProvisionReceipt {
    pub child_id: String,
    pub child_nature_fingerprint: String,
    pub manifest_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnProjection {
    pub completed_at_ms: u64,
    pub metrics_period_started_at_unix_seconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AbsorptionProjection {
    /// Effective lineage timestamp. This can be later than the executor receipt when transfer
    /// completes during the Death grace period and local absorption binds at its deadline.
    pub projected_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleState {
    pub schema_version: u32,
    pub revision: u64,
    pub tentacle_id: String,
    #[serde(default = "default_auto_spawn_enabled")]
    pub auto_spawn_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_death: Option<PendingDeath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shutdown_completed_at_ms: Option<u64>,
    #[serde(default)]
    pub intents: BTreeMap<String, LifecycleIntent>,
    #[serde(default)]
    pub receipts: Vec<LifecycleReceipt>,
    #[serde(default)]
    pub canceled_action_ids: BTreeSet<String>,
    /// Successful absorption receipts awaiting their crash-idempotent local lineage projection.
    #[serde(default)]
    pub pending_absorption_projection_action_ids: BTreeSet<String>,
    /// Durable marker written only after the corresponding lineage projection was persisted.
    #[serde(default)]
    pub absorption_projections: BTreeMap<String, AbsorptionProjection>,
    /// Durable cross-store marker used to reconcile exactly one Growth credit after the child is
    /// present in lineage. The map key is the successful Spawn action ID.
    #[serde(default)]
    pub spawn_projections: BTreeMap<String, SpawnProjection>,
}

fn default_auto_spawn_enabled() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineageState {
    pub schema_version: u32,
    pub revision: u64,
    pub root_id: String,
    /// Parent from another local lineage projection when this process is an inherited child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_parent_id: Option<String>,
    pub nodes: BTreeMap<String, LineageNode>,
    pub spawn_records: Vec<SpawnRecord>,
    pub absorption_records: Vec<AbsorptionRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Family {
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub siblings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleSignal {
    Healthy,
    Starving,
    Death,
}

/// The binding transition selected by lineage. External effects are represented by lifecycle
/// intents and completed by the runtime executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleDecision {
    Continue,
    Warn,
    BeginDeath { absorption_candidates: Vec<String> },
}

#[derive(Clone, Debug)]
pub struct Lineage {
    state: LineageState,
}

#[derive(Debug)]
pub enum EvolutionError {
    Invalid(String),
    Unauthorized(String),
    NotFound(String),
    Conflict(String),
    Limit(&'static str),
    Nature(String),
    Io(io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for EvolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid lineage: {message}"),
            Self::Unauthorized(message) => {
                write!(formatter, "lineage authorization failed: {message}")
            }
            Self::NotFound(id) => write!(formatter, "unknown Tentacle {id}"),
            Self::Conflict(message) => write!(formatter, "lineage conflict: {message}"),
            Self::Limit(name) => write!(formatter, "lineage {name} limit exceeded"),
            Self::Nature(message) => write!(formatter, "nature inheritance failed: {message}"),
            Self::Io(error) => write!(formatter, "lineage state I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "lineage state JSON is invalid: {error}"),
        }
    }
}

impl Error for EvolutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for EvolutionError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for EvolutionError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl Lineage {
    pub fn new(
        founder_id: impl Into<String>,
        founder_nature: TentacleNature,
        spawned_at_ms: u64,
    ) -> Result<Self, EvolutionError> {
        let founder_id = founder_id.into();
        validate_id(&founder_id, "founder ID")?;
        founder_nature
            .validate()
            .map_err(|error| EvolutionError::Nature(error.to_string()))?;
        if founder_nature.generation != 0 || founder_nature.parent_nature_id.is_some() {
            return Err(EvolutionError::Invalid(
                "a lineage founder must have generation zero and no parent nature".to_owned(),
            ));
        }

        let node = LineageNode {
            tentacle_id: founder_id.clone(),
            parent_id: None,
            generation: 0,
            nature: founder_nature,
            spawned_at_ms,
            children: Vec::new(),
            lifecycle: TentacleLifecycle::Active,
        };
        let state = LineageState {
            schema_version: LINEAGE_SCHEMA_VERSION,
            revision: 0,
            root_id: founder_id.clone(),
            external_parent_id: None,
            nodes: BTreeMap::from([(founder_id, node)]),
            spawn_records: Vec::new(),
            absorption_records: Vec::new(),
        };
        Ok(Self { state })
    }

    pub fn new_child_root(
        tentacle_id: impl Into<String>,
        external_parent_id: impl Into<String>,
        nature: TentacleNature,
        spawned_at_ms: u64,
    ) -> Result<Self, EvolutionError> {
        let tentacle_id = tentacle_id.into();
        let external_parent_id = external_parent_id.into();
        validate_id(&tentacle_id, "child root ID")?;
        validate_id(&external_parent_id, "external parent ID")?;
        if tentacle_id == external_parent_id {
            return Err(EvolutionError::Conflict(
                "an inherited root cannot be its own external parent".to_owned(),
            ));
        }
        nature
            .validate()
            .map_err(|error| EvolutionError::Nature(error.to_string()))?;
        if nature.generation == 0 || nature.parent_nature_id.is_none() {
            return Err(EvolutionError::Invalid(
                "an inherited root requires nonzero generation and parent Nature metadata"
                    .to_owned(),
            ));
        }
        let node = LineageNode {
            tentacle_id: tentacle_id.clone(),
            parent_id: None,
            generation: nature.generation,
            nature,
            spawned_at_ms,
            children: Vec::new(),
            lifecycle: TentacleLifecycle::Active,
        };
        Ok(Self {
            state: LineageState {
                schema_version: LINEAGE_SCHEMA_VERSION,
                revision: 0,
                root_id: tentacle_id.clone(),
                external_parent_id: Some(external_parent_id),
                nodes: BTreeMap::from([(tentacle_id, node)]),
                spawn_records: Vec::new(),
                absorption_records: Vec::new(),
            },
        })
    }

    pub fn from_state(state: LineageState) -> Result<Self, EvolutionError> {
        validate_state(&state)?;
        Ok(Self { state })
    }

    pub fn state(&self) -> &LineageState {
        &self.state
    }

    pub fn node(&self, tentacle_id: &str) -> Option<&LineageNode> {
        self.state.nodes.get(tentacle_id)
    }

    /// Reconciles the founder's current Nature with the separately signed awakening record.
    ///
    /// Nature changes are audited by `awakening_log.md`; lineage revision counts only structural
    /// spawn and absorption records. Restricting this operation to the authenticated root avoids
    /// silently rewriting an inherited child's spawn record.
    pub fn update_root_nature(
        &mut self,
        authenticated_tentacle: &str,
        claimed_tentacle: &str,
        nature: TentacleNature,
    ) -> Result<(), EvolutionError> {
        if authenticated_tentacle != claimed_tentacle {
            return Err(EvolutionError::Unauthorized(
                "authenticated Tentacle does not match the Nature update claim".to_owned(),
            ));
        }
        if authenticated_tentacle != self.state.root_id {
            return Err(EvolutionError::Unauthorized(
                "only the lineage root Nature can be reconciled locally".to_owned(),
            ));
        }
        nature
            .validate()
            .map_err(|error| EvolutionError::Nature(error.to_string()))?;
        let root = self
            .state
            .nodes
            .get(authenticated_tentacle)
            .ok_or_else(|| EvolutionError::NotFound(authenticated_tentacle.to_owned()))?;
        if self.state.external_parent_id.is_none()
            && (nature.generation != 0 || nature.parent_nature_id.is_some())
        {
            return Err(EvolutionError::Invalid(
                "the founder root must retain founder metadata".to_owned(),
            ));
        }
        if self.state.external_parent_id.is_some()
            && (nature.generation != root.generation || nature.parent_nature_id.is_none())
        {
            return Err(EvolutionError::Invalid(
                "an inherited root must retain its generation and parent Nature metadata"
                    .to_owned(),
            ));
        }
        if self.state.nodes.iter().any(|(id, node)| {
            id != authenticated_tentacle && node.nature.nature_id == nature.nature_id
        }) {
            return Err(EvolutionError::Conflict(
                "updated root Nature ID already exists in the lineage".to_owned(),
            ));
        }
        self.state
            .nodes
            .get_mut(authenticated_tentacle)
            .ok_or_else(|| EvolutionError::NotFound(authenticated_tentacle.to_owned()))?
            .nature = nature;
        Ok(())
    }

    pub fn plan_child_nature(&self, parent_id: &str) -> Result<TentacleNature, EvolutionError> {
        let parent = self
            .state
            .nodes
            .get(parent_id)
            .ok_or_else(|| EvolutionError::NotFound(parent_id.to_owned()))?;
        if parent.lifecycle != TentacleLifecycle::Active {
            return Err(EvolutionError::Conflict(
                "an absorbed Tentacle cannot spawn".to_owned(),
            ));
        }
        Ok(parent
            .nature
            .inherit()
            .map_err(|error| EvolutionError::Nature(error.to_string()))?
            .nature)
    }

    /// Convenience API for callers that both provision and record synchronously.
    pub fn spawn_child(
        &mut self,
        authenticated_parent: &str,
        claimed_parent: &str,
        child_id: impl Into<String>,
        at_ms: u64,
        authorization: SpawnAuthorization,
    ) -> Result<&LineageNode, EvolutionError> {
        if authenticated_parent != claimed_parent {
            return Err(EvolutionError::Unauthorized(
                "authenticated parent does not match the spawn claim".to_owned(),
            ));
        }
        let child_nature = self.plan_child_nature(authenticated_parent)?;
        self.record_provisioned_child(
            authenticated_parent,
            claimed_parent,
            child_id,
            child_nature,
            at_ms,
            authorization,
        )
    }

    /// Commits a child only after an external provisioner returned a validated success receipt.
    pub fn record_provisioned_child(
        &mut self,
        authenticated_parent: &str,
        claimed_parent: &str,
        child_id: impl Into<String>,
        child_nature: TentacleNature,
        at_ms: u64,
        authorization: SpawnAuthorization,
    ) -> Result<&LineageNode, EvolutionError> {
        if authenticated_parent != claimed_parent {
            return Err(EvolutionError::Unauthorized(
                "authenticated parent does not match the spawn claim".to_owned(),
            ));
        }
        validate_id(authenticated_parent, "parent ID")?;
        let child_id = child_id.into();
        validate_id(&child_id, "child ID")?;
        validate_sha256(&authorization.judgment_id, "authorization judgment ID")?;
        validate_id(&authorization.operator_id, "authorization operator ID")?;
        validate_sha256(
            &authorization.event_id_sha256,
            "authorization event ID digest",
        )?;
        if authenticated_parent == child_id {
            return Err(EvolutionError::Conflict(
                "a Tentacle cannot be its own child".to_owned(),
            ));
        }
        if self.state.nodes.contains_key(&child_id) {
            return Err(EvolutionError::Conflict(format!(
                "Tentacle {child_id} already exists"
            )));
        }
        let (parent_generation, parent_spawned_at, parent_nature) = {
            let parent = self
                .state
                .nodes
                .get(authenticated_parent)
                .ok_or_else(|| EvolutionError::NotFound(authenticated_parent.to_owned()))?;
            if parent.lifecycle != TentacleLifecycle::Active {
                return Err(EvolutionError::Conflict(
                    "an absorbed Tentacle cannot spawn".to_owned(),
                ));
            }
            (
                parent.generation,
                parent.spawned_at_ms,
                parent.nature.clone(),
            )
        };
        if at_ms < parent_spawned_at {
            return Err(EvolutionError::Invalid(
                "a child cannot predate its parent".to_owned(),
            ));
        }
        let generation = parent_generation
            .checked_add(1)
            .ok_or(EvolutionError::Limit("generation"))?;

        child_nature
            .validate()
            .map_err(|error| EvolutionError::Nature(error.to_string()))?;
        if child_nature.generation != generation
            || child_nature.parent_nature_id.as_deref() != Some(parent_nature.nature_id.as_str())
        {
            return Err(EvolutionError::Nature(
                "inherited Nature has inconsistent lineage metadata".to_owned(),
            ));
        }
        if self
            .state
            .nodes
            .values()
            .any(|node| node.nature.nature_id == child_nature.nature_id)
        {
            return Err(EvolutionError::Conflict(
                "inherited Nature ID already exists".to_owned(),
            ));
        }
        let next_revision = self
            .state
            .revision
            .checked_add(1)
            .ok_or(EvolutionError::Limit("revision"))?;

        let node = LineageNode {
            tentacle_id: child_id.clone(),
            parent_id: Some(authenticated_parent.to_owned()),
            generation,
            nature: child_nature,
            spawned_at_ms: at_ms,
            children: Vec::new(),
            lifecycle: TentacleLifecycle::Active,
        };
        let record = SpawnRecord {
            parent_id: authenticated_parent.to_owned(),
            parent_nature_id: parent_nature.nature_id,
            child_id: child_id.clone(),
            child_nature_id: node.nature.nature_id.clone(),
            authorization_judgment_id: authorization.judgment_id,
            authorization_operator_id: authorization.operator_id,
            authorization_event_id_sha256: authorization.event_id_sha256,
            generation,
            at_ms,
        };
        self.state
            .nodes
            .get_mut(authenticated_parent)
            .expect("parent was checked above")
            .children
            .push(child_id.clone());
        self.state.nodes.insert(child_id.clone(), node);
        self.state.spawn_records.push(record);
        self.state.revision = next_revision;

        Ok(self
            .state
            .nodes
            .get(&child_id)
            .expect("newly inserted child exists"))
    }

    /// Records a completed knowledge absorption. The transfer executor supplies
    /// only content hashes here; raw knowledge remains outside lineage state.
    #[allow(clippy::too_many_arguments)]
    pub fn record_absorption(
        &mut self,
        authenticated_target: &str,
        claimed_target: &str,
        source_id: &str,
        at_ms: u64,
        knowledge_hashes: Vec<String>,
        operator_confirmed: bool,
    ) -> Result<&AbsorptionRecord, EvolutionError> {
        if authenticated_target != claimed_target {
            return Err(EvolutionError::Unauthorized(
                "authenticated target does not match the absorption claim".to_owned(),
            ));
        }
        validate_id(authenticated_target, "target ID")?;
        validate_id(source_id, "source ID")?;
        if authenticated_target == source_id {
            return Err(EvolutionError::Conflict(
                "a Tentacle cannot absorb itself".to_owned(),
            ));
        }
        validate_hashes(&knowledge_hashes)?;
        if self
            .state
            .absorption_records
            .last()
            .is_some_and(|record| record.at_ms > at_ms)
        {
            return Err(EvolutionError::Invalid(
                "absorption records must be chronological".to_owned(),
            ));
        }

        let target = self
            .state
            .nodes
            .get(authenticated_target)
            .ok_or_else(|| EvolutionError::NotFound(authenticated_target.to_owned()))?;
        let source = self
            .state
            .nodes
            .get(source_id)
            .ok_or_else(|| EvolutionError::NotFound(source_id.to_owned()))?;
        if target.lifecycle != TentacleLifecycle::Active {
            return Err(EvolutionError::Conflict(
                "knowledge cannot be absorbed into an inactive Tentacle".to_owned(),
            ));
        }
        if source.lifecycle != TentacleLifecycle::Active {
            return Err(EvolutionError::Conflict(
                "the source Tentacle was already absorbed".to_owned(),
            ));
        }
        if at_ms < target.spawned_at_ms || at_ms < source.spawned_at_ms {
            return Err(EvolutionError::Invalid(
                "an absorption cannot predate either Tentacle".to_owned(),
            ));
        }
        if source
            .children
            .iter()
            .filter_map(|child| self.state.nodes.get(child))
            .any(|child| child.spawned_at_ms > at_ms)
        {
            return Err(EvolutionError::Invalid(
                "an absorption cannot predate an existing child spawn".to_owned(),
            ));
        }

        self.state
            .nodes
            .get_mut(source_id)
            .expect("source was checked above")
            .lifecycle = TentacleLifecycle::Absorbed {
            into: authenticated_target.to_owned(),
            at_ms,
        };
        self.state.absorption_records.push(AbsorptionRecord {
            source_id: source_id.to_owned(),
            target_id: authenticated_target.to_owned(),
            at_ms,
            knowledge_hashes,
            operator_confirmed,
        });
        self.state.revision = self
            .state
            .revision
            .checked_add(1)
            .ok_or(EvolutionError::Limit("revision"))?;
        Ok(self
            .state
            .absorption_records
            .last()
            .expect("newly inserted absorption exists"))
    }

    /// Records an inherited local root being absorbed into the external parent that provisioned
    /// it. The external parent is intentionally not fabricated as a node in this local projection;
    /// the executor receipt remains the evidence that the cross-process transfer completed.
    pub fn record_external_parent_absorption(
        &mut self,
        source_id: &str,
        target_id: &str,
        at_ms: u64,
        knowledge_hashes: Vec<String>,
    ) -> Result<&AbsorptionRecord, EvolutionError> {
        validate_id(source_id, "external absorption source ID")?;
        validate_id(target_id, "external absorption target ID")?;
        validate_hashes(&knowledge_hashes)?;
        if source_id != self.state.root_id
            || self.state.external_parent_id.as_deref() != Some(target_id)
        {
            return Err(EvolutionError::Unauthorized(
                "external absorption target is not this inherited root's recorded parent"
                    .to_owned(),
            ));
        }
        if self
            .state
            .absorption_records
            .last()
            .is_some_and(|record| record.at_ms > at_ms)
        {
            return Err(EvolutionError::Invalid(
                "absorption records must be chronological".to_owned(),
            ));
        }
        let source = self
            .state
            .nodes
            .get(source_id)
            .ok_or_else(|| EvolutionError::NotFound(source_id.to_owned()))?;
        if source.lifecycle != TentacleLifecycle::Active {
            return Err(EvolutionError::Conflict(
                "the source Tentacle was already absorbed".to_owned(),
            ));
        }
        if at_ms < source.spawned_at_ms
            || source
                .children
                .iter()
                .filter_map(|child| self.state.nodes.get(child))
                .any(|child| child.spawned_at_ms > at_ms)
        {
            return Err(EvolutionError::Invalid(
                "external-parent absorption predates the source or one of its children".to_owned(),
            ));
        }

        self.state
            .nodes
            .get_mut(source_id)
            .expect("source was checked above")
            .lifecycle = TentacleLifecycle::Absorbed {
            into: target_id.to_owned(),
            at_ms,
        };
        self.state.absorption_records.push(AbsorptionRecord {
            source_id: source_id.to_owned(),
            target_id: target_id.to_owned(),
            at_ms,
            knowledge_hashes,
            operator_confirmed: false,
        });
        self.state.revision = self
            .state
            .revision
            .checked_add(1)
            .ok_or(EvolutionError::Limit("revision"))?;
        Ok(self
            .state
            .absorption_records
            .last()
            .expect("newly inserted absorption exists"))
    }

    pub fn family(&self, tentacle_id: &str) -> Result<Family, EvolutionError> {
        let node = self
            .state
            .nodes
            .get(tentacle_id)
            .ok_or_else(|| EvolutionError::NotFound(tentacle_id.to_owned()))?;
        let siblings = node
            .parent_id
            .as_ref()
            .and_then(|parent| self.state.nodes.get(parent))
            .map(|parent| {
                parent
                    .children
                    .iter()
                    .filter(|candidate| candidate.as_str() != tentacle_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        Ok(Family {
            parent: node.parent_id.clone().or_else(|| {
                (tentacle_id == self.state.root_id)
                    .then(|| self.state.external_parent_id.clone())
                    .flatten()
            }),
            children: node.children.clone(),
            siblings,
        })
    }

    pub fn ancestors(&self, tentacle_id: &str) -> Result<Vec<String>, EvolutionError> {
        let mut current = self
            .state
            .nodes
            .get(tentacle_id)
            .ok_or_else(|| EvolutionError::NotFound(tentacle_id.to_owned()))?;
        let mut result = Vec::new();
        while let Some(parent_id) = &current.parent_id {
            result.push(parent_id.clone());
            current =
                self.state.nodes.get(parent_id).ok_or_else(|| {
                    EvolutionError::Invalid(format!("missing parent {parent_id}"))
                })?;
        }
        if current.tentacle_id == self.state.root_id
            && let Some(parent_id) = &self.state.external_parent_id
        {
            result.push(parent_id.clone());
        }
        Ok(result)
    }

    pub fn lifecycle_decision(
        &self,
        tentacle_id: &str,
        signal: LifecycleSignal,
    ) -> Result<LifecycleDecision, EvolutionError> {
        let node = self
            .state
            .nodes
            .get(tentacle_id)
            .ok_or_else(|| EvolutionError::NotFound(tentacle_id.to_owned()))?;
        if node.lifecycle != TentacleLifecycle::Active {
            return Err(EvolutionError::Conflict(
                "an inactive Tentacle has no pending lifecycle decision".to_owned(),
            ));
        }
        Ok(match signal {
            LifecycleSignal::Healthy => LifecycleDecision::Continue,
            LifecycleSignal::Starving => LifecycleDecision::Warn,
            LifecycleSignal::Death => {
                let mut candidates = BTreeSet::new();
                let family = self.family(tentacle_id)?;
                candidates.extend(family.parent);
                candidates.extend(family.siblings);
                candidates.extend(family.children);
                let candidates: Vec<_> = candidates
                    .into_iter()
                    .filter(|id| {
                        (tentacle_id == self.state.root_id
                            && self.state.external_parent_id.as_deref() == Some(id.as_str()))
                            || self.state.nodes.get(id).is_some_and(|candidate| {
                                candidate.lifecycle == TentacleLifecycle::Active
                            })
                    })
                    .collect();
                LifecycleDecision::BeginDeath {
                    absorption_candidates: candidates,
                }
            }
        })
    }
}

impl LifecycleState {
    pub fn new(
        tentacle_id: impl Into<String>,
        auto_spawn_enabled: bool,
    ) -> Result<Self, EvolutionError> {
        let tentacle_id = tentacle_id.into();
        validate_id(&tentacle_id, "lifecycle Tentacle ID")?;
        Ok(Self {
            schema_version: LIFECYCLE_SCHEMA_VERSION,
            revision: 0,
            tentacle_id,
            auto_spawn_enabled,
            pending_death: None,
            shutdown_completed_at_ms: None,
            intents: BTreeMap::new(),
            receipts: Vec::new(),
            canceled_action_ids: BTreeSet::new(),
            pending_absorption_projection_action_ids: BTreeSet::new(),
            absorption_projections: BTreeMap::new(),
            spawn_projections: BTreeMap::new(),
        })
    }

    pub fn validate(&self) -> Result<(), EvolutionError> {
        if self.schema_version != LIFECYCLE_SCHEMA_VERSION {
            return Err(EvolutionError::Invalid(format!(
                "unsupported lifecycle schema version {}",
                self.schema_version
            )));
        }
        validate_id(&self.tentacle_id, "lifecycle Tentacle ID")?;
        if let Some(death) = &self.pending_death {
            validate_sha256(&death.judgment_id, "pending-death judgment ID")?;
            if death.grace_ends_at_ms < death.scheduled_at_ms {
                return Err(EvolutionError::Invalid(
                    "death grace period ends before it begins".to_owned(),
                ));
            }
        }
        for (action_id, intent) in &self.intents {
            validate_sha256(action_id, "lifecycle action ID")?;
            if action_id != &intent.action_id {
                return Err(EvolutionError::Invalid(
                    "lifecycle intent map key disagrees with its action ID".to_owned(),
                ));
            }
            validate_lifecycle_action(&intent.action)?;
            if lifecycle_action_id(&intent.action)? != *action_id {
                return Err(EvolutionError::Invalid(
                    "lifecycle action ID is not canonical".to_owned(),
                ));
            }
            if let LifecycleAction::Shutdown {
                tentacle_id,
                judgment_id,
                after_action_id: Some(dependency_id),
            } = &intent.action
            {
                let dependency = self.intents.get(dependency_id).ok_or_else(|| {
                    EvolutionError::Invalid(
                        "shutdown references a missing absorption action".to_owned(),
                    )
                })?;
                if !matches!(
                    &dependency.action,
                    LifecycleAction::Absorb {
                        source_id,
                        judgment_id: dependency_judgment,
                        ..
                    } if source_id == tentacle_id && dependency_judgment == judgment_id
                ) {
                    return Err(EvolutionError::Invalid(
                        "shutdown dependency is not the exact matching death absorption".to_owned(),
                    ));
                }
            }
        }
        let mut receipt_ids = BTreeSet::new();
        let mut consumed_token_transactions = BTreeSet::new();
        for receipt in &self.receipts {
            validate_sha256(&receipt.action_id, "lifecycle receipt action ID")?;
            let intent = self.intents.get(&receipt.action_id).ok_or_else(|| {
                EvolutionError::Invalid("lifecycle receipt references a missing intent".to_owned())
            })?;
            if !receipt_ids.insert(&receipt.action_id) {
                return Err(EvolutionError::Invalid(
                    "lifecycle action has more than one terminal receipt".to_owned(),
                ));
            }
            validate_lifecycle_receipt(intent, receipt)?;
            if receipt.status == LifecycleReceiptStatus::Succeeded
                && matches!(intent.action, LifecycleAction::SpendForSurvival { .. })
            {
                let transaction_hash = &receipt
                    .confirmed_chain_receipt
                    .as_ref()
                    .expect("successful survival receipt was validated above")
                    .transaction_hash;
                if !consumed_token_transactions.insert(transaction_hash) {
                    return Err(EvolutionError::Invalid(
                        "survival transaction is consumed by more than one death".to_owned(),
                    ));
                }
            }
            if receipt.status == LifecycleReceiptStatus::Succeeded
                && matches!(
                    intent.action,
                    LifecycleAction::RewardVeniceKey { .. }
                        | LifecycleAction::RewardAcolyteContribution { .. }
                )
            {
                let transaction_hash = &receipt
                    .confirmed_transfer_receipt
                    .as_ref()
                    .expect("successful key-reward receipt was validated above")
                    .transaction_hash;
                if !consumed_token_transactions.insert(transaction_hash) {
                    return Err(EvolutionError::Invalid(
                        "one token transaction cannot satisfy more than one lifecycle action"
                            .to_owned(),
                    ));
                }
            }
        }
        for action_id in &self.canceled_action_ids {
            validate_sha256(action_id, "canceled lifecycle action ID")?;
            if !self.intents.contains_key(action_id) {
                return Err(EvolutionError::Invalid(
                    "canceled lifecycle action references a missing intent".to_owned(),
                ));
            }
        }
        for action_id in &self.pending_absorption_projection_action_ids {
            validate_sha256(action_id, "pending absorption projection action ID")?;
            let intent = self.intents.get(action_id).ok_or_else(|| {
                EvolutionError::Invalid(
                    "pending absorption projection references a missing intent".to_owned(),
                )
            })?;
            let receipt = self.receipt(action_id).ok_or_else(|| {
                EvolutionError::Invalid(
                    "pending absorption projection has no terminal receipt".to_owned(),
                )
            })?;
            if !matches!(
                &intent.action,
                LifecycleAction::Absorb { judgment_id, .. }
                    if self.pending_death.as_ref().is_some_and(|pending| {
                        pending.judgment_id == *judgment_id
                    })
            ) || receipt.status != LifecycleReceiptStatus::Succeeded
                || self.absorption_projections.contains_key(action_id)
            {
                return Err(EvolutionError::Invalid(
                    "pending absorption projection is not an unprojected successful absorption"
                        .to_owned(),
                ));
            }
        }
        for (action_id, projection) in &self.absorption_projections {
            validate_sha256(action_id, "absorption projection action ID")?;
            let intent = self.intents.get(action_id).ok_or_else(|| {
                EvolutionError::Invalid(
                    "absorption projection references a missing intent".to_owned(),
                )
            })?;
            let receipt = self.receipt(action_id).ok_or_else(|| {
                EvolutionError::Invalid("absorption projection has no terminal receipt".to_owned())
            })?;
            if !matches!(intent.action, LifecycleAction::Absorb { .. })
                || receipt.status != LifecycleReceiptStatus::Succeeded
                || receipt.completed_at_ms
                    > projection
                        .projected_at_ms
                        .saturating_add(LIFECYCLE_RECEIPT_CLOCK_SKEW_MS)
                || self
                    .pending_absorption_projection_action_ids
                    .contains(action_id)
            {
                return Err(EvolutionError::Invalid(
                    "absorption projection does not match its successful executor receipt"
                        .to_owned(),
                ));
            }
        }
        for (action_id, projection) in &self.spawn_projections {
            validate_sha256(action_id, "spawn projection action ID")?;
            let intent = self.intents.get(action_id).ok_or_else(|| {
                EvolutionError::Invalid(
                    "spawn projection references a missing lifecycle intent".to_owned(),
                )
            })?;
            if !matches!(intent.action, LifecycleAction::Spawn { .. }) {
                return Err(EvolutionError::Invalid(
                    "spawn projection references a non-spawn action".to_owned(),
                ));
            }
            let receipt = self.receipt(action_id).ok_or_else(|| {
                EvolutionError::Invalid(
                    "spawn projection has no terminal provisioning receipt".to_owned(),
                )
            })?;
            if receipt.status != LifecycleReceiptStatus::Succeeded
                || projection.completed_at_ms != receipt.completed_at_ms
                || projection.completed_at_ms < intent.created_at_ms
                || projection.metrics_period_started_at_unix_seconds < 0
            {
                return Err(EvolutionError::Invalid(
                    "spawn projection does not match its successful provisioning receipt"
                        .to_owned(),
                ));
            }
        }
        if self.shutdown_completed_at_ms.is_some()
            && !self.receipts.iter().any(|receipt| {
                receipt.status == LifecycleReceiptStatus::Succeeded
                    && self.intents.get(&receipt.action_id).is_some_and(|intent| {
                        matches!(intent.action, LifecycleAction::Shutdown { .. })
                    })
            })
        {
            return Err(EvolutionError::Invalid(
                "terminal shutdown state has no successful shutdown receipt".to_owned(),
            ));
        }
        let expected_revision = u64::try_from(self.intents.len())
            .ok()
            .and_then(|intents| intents.checked_add(u64::try_from(self.receipts.len()).ok()?))
            .and_then(|mutations| {
                mutations.checked_add(u64::try_from(self.spawn_projections.len()).ok()?)
            })
            .and_then(|mutations| {
                mutations.checked_add(u64::try_from(self.absorption_projections.len()).ok()?)
            })
            .and_then(|mutations| mutations.checked_add(u64::from(self.pending_death.is_some())))
            .ok_or(EvolutionError::Limit("lifecycle revision"))?;
        if self.revision < expected_revision {
            return Err(EvolutionError::Invalid(
                "lifecycle revision predates its durable mutation log".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn set_auto_spawn_enabled(&mut self, enabled: bool) -> Result<bool, EvolutionError> {
        if self.auto_spawn_enabled == enabled {
            return Ok(false);
        }
        self.auto_spawn_enabled = enabled;
        self.bump_revision()?;
        Ok(true)
    }

    pub fn enqueue_venice_key_reward(
        &mut self,
        created_at_ms: u64,
        action: LifecycleAction,
    ) -> Result<LifecycleIntent, EvolutionError> {
        if !matches!(action, LifecycleAction::RewardVeniceKey { .. }) {
            return Err(EvolutionError::Invalid(
                "Venice-key reward enqueue requires its exact action type".to_owned(),
            ));
        }
        Ok(self.insert_intent(created_at_ms, action)?.clone())
    }

    pub fn enqueue_acolyte_contribution_reward(
        &mut self,
        created_at_ms: u64,
        action: LifecycleAction,
    ) -> Result<LifecycleIntent, EvolutionError> {
        if !matches!(action, LifecycleAction::RewardAcolyteContribution { .. }) {
            return Err(EvolutionError::Invalid(
                "acolyte contribution reward enqueue requires its exact action type".to_owned(),
            ));
        }
        Ok(self.insert_intent(created_at_ms, action)?.clone())
    }

    pub fn death_pending(&self) -> bool {
        self.pending_death.is_some()
    }

    pub fn has_mandatory_recovery_work(&self) -> bool {
        self.shutdown_completed_at_ms.is_some()
            || self.pending_death.is_some()
            || self.has_unapplied_absorption_projection()
            || self.intents.values().any(|intent| {
                self.receipt(&intent.action_id).is_none()
                    && !self.canceled_action_ids.contains(&intent.action_id)
                    && matches!(
                        intent.action,
                        LifecycleAction::Absorb { .. } | LifecycleAction::Shutdown { .. }
                    )
            })
    }

    /// Retires the old policy that treated a low Scales judgment as terminal. A completed
    /// external absorption cannot be undone, but pending work and a local Shutdown receipt can be
    /// preserved as audit history while this identity returns to ordinary dormant operation.
    pub fn retire_legacy_death_as_dormancy(&mut self) -> Result<bool, EvolutionError> {
        let Some(pending) = self.pending_death.clone() else {
            return Ok(false);
        };
        let has_successful_absorption = self.receipts.iter().any(|receipt| {
            receipt.status == LifecycleReceiptStatus::Succeeded
                && self.intents.get(&receipt.action_id).is_some_and(|intent| {
                    matches!(
                        &intent.action,
                        LifecycleAction::Absorb { judgment_id, .. }
                            if judgment_id == &pending.judgment_id
                    )
                })
        });
        if has_successful_absorption || self.has_unapplied_absorption_projection() {
            return Err(EvolutionError::Conflict(
                "legacy Death already completed external absorption and cannot be revived locally"
                    .to_owned(),
            ));
        }

        let canceled = self
            .intents
            .values()
            .filter(|intent| {
                self.receipt(&intent.action_id).is_none()
                    && matches!(
                        &intent.action,
                        LifecycleAction::SpendForSurvival { judgment_id, .. }
                            | LifecycleAction::Absorb { judgment_id, .. }
                            | LifecycleAction::Shutdown { judgment_id, .. }
                            if judgment_id == &pending.judgment_id
                    )
            })
            .map(|intent| intent.action_id.clone())
            .collect::<Vec<_>>();
        for action_id in canceled {
            self.canceled_action_ids.insert(action_id);
        }
        self.pending_death = None;
        self.shutdown_completed_at_ms = None;
        self.bump_revision()?;
        Ok(true)
    }

    pub fn schedule_death(
        &mut self,
        judgment_id: &str,
        scheduled_at_ms: u64,
        grace_period_ms: u64,
        survival_spend: Option<SurvivalSpendBinding>,
    ) -> Result<bool, EvolutionError> {
        validate_sha256(judgment_id, "death judgment ID")?;
        if let Some(existing) = &self.pending_death {
            if existing.judgment_id == judgment_id {
                return Ok(false);
            }
            return Err(EvolutionError::Conflict(
                "a death grace period is already active".to_owned(),
            ));
        }
        let grace_ends_at_ms = scheduled_at_ms
            .checked_add(grace_period_ms)
            .ok_or(EvolutionError::Limit("death grace timestamp"))?;
        self.pending_death = Some(PendingDeath {
            judgment_id: judgment_id.to_owned(),
            scheduled_at_ms,
            grace_ends_at_ms,
        });
        self.bump_revision()?;
        if let Some(binding) = survival_spend.filter(|binding| binding.expenditure_basis_points > 0)
        {
            self.enqueue_survival_spend_for_pending_death(judgment_id, scheduled_at_ms, binding)?;
        }
        Ok(true)
    }

    pub fn enqueue_survival_spend_for_pending_death(
        &mut self,
        judgment_id: &str,
        created_at_ms: u64,
        binding: SurvivalSpendBinding,
    ) -> Result<bool, EvolutionError> {
        let pending = self.pending_death.as_ref().ok_or_else(|| {
            EvolutionError::Conflict("no pending death can receive a survival spend".to_owned())
        })?;
        if pending.judgment_id != judgment_id || created_at_ms > pending.grace_ends_at_ms {
            return Err(EvolutionError::Conflict(
                "survival spend is outside its exact pending death grace period".to_owned(),
            ));
        }
        let grace_ends_at_ms = pending.grace_ends_at_ms;
        let unreceipted = self.intents.values().find(|intent| {
            matches!(
                &intent.action,
                LifecycleAction::SpendForSurvival { judgment_id: existing, .. }
                    if existing == judgment_id && self.receipt(&intent.action_id).is_none()
            )
        });
        if let Some(existing) = unreceipted {
            let same_binding = matches!(
                &existing.action,
                LifecycleAction::SpendForSurvival {
                    grace_ends_at_ms: existing_grace,
                    expenditure_basis_points,
                    chain_id,
                    token_contract,
                    treasury_address,
                    configuration_identity,
                    exact_amount,
                    ..
                } if *existing_grace == grace_ends_at_ms
                    && *expenditure_basis_points == binding.expenditure_basis_points
                    && *chain_id == binding.chain_id
                    && *token_contract == binding.token_contract
                    && *treasury_address == binding.treasury_address
                    && *configuration_identity == binding.configuration_identity
                    && exact_amount == &binding.exact_amount
            );
            if same_binding {
                // A transient underfunded observation may have hidden this action while its
                // executor outcome was still unknown. Re-offer the same canonical action ID;
                // never mint a replacement idempotency key for an unreceipted burn.
                if self.canceled_action_ids.remove(&existing.action_id) {
                    self.bump_revision()?;
                    return Ok(true);
                }
                return Ok(false);
            }
            return Err(EvolutionError::Conflict(
                "an unreceipted survival burn must be reconciled on-chain before its binding can change"
                    .to_owned(),
            ));
        }
        let attempt = self
            .intents
            .values()
            .filter_map(|intent| match &intent.action {
                LifecycleAction::SpendForSurvival {
                    judgment_id: existing,
                    attempt,
                    ..
                } if existing == judgment_id => Some(*attempt),
                _ => None,
            })
            .max()
            .map_or(0, |attempt| attempt.saturating_add(1));
        let action = LifecycleAction::SpendForSurvival {
            tentacle_id: self.tentacle_id.clone(),
            judgment_id: judgment_id.to_owned(),
            attempt,
            grace_ends_at_ms,
            expenditure_basis_points: binding.expenditure_basis_points,
            chain_id: binding.chain_id,
            token_contract: binding.token_contract,
            treasury_address: binding.treasury_address,
            burn_destination: [0; 20],
            configuration_identity: binding.configuration_identity,
            exact_amount: binding.exact_amount,
        };
        let before = self.intents.len();
        self.insert_intent(created_at_ms, action)?;
        Ok(self.intents.len() != before)
    }

    pub fn cancel_pending_survival_spends(
        &mut self,
        judgment_id: &str,
    ) -> Result<bool, EvolutionError> {
        validate_sha256(judgment_id, "survival judgment ID")?;
        let action_ids = self
            .intents
            .values()
            .filter(|intent| {
                matches!(
                    &intent.action,
                    LifecycleAction::SpendForSurvival { judgment_id: existing, .. }
                        if existing == judgment_id && self.receipt(&intent.action_id).is_none()
                )
            })
            .map(|intent| intent.action_id.clone())
            .collect::<Vec<_>>();
        let mut changed = false;
        for action_id in action_ids {
            changed |= self.canceled_action_ids.insert(action_id);
        }
        if changed {
            self.bump_revision()?;
        }
        Ok(changed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_spawn(
        &mut self,
        created_at_ms: u64,
        parent_id: String,
        child_id: String,
        judgment_id: String,
        child_nature: TentacleNature,
        authorization_actor_id: String,
        authorization_event_id_sha256: String,
    ) -> Result<&LifecycleIntent, EvolutionError> {
        self.insert_intent(
            created_at_ms,
            LifecycleAction::Spawn {
                parent_id,
                child_id,
                judgment_id,
                child_nature,
                authorization_actor_id,
                authorization_event_id_sha256,
            },
        )
    }

    pub fn enqueue_absorption(
        &mut self,
        created_at_ms: u64,
        source_id: String,
        target_id: String,
        judgment_id: String,
    ) -> Result<&LifecycleIntent, EvolutionError> {
        self.insert_intent(
            created_at_ms,
            LifecycleAction::Absorb {
                source_id,
                target_id,
                judgment_id,
            },
        )
    }

    pub fn reconcile_expired_death(
        &mut self,
        now_ms: u64,
        absorption_target: Option<String>,
    ) -> Result<bool, EvolutionError> {
        let Some(pending) = &self.pending_death else {
            return Ok(false);
        };
        if now_ms < pending.grace_ends_at_ms {
            return Ok(false);
        }
        let judgment_id = pending.judgment_id.clone();
        let mut changed = false;
        let expired_spend_ids = self
            .intents
            .values()
            .filter(|intent| {
                matches!(
                    &intent.action,
                    LifecycleAction::SpendForSurvival { judgment_id: action_judgment, .. }
                        if action_judgment == &judgment_id
                            && self.receipt(&intent.action_id).is_none()
                )
            })
            .map(|intent| intent.action_id.clone())
            .collect::<Vec<_>>();
        for action_id in expired_spend_ids {
            changed |= self.canceled_action_ids.insert(action_id);
        }
        if let Some(target_id) = absorption_target
            && !self.intents.values().any(|intent| {
                matches!(
                    &intent.action,
                    LifecycleAction::Absorb { source_id, judgment_id: action_judgment, .. }
                        if source_id == &self.tentacle_id && action_judgment == &judgment_id
                )
            })
        {
            self.enqueue_absorption(
                now_ms,
                self.tentacle_id.clone(),
                target_id,
                judgment_id.clone(),
            )?;
            changed = true;
        }
        let absorption_action_id = self.intents.values().find_map(|intent| {
            matches!(
                &intent.action,
                LifecycleAction::Absorb { source_id, judgment_id: action_judgment, .. }
                    if source_id == &self.tentacle_id && action_judgment == &judgment_id
            )
            .then(|| intent.action_id.clone())
        });
        let before = self.intents.len();
        self.insert_intent(
            now_ms,
            LifecycleAction::Shutdown {
                tentacle_id: self.tentacle_id.clone(),
                judgment_id,
                after_action_id: absorption_action_id,
            },
        )?;
        if changed {
            self.bump_revision()?;
        }
        Ok(changed || self.intents.len() != before)
    }

    pub fn next_due_action(&self) -> Option<&LifecycleIntent> {
        self.next_due_action_excluding(&BTreeSet::new())
    }

    /// Selects the next due outbox action while skipping actions the supervisor already attempted
    /// during its current tick. This prevents an unreceipted executor attempt from starving other
    /// independent actions without terminalizing or rate-limiting the original intent.
    pub fn next_due_action_excluding(
        &self,
        excluded_action_ids: &BTreeSet<String>,
    ) -> Option<&LifecycleIntent> {
        self.intents
            .values()
            .filter(|intent| {
                if self.receipt(&intent.action_id).is_some() {
                    return false;
                }
                if self.canceled_action_ids.contains(&intent.action_id)
                    || excluded_action_ids.contains(&intent.action_id)
                {
                    return false;
                }
                match &intent.action {
                    LifecycleAction::Spawn { .. } if self.pending_death.is_some() => false,
                    LifecycleAction::RewardVeniceKey { .. }
                    | LifecycleAction::RewardAcolyteContribution { .. }
                        if self.pending_death.is_some() =>
                    {
                        false
                    }
                    LifecycleAction::Shutdown {
                        after_action_id: Some(dependency),
                        ..
                    } => {
                        self.receipt(dependency).is_some()
                            || excluded_action_ids.contains(dependency)
                    }
                    _ => true,
                }
            })
            .min_by_key(|intent| {
                (
                    match &intent.action {
                        LifecycleAction::Absorb { .. } => 0_u8,
                        LifecycleAction::Shutdown { .. } => 1,
                        LifecycleAction::SpendForSurvival { .. } => 2,
                        LifecycleAction::RewardVeniceKey { .. }
                        | LifecycleAction::RewardAcolyteContribution { .. } => 3,
                        LifecycleAction::Spawn { .. } => 4,
                    },
                    intent.created_at_ms,
                )
            })
    }

    pub fn receipt(&self, action_id: &str) -> Option<&LifecycleReceipt> {
        self.receipts
            .iter()
            .find(|receipt| receipt.action_id == action_id)
    }

    pub fn action_succeeded(&self, action_id: &str) -> bool {
        self.receipt(action_id)
            .is_some_and(|receipt| receipt.status == LifecycleReceiptStatus::Succeeded)
    }

    pub fn has_unapplied_absorption_projection(&self) -> bool {
        self.receipts.iter().any(|receipt| {
            receipt.status == LifecycleReceiptStatus::Succeeded
                && !self.absorption_projections.contains_key(&receipt.action_id)
                && self.intents.get(&receipt.action_id).is_some_and(|intent| {
                    matches!(
                        &intent.action,
                        LifecycleAction::Absorb { judgment_id, .. }
                            if self.pending_death.as_ref().is_some_and(|pending| {
                                pending.judgment_id == *judgment_id
                            }) && !self.canceled_action_ids.contains(&intent.action_id)
                    )
                })
        })
    }

    /// Migrates successful pre-marker absorption receipts into explicit pending projection state.
    pub fn reconcile_absorption_projection_tracking(&mut self) -> Result<bool, EvolutionError> {
        let expected = self
            .receipts
            .iter()
            .filter(|receipt| receipt.status == LifecycleReceiptStatus::Succeeded)
            .filter_map(|receipt| {
                self.intents.get(&receipt.action_id).and_then(|intent| {
                    matches!(
                        &intent.action,
                        LifecycleAction::Absorb { judgment_id, .. }
                            if self.pending_death.as_ref().is_some_and(|pending| {
                                pending.judgment_id == *judgment_id
                            }) && !self.canceled_action_ids.contains(&intent.action_id)
                                && !self.absorption_projections.contains_key(&intent.action_id)
                    )
                    .then(|| intent.action_id.clone())
                })
            })
            .collect::<BTreeSet<_>>();
        let before = self.pending_absorption_projection_action_ids.len();
        self.pending_absorption_projection_action_ids
            .extend(expected);
        if self.pending_absorption_projection_action_ids.len() == before {
            return Ok(false);
        }
        self.bump_revision()?;
        Ok(true)
    }

    pub fn record_absorption_projection(
        &mut self,
        action_id: &str,
        projected_at_ms: u64,
    ) -> Result<bool, EvolutionError> {
        validate_sha256(action_id, "absorption projection action ID")?;
        let intent = self
            .intents
            .get(action_id)
            .ok_or_else(|| EvolutionError::NotFound(action_id.to_owned()))?;
        if !matches!(intent.action, LifecycleAction::Absorb { .. }) {
            return Err(EvolutionError::Conflict(
                "only an absorption action can receive an absorption projection marker".to_owned(),
            ));
        }
        let receipt = self.receipt(action_id).ok_or_else(|| {
            EvolutionError::Conflict(
                "absorption projection requires a terminal executor receipt".to_owned(),
            )
        })?;
        if receipt.status != LifecycleReceiptStatus::Succeeded
            || receipt.completed_at_ms
                > projected_at_ms.saturating_add(LIFECYCLE_RECEIPT_CLOCK_SKEW_MS)
        {
            return Err(EvolutionError::Conflict(
                "absorption projection must follow its successful executor receipt".to_owned(),
            ));
        }
        let projection = AbsorptionProjection { projected_at_ms };
        if let Some(existing) = self.absorption_projections.get(action_id) {
            if existing == &projection {
                return Ok(false);
            }
            return Err(EvolutionError::Conflict(
                "absorption action already has a different lineage projection".to_owned(),
            ));
        }
        self.pending_absorption_projection_action_ids
            .remove(action_id);
        self.absorption_projections
            .insert(action_id.to_owned(), projection);
        self.bump_revision()?;
        Ok(true)
    }

    pub fn record_spawn_projection(
        &mut self,
        action_id: &str,
        completed_at_ms: u64,
        metrics_period_started_at_unix_seconds: i64,
    ) -> Result<bool, EvolutionError> {
        validate_sha256(action_id, "spawn projection action ID")?;
        let intent = self
            .intents
            .get(action_id)
            .ok_or_else(|| EvolutionError::NotFound(action_id.to_owned()))?;
        if !matches!(intent.action, LifecycleAction::Spawn { .. }) {
            return Err(EvolutionError::Invalid(
                "only a Spawn action can receive a spawn projection marker".to_owned(),
            ));
        }
        let receipt = self.receipt(action_id).ok_or_else(|| {
            EvolutionError::Conflict(
                "spawn projection requires a successful provisioning receipt".to_owned(),
            )
        })?;
        if receipt.status != LifecycleReceiptStatus::Succeeded
            || receipt.completed_at_ms != completed_at_ms
            || metrics_period_started_at_unix_seconds < 0
        {
            return Err(EvolutionError::Conflict(
                "spawn projection does not match its provisioning receipt or metrics period"
                    .to_owned(),
            ));
        }
        let projection = SpawnProjection {
            completed_at_ms,
            metrics_period_started_at_unix_seconds,
        };
        if let Some(existing) = self.spawn_projections.get(action_id) {
            if existing == &projection {
                return Ok(false);
            }
            return Err(EvolutionError::Conflict(
                "spawn action already projects into a different metrics period".to_owned(),
            ));
        }
        self.spawn_projections
            .insert(action_id.to_owned(), projection);
        self.bump_revision()?;
        Ok(true)
    }

    pub fn acknowledge_action(
        &mut self,
        receipt: LifecycleReceipt,
    ) -> Result<(LifecycleAction, bool), EvolutionError> {
        let intent = self
            .intents
            .get(&receipt.action_id)
            .ok_or_else(|| EvolutionError::NotFound(receipt.action_id.clone()))?
            .clone();
        let action_was_canceled = self.canceled_action_ids.contains(&receipt.action_id);
        validate_lifecycle_receipt(&intent, &receipt)?;
        if let Some(existing) = self.receipt(&receipt.action_id) {
            if existing == &receipt {
                return Ok((intent.action, false));
            }
            return Err(EvolutionError::Conflict(
                "lifecycle action already has a different terminal receipt".to_owned(),
            ));
        }
        if matches!(intent.action, LifecycleAction::SpendForSurvival { .. })
            && receipt.status == LifecycleReceiptStatus::Succeeded
        {
            let transaction_hash = &receipt
                .confirmed_chain_receipt
                .as_ref()
                .expect("successful survival receipt was validated above")
                .transaction_hash;
            if self.receipts.iter().any(|existing| {
                existing.action_id != receipt.action_id
                    && existing.status == LifecycleReceiptStatus::Succeeded
                    && existing
                        .confirmed_chain_receipt
                        .as_ref()
                        .is_some_and(|confirmed| &confirmed.transaction_hash == transaction_hash)
            }) {
                return Err(EvolutionError::Conflict(
                    "confirmed survival transaction was already consumed by another death"
                        .to_owned(),
                ));
            }
        }
        if let LifecycleAction::SpendForSurvival {
            judgment_id,
            grace_ends_at_ms,
            ..
        } = &intent.action
            && receipt.status == LifecycleReceiptStatus::Succeeded
        {
            let pending = self.pending_death.as_ref().ok_or_else(|| {
                EvolutionError::Conflict("no pending death remains to cancel".to_owned())
            })?;
            if pending.judgment_id != *judgment_id || pending.grace_ends_at_ms != *grace_ends_at_ms
            {
                return Err(EvolutionError::Conflict(
                    "survival expenditure did not complete within the matching death grace period"
                        .to_owned(),
                ));
            }
            self.pending_death = None;
            // The deadline reconciler may have stopped offering this spend before a delayed
            // executor delivered proof that it actually completed within grace.
            self.canceled_action_ids.remove(&intent.action_id);
            let related_absorptions = self
                .intents
                .values()
                .filter(|related| {
                    matches!(
                        &related.action,
                        LifecycleAction::Absorb { judgment_id: related_id, .. }
                            if related_id == judgment_id
                    )
                })
                .map(|related| related.action_id.clone())
                .collect::<BTreeSet<_>>();
            let canceled = self
                .intents
                .values()
                .filter_map(|related| {
                    let same_death = match &related.action {
                        LifecycleAction::SpendForSurvival {
                            judgment_id: related_id,
                            ..
                        }
                        | LifecycleAction::Absorb {
                            judgment_id: related_id,
                            ..
                        } => related_id == judgment_id,
                        LifecycleAction::Shutdown {
                            judgment_id: related_id,
                            after_action_id,
                            ..
                        } => {
                            related_id == judgment_id
                                && after_action_id.as_ref().is_none_or(|dependency| {
                                    related_absorptions.contains(dependency)
                                })
                        }
                        LifecycleAction::Spawn { .. } => false,
                        LifecycleAction::RewardVeniceKey { .. }
                        | LifecycleAction::RewardAcolyteContribution { .. } => false,
                    };
                    (same_death
                        && related.action_id != intent.action_id
                        && self.receipt(&related.action_id).is_none())
                    .then(|| related.action_id.clone())
                })
                .collect::<Vec<_>>();
            self.canceled_action_ids.extend(canceled);
            let completed_absorptions = self
                .intents
                .values()
                .filter(|related| {
                    matches!(
                        &related.action,
                        LifecycleAction::Absorb { judgment_id: related_id, .. }
                            if related_id == judgment_id
                                && self.receipt(&related.action_id).is_some_and(|receipt| {
                                    receipt.status == LifecycleReceiptStatus::Succeeded
                                })
                    )
                })
                .map(|related| related.action_id.clone())
                .collect::<Vec<_>>();
            for action_id in completed_absorptions {
                self.pending_absorption_projection_action_ids
                    .remove(&action_id);
            }
        }
        if matches!(intent.action, LifecycleAction::Shutdown { .. })
            && receipt.status == LifecycleReceiptStatus::Succeeded
            && !action_was_canceled
        {
            self.shutdown_completed_at_ms = Some(receipt.completed_at_ms);
        }
        let tracks_pending_absorption = receipt.status == LifecycleReceiptStatus::Succeeded
            && matches!(
                &intent.action,
                LifecycleAction::Absorb { judgment_id, .. }
                    if self.pending_death.as_ref().is_some_and(|pending| {
                        pending.judgment_id == *judgment_id
                    }) && !action_was_canceled
            );
        self.receipts.push(receipt);
        if tracks_pending_absorption {
            self.pending_absorption_projection_action_ids
                .insert(intent.action_id.clone());
        }
        self.bump_revision()?;
        Ok((intent.action, true))
    }

    fn insert_intent(
        &mut self,
        created_at_ms: u64,
        action: LifecycleAction,
    ) -> Result<&LifecycleIntent, EvolutionError> {
        validate_lifecycle_action(&action)?;
        let action_id = lifecycle_action_id(&action)?;
        if !self.intents.contains_key(&action_id) {
            self.intents.insert(
                action_id.clone(),
                LifecycleIntent {
                    action_id: action_id.clone(),
                    created_at_ms,
                    action,
                },
            );
            self.bump_revision()?;
        }
        Ok(self
            .intents
            .get(&action_id)
            .expect("inserted lifecycle action exists"))
    }

    fn bump_revision(&mut self) -> Result<(), EvolutionError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(EvolutionError::Limit("lifecycle revision"))?;
        Ok(())
    }
}

fn validate_lifecycle_action(action: &LifecycleAction) -> Result<(), EvolutionError> {
    match action {
        LifecycleAction::SpendForSurvival {
            tentacle_id,
            judgment_id,
            attempt: _,
            grace_ends_at_ms,
            expenditure_basis_points,
            chain_id,
            token_contract,
            treasury_address,
            burn_destination,
            configuration_identity,
            exact_amount,
        } => {
            validate_id(tentacle_id, "survival Tentacle ID")?;
            validate_sha256(judgment_id, "survival judgment ID")?;
            if *expenditure_basis_points == 0 || *expenditure_basis_points > 10_000 {
                return Err(EvolutionError::Invalid(
                    "survival expenditure must be within 1..=10000 basis points".to_owned(),
                ));
            }
            if *chain_id == 0
                || *token_contract == [0; 20]
                || *treasury_address == [0; 20]
                || *configuration_identity == [0; 32]
                || *grace_ends_at_ms == 0
            {
                return Err(EvolutionError::Invalid(
                    "survival spend is missing its bound chain, token, treasury, or configuration"
                        .to_owned(),
                ));
            }
            if *burn_destination != [0; 20] {
                return Err(EvolutionError::Invalid(
                    "survival burn destination must be the canonical zero address".to_owned(),
                ));
            }
            validate_exact_token_amount(exact_amount)?;
            if exact_amount.basis_points != *expenditure_basis_points {
                return Err(EvolutionError::Invalid(
                    "survival spend amount disagrees with its expenditure basis points".to_owned(),
                ));
            }
        }
        LifecycleAction::RewardVeniceKey {
            tentacle_id,
            provision_event_id_sha256,
            chain_id,
            token_contract,
            treasury_address,
            acolyte_address,
            configuration_identity,
            exact_amount,
        } => {
            validate_id(tentacle_id, "Venice-key reward Tentacle ID")?;
            validate_sha256(
                provision_event_id_sha256,
                "Venice-key provision event ID digest",
            )?;
            if *chain_id != 8_453
                || *token_contract == [0; 20]
                || *treasury_address == [0; 20]
                || *acolyte_address == [0; 20]
                || treasury_address == acolyte_address
                || *configuration_identity == [0; 32]
            {
                return Err(EvolutionError::Invalid(
                    "Venice-key reward is missing its exact Base token, treasury, acolyte, or configuration binding"
                        .to_owned(),
                ));
            }
            validate_whole_token_amount(exact_amount)?;
        }
        LifecycleAction::RewardAcolyteContribution {
            tentacle_id,
            contribution_event_id_sha256,
            information_hunger_basis_points,
            chain_id,
            token_contract,
            treasury_address,
            acolyte_address,
            configuration_identity,
            exact_amount,
            ..
        } => {
            validate_id(tentacle_id, "contribution reward Tentacle ID")?;
            validate_sha256(contribution_event_id_sha256, "contribution event ID digest")?;
            if *information_hunger_basis_points < 10 || *information_hunger_basis_points > 100 {
                return Err(EvolutionError::Invalid(
                    "contribution reward hunger must be between 0.1% and 1%".to_owned(),
                ));
            }
            if *chain_id != 8_453
                || *token_contract == [0; 20]
                || *treasury_address == [0; 20]
                || *acolyte_address == [0; 20]
                || treasury_address == acolyte_address
                || *configuration_identity == [0; 32]
            {
                return Err(EvolutionError::Invalid(
                    "contribution reward is missing its exact Base token, treasury, acolyte, or configuration binding"
                        .to_owned(),
                ));
            }
            validate_whole_token_amount(exact_amount)?;
        }
        LifecycleAction::Absorb {
            source_id,
            target_id,
            judgment_id,
        } => {
            validate_id(source_id, "absorption source ID")?;
            validate_id(target_id, "absorption target ID")?;
            validate_sha256(judgment_id, "absorption death judgment ID")?;
            if source_id == target_id {
                return Err(EvolutionError::Conflict(
                    "a Tentacle cannot absorb itself".to_owned(),
                ));
            }
        }
        LifecycleAction::Spawn {
            parent_id,
            child_id,
            judgment_id,
            child_nature,
            authorization_actor_id,
            authorization_event_id_sha256,
        } => {
            validate_id(parent_id, "spawn parent ID")?;
            validate_id(child_id, "spawn child ID")?;
            validate_sha256(judgment_id, "spawn judgment ID")?;
            validate_id(authorization_actor_id, "spawn authorization actor ID")?;
            validate_sha256(
                authorization_event_id_sha256,
                "spawn authorization event ID digest",
            )?;
            child_nature
                .validate()
                .map_err(|error| EvolutionError::Nature(error.to_string()))?;
            if child_nature.parent_nature_id.is_none() || child_nature.generation == 0 {
                return Err(EvolutionError::Invalid(
                    "spawn action child Nature must contain inherited lineage metadata".to_owned(),
                ));
            }
        }
        LifecycleAction::Shutdown {
            tentacle_id,
            judgment_id,
            after_action_id,
        } => {
            validate_id(tentacle_id, "shutdown Tentacle ID")?;
            validate_sha256(judgment_id, "shutdown death judgment ID")?;
            if let Some(action_id) = after_action_id {
                validate_sha256(action_id, "shutdown dependency action ID")?;
            }
        }
    }
    Ok(())
}

fn validate_lifecycle_receipt(
    intent: &LifecycleIntent,
    receipt: &LifecycleReceipt,
) -> Result<(), EvolutionError> {
    validate_execution_text(receipt.external_reference.as_deref(), "external reference")?;
    validate_execution_text(receipt.detail.as_deref(), "execution detail")?;
    if receipt.completed_at_ms < intent.created_at_ms {
        return Err(EvolutionError::Invalid(
            "lifecycle receipt predates its intent".to_owned(),
        ));
    }
    if let Some(chain_receipt) = &receipt.confirmed_chain_receipt {
        validate_chain_receipt(chain_receipt)?;
    }
    if let Some(transfer_receipt) = &receipt.confirmed_transfer_receipt {
        validate_transfer_receipt(transfer_receipt)?;
    }
    if let Some(provision_receipt) = &receipt.provision_receipt {
        validate_provision_receipt(provision_receipt)?;
    }
    if receipt.status != LifecycleReceiptStatus::Succeeded {
        return Ok(());
    }

    match &intent.action {
        LifecycleAction::SpendForSurvival {
            grace_ends_at_ms,
            chain_id,
            token_contract,
            treasury_address,
            burn_destination,
            configuration_identity,
            exact_amount,
            ..
        } => {
            let confirmed = receipt.confirmed_chain_receipt.as_ref().ok_or_else(|| {
                EvolutionError::Invalid(
                    "successful survival spend lacks confirmed chain evidence".to_owned(),
                )
            })?;
            if confirmed.chain_id != *chain_id
                || confirmed.token_contract != *token_contract
                || confirmed.from_address != *treasury_address
                || confirmed.burn_destination != *burn_destination
                || confirmed.configuration_identity != *configuration_identity
                || confirmed.exact_amount != *exact_amount
                || confirmed.operation != TokenSpendOperation::Burn
                || confirmed.block_timestamp_unix_seconds.saturating_mul(1_000)
                    < intent.created_at_ms
                || confirmed.block_timestamp_unix_seconds.saturating_mul(1_000) > *grace_ends_at_ms
            {
                return Err(EvolutionError::Invalid(
                    "survival receipt does not match the exact burn action".to_owned(),
                ));
            }
        }
        LifecycleAction::RewardVeniceKey {
            chain_id,
            token_contract,
            treasury_address,
            acolyte_address,
            configuration_identity,
            exact_amount,
            ..
        } => {
            let confirmed = receipt.confirmed_transfer_receipt.as_ref().ok_or_else(|| {
                EvolutionError::Invalid(
                    "successful Venice-key reward lacks confirmed transfer evidence".to_owned(),
                )
            })?;
            if confirmed.chain_id != *chain_id
                || confirmed.token_contract != *token_contract
                || confirmed.from_address != *treasury_address
                || confirmed.to_address != *acolyte_address
                || confirmed.configuration_identity != *configuration_identity
                || confirmed.exact_amount != *exact_amount
                || confirmed.block_timestamp_unix_seconds.saturating_mul(1_000)
                    < intent.created_at_ms
            {
                return Err(EvolutionError::Invalid(
                    "Venice-key reward receipt does not match the exact transfer action".to_owned(),
                ));
            }
        }
        LifecycleAction::RewardAcolyteContribution {
            chain_id,
            token_contract,
            treasury_address,
            acolyte_address,
            configuration_identity,
            exact_amount,
            ..
        } => {
            let confirmed = receipt.confirmed_transfer_receipt.as_ref().ok_or_else(|| {
                EvolutionError::Invalid(
                    "successful contribution reward lacks confirmed transfer evidence".to_owned(),
                )
            })?;
            if confirmed.chain_id != *chain_id
                || confirmed.token_contract != *token_contract
                || confirmed.from_address != *treasury_address
                || confirmed.to_address != *acolyte_address
                || confirmed.configuration_identity != *configuration_identity
                || confirmed.exact_amount != *exact_amount
                || confirmed.block_timestamp_unix_seconds.saturating_mul(1_000)
                    < intent.created_at_ms
            {
                return Err(EvolutionError::Invalid(
                    "contribution reward receipt does not match the exact transfer action"
                        .to_owned(),
                ));
            }
        }
        LifecycleAction::Absorb { .. } => {
            let reference = receipt.external_reference.as_deref().ok_or_else(|| {
                EvolutionError::Invalid(
                    "successful absorption lacks a transfer-manifest digest".to_owned(),
                )
            })?;
            validate_sha256(reference, "absorption transfer-manifest digest")?;
        }
        LifecycleAction::Spawn {
            child_id,
            child_nature,
            ..
        } => {
            let provision = receipt.provision_receipt.as_ref().ok_or_else(|| {
                EvolutionError::Invalid(
                    "successful spawn lacks structured provision evidence".to_owned(),
                )
            })?;
            let fingerprint = child_nature
                .fingerprint()
                .map_err(|error| EvolutionError::Nature(error.to_string()))?;
            if provision.child_id != *child_id || provision.child_nature_fingerprint != fingerprint
            {
                return Err(EvolutionError::Invalid(
                    "provision receipt does not match the planned child".to_owned(),
                ));
            }
        }
        LifecycleAction::Shutdown { .. } => {
            if receipt.external_reference.is_none() {
                return Err(EvolutionError::Invalid(
                    "successful shutdown lacks a process-controller acknowledgement".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn lifecycle_action_id(action: &LifecycleAction) -> Result<String, EvolutionError> {
    let canonical = serde_json::to_vec(action)?;
    let mut digest = Sha256::new();
    digest.update(b"cthuwu-lifecycle-action-v1\0");
    digest.update(canonical);
    Ok(format!("{:x}", digest.finalize()))
}

fn validate_execution_text(value: Option<&str>, description: &str) -> Result<(), EvolutionError> {
    if value.is_some_and(|value| {
        value.is_empty()
            || value.len() > MAX_EXECUTION_DETAIL_BYTES
            || value.chars().any(char::is_control)
    }) {
        return Err(EvolutionError::Invalid(format!(
            "{description} is empty, oversized, or contains control characters"
        )));
    }
    Ok(())
}

fn validate_chain_receipt(receipt: &ConfirmedChainReceipt) -> Result<(), EvolutionError> {
    if receipt.chain_id == 0
        || receipt.block_number == 0
        || receipt.block_timestamp_unix_seconds == 0
    {
        return Err(EvolutionError::Invalid(
            "confirmed chain receipt requires nonzero chain, block, and block timestamp".to_owned(),
        ));
    }
    let hash = receipt.transaction_hash.as_bytes();
    if hash.len() != 66
        || &hash[..2] != b"0x"
        || !hash[2..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(EvolutionError::Invalid(
            "confirmed transaction hash must be lowercase 0x-prefixed 32-byte hex".to_owned(),
        ));
    }
    if receipt.token_contract == [0; 20]
        || receipt.from_address == [0; 20]
        || receipt.configuration_identity == [0; 32]
        || receipt.burn_destination != [0; 20]
    {
        return Err(EvolutionError::Invalid(
            "confirmed token burn receipt has invalid token, source, destination, or configuration binding"
                .to_owned(),
        ));
    }
    validate_exact_token_amount(&receipt.exact_amount)?;
    Ok(())
}

fn validate_transfer_receipt(receipt: &ConfirmedTransferReceipt) -> Result<(), EvolutionError> {
    if receipt.chain_id != 8_453
        || receipt.block_number == 0
        || receipt.block_timestamp_unix_seconds == 0
        || receipt.token_contract == [0; 20]
        || receipt.from_address == [0; 20]
        || receipt.to_address == [0; 20]
        || receipt.from_address == receipt.to_address
        || receipt.configuration_identity == [0; 32]
    {
        return Err(EvolutionError::Invalid(
            "confirmed Venice-key reward receipt has invalid chain, token, address, block, or configuration binding"
                .to_owned(),
        ));
    }
    let hash = receipt.transaction_hash.as_bytes();
    if hash.len() != 66
        || &hash[..2] != b"0x"
        || !hash[2..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(EvolutionError::Invalid(
            "confirmed transfer hash must be lowercase 0x-prefixed 32-byte hex".to_owned(),
        ));
    }
    validate_whole_token_amount(&receipt.exact_amount)
}

fn validate_whole_token_amount(amount: &WholeTokenAmount) -> Result<(), EvolutionError> {
    if amount.whole_tokens == 0 || amount.token_decimals > 77 {
        return Err(EvolutionError::Invalid(
            "whole-token transfer amount requires positive units and decimals <= 77".to_owned(),
        ));
    }
    if amount.raw_amount != exact_whole_token_amount(amount.whole_tokens, amount.token_decimals)? {
        return Err(EvolutionError::Invalid(
            "whole-token transfer amount does not match its canonical base-unit formula".to_owned(),
        ));
    }
    Ok(())
}

pub fn exact_whole_token_amount(
    whole_tokens: u64,
    token_decimals: u8,
) -> Result<String, EvolutionError> {
    if whole_tokens == 0 || token_decimals > 77 {
        return Err(EvolutionError::Invalid(
            "whole-token amount requires positive units and decimals <= 77".to_owned(),
        ));
    }
    let mut raw = whole_tokens.to_string();
    raw.extend(std::iter::repeat_n('0', usize::from(token_decimals)));
    const U256_MAX_DECIMAL: &str =
        "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    if raw.len() > U256_MAX_DECIMAL.len()
        || (raw.len() == U256_MAX_DECIMAL.len() && raw.as_str() > U256_MAX_DECIMAL)
    {
        return Err(EvolutionError::Limit("whole-token transfer amount"));
    }
    Ok(raw)
}

fn validate_exact_token_amount(amount: &ExactTokenAmount) -> Result<(), EvolutionError> {
    if amount.total_supply_whole == 0
        || amount.token_decimals > 77
        || amount.basis_points == 0
        || amount.basis_points > 10_000
    {
        return Err(EvolutionError::Invalid(
            "exact token amount formula has invalid supply, decimals, or basis points".to_owned(),
        ));
    }
    let expected = exact_raw_token_amount(
        amount.total_supply_whole,
        amount.token_decimals,
        amount.basis_points,
    )?;
    if amount.raw_amount != expected {
        return Err(EvolutionError::Invalid(
            "exact token amount does not match its canonical base-unit formula".to_owned(),
        ));
    }
    Ok(())
}

/// Returns `floor(total_supply_whole * 10^decimals * basis_points / 10_000)` as
/// canonical decimal. The configured supply is a `u64`, so applying the basis-point multiplier
/// first fits in `u128`; powers of ten are appended only after the exact division is reduced.
pub fn exact_raw_token_amount(
    total_supply_whole: u64,
    token_decimals: u8,
    basis_points: u16,
) -> Result<String, EvolutionError> {
    if total_supply_whole == 0 || token_decimals > 77 || !(1..=10_000).contains(&basis_points) {
        return Err(EvolutionError::Invalid(
            "raw token amount requires positive supply, decimals <= 77, and 1..=10000 basis points"
                .to_owned(),
        ));
    }
    let numerator = u128::from(total_supply_whole)
        .checked_mul(u128::from(basis_points))
        .ok_or(EvolutionError::Limit("survival token amount"))?;
    let raw = if token_decimals >= 4 {
        let mut value = numerator.to_string();
        value.extend(std::iter::repeat_n('0', usize::from(token_decimals - 4)));
        value
    } else {
        let divisor = 10_u128.pow(u32::from(4 - token_decimals));
        (numerator / divisor).to_string()
    };
    if raw == "0" {
        return Err(EvolutionError::Invalid(
            "survival burn formula rounds to zero base units".to_owned(),
        ));
    }
    // 2^256 - 1 has 78 decimal digits. Compare the only ambiguous length explicitly.
    const U256_MAX_DECIMAL: &str =
        "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    if raw.len() > U256_MAX_DECIMAL.len()
        || (raw.len() == U256_MAX_DECIMAL.len() && raw.as_str() > U256_MAX_DECIMAL)
    {
        return Err(EvolutionError::Limit("survival token amount"));
    }
    Ok(raw)
}

fn validate_provision_receipt(receipt: &ProvisionReceipt) -> Result<(), EvolutionError> {
    validate_id(&receipt.child_id, "provisioned child ID")?;
    validate_sha256(
        &receipt.child_nature_fingerprint,
        "provisioned child Nature fingerprint",
    )?;
    validate_sha256(&receipt.manifest_sha256, "provisioning manifest digest")
}

#[derive(Clone, Debug)]
pub struct LifecycleStore {
    state_directory: PathBuf,
    path: PathBuf,
}

impl LifecycleStore {
    pub fn new(data_dir: &Path) -> Result<Self, EvolutionError> {
        require_real_directory(data_dir, "data directory")?;
        restrict_directory(data_dir)?;
        let state_directory = data_dir.join("state");
        ensure_real_private_directory(&state_directory)?;
        let path = state_directory.join("lifecycle.json");
        reject_non_regular_if_present(&path)?;
        Ok(Self {
            state_directory,
            path,
        })
    }

    pub fn save(&self, state: &LifecycleState) -> Result<(), EvolutionError> {
        state.validate()?;
        reject_non_regular_if_present(&self.path)?;
        atomic_write_json(&self.state_directory, &self.path, state)
    }

    pub fn load(&self) -> Result<Option<LifecycleState>, EvolutionError> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(EvolutionError::Invalid(format!(
                "{} must be a regular file",
                self.path.display()
            )));
        }
        restrict_file_path(&self.path)?;
        let state: LifecycleState =
            serde_json::from_reader(BufReader::new(File::open(&self.path)?))?;
        state.validate()?;
        Ok(Some(state))
    }
}

#[derive(Clone, Debug)]
pub struct LineageStore {
    state_directory: PathBuf,
    path: PathBuf,
}

impl LineageStore {
    /// Opens `<data_dir>/state/lineage.json`. `data_dir` must already be a
    /// real directory; the state directory is created owner-only if absent.
    pub fn new(data_dir: &Path) -> Result<Self, EvolutionError> {
        require_real_directory(data_dir, "data directory")?;
        restrict_directory(data_dir)?;
        let state_directory = data_dir.join("state");
        ensure_real_private_directory(&state_directory)?;
        let path = state_directory.join("lineage.json");
        reject_non_regular_if_present(&path)?;
        Ok(Self {
            state_directory,
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn save(&self, lineage: &Lineage) -> Result<(), EvolutionError> {
        validate_state(lineage.state())?;
        reject_non_regular_if_present(&self.path)?;
        atomic_write_json(&self.state_directory, &self.path, lineage.state())?;
        Ok(())
    }

    pub fn load(&self) -> Result<Option<Lineage>, EvolutionError> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(EvolutionError::Invalid(format!(
                "{} must be a regular file",
                self.path.display()
            )));
        }
        restrict_file_path(&self.path)?;
        let state: LineageState = serde_json::from_reader(BufReader::new(File::open(&self.path)?))?;
        Lineage::from_state(state).map(Some)
    }
}

fn validate_state(state: &LineageState) -> Result<(), EvolutionError> {
    if state.schema_version != LINEAGE_SCHEMA_VERSION {
        return Err(EvolutionError::Invalid(format!(
            "unsupported schema version {}",
            state.schema_version
        )));
    }
    validate_id(&state.root_id, "root ID")?;
    if state.nodes.is_empty() {
        return Err(EvolutionError::Invalid(
            "lineage must contain its root node".to_owned(),
        ));
    }
    if state.spawn_records.len() != state.nodes.len().saturating_sub(1) {
        return Err(EvolutionError::Invalid(
            "spawn records must describe every non-root node exactly once".to_owned(),
        ));
    }
    let expected_revision = state
        .spawn_records
        .len()
        .checked_add(state.absorption_records.len())
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(EvolutionError::Limit("revision"))?;
    if state.revision != expected_revision {
        return Err(EvolutionError::Invalid(
            "revision does not match the mutation log".to_owned(),
        ));
    }

    let root = state
        .nodes
        .get(&state.root_id)
        .ok_or_else(|| EvolutionError::Invalid("root ID does not reference a node".to_owned()))?;
    if root.parent_id.is_some() {
        return Err(EvolutionError::Invalid(
            "local root must not reference a parent node inside its projection".to_owned(),
        ));
    }
    match &state.external_parent_id {
        None if root.generation != 0 || root.nature.parent_nature_id.is_some() => {
            return Err(EvolutionError::Invalid(
                "founder root must have generation zero and no parent Nature".to_owned(),
            ));
        }
        Some(parent_id) => {
            validate_id(parent_id, "external parent ID")?;
            if parent_id == &state.root_id
                || root.generation == 0
                || root.nature.parent_nature_id.is_none()
            {
                return Err(EvolutionError::Invalid(
                    "inherited root has invalid external-parent metadata".to_owned(),
                ));
            }
        }
        None => {}
    }

    let mut nature_ids = BTreeSet::new();
    for (key, node) in &state.nodes {
        validate_id(key, "node key")?;
        validate_id(&node.tentacle_id, "Tentacle ID")?;
        if key != &node.tentacle_id {
            return Err(EvolutionError::Invalid(
                "node map key does not match its Tentacle ID".to_owned(),
            ));
        }
        node.nature
            .validate()
            .map_err(|error| EvolutionError::Nature(error.to_string()))?;
        if node.nature.generation != node.generation {
            return Err(EvolutionError::Invalid(
                "node and Nature generations disagree".to_owned(),
            ));
        }
        if !nature_ids.insert(node.nature.nature_id.clone()) {
            return Err(EvolutionError::Invalid(
                "duplicate Nature ID in lineage".to_owned(),
            ));
        }
        let mut unique_children = BTreeSet::new();
        for child_id in &node.children {
            validate_id(child_id, "child ID")?;
            if !unique_children.insert(child_id) {
                return Err(EvolutionError::Invalid("duplicate child edge".to_owned()));
            }
        }
        match &node.lifecycle {
            TentacleLifecycle::Active => {}
            TentacleLifecycle::Absorbed { into, at_ms } => {
                validate_id(into, "absorption target ID")?;
                if *at_ms < node.spawned_at_ms {
                    return Err(EvolutionError::Invalid(
                        "absorption predates the source spawn".to_owned(),
                    ));
                }
            }
        }
    }

    // Validate every edge in both directions before walking from the root.
    for node in state.nodes.values() {
        match &node.parent_id {
            None if node.tentacle_id != state.root_id => {
                return Err(EvolutionError::Invalid(
                    "only the root may omit its parent".to_owned(),
                ));
            }
            None => {
                if node.tentacle_id == state.root_id
                    && state.external_parent_id.is_none()
                    && node.nature.parent_nature_id.is_some()
                {
                    return Err(EvolutionError::Invalid(
                        "root Nature unexpectedly has a parent".to_owned(),
                    ));
                }
            }
            Some(parent_id) => {
                let parent = state.nodes.get(parent_id).ok_or_else(|| {
                    EvolutionError::Invalid(format!("missing parent {parent_id}"))
                })?;
                if parent_id == &node.tentacle_id {
                    return Err(EvolutionError::Invalid("self-parent cycle".to_owned()));
                }
                if !parent.children.contains(&node.tentacle_id) {
                    return Err(EvolutionError::Invalid(
                        "parent is missing the reverse child edge".to_owned(),
                    ));
                }
                if node.generation != parent.generation.saturating_add(1) {
                    return Err(EvolutionError::Invalid(
                        "child generation is inconsistent".to_owned(),
                    ));
                }
                if node.spawned_at_ms < parent.spawned_at_ms {
                    return Err(EvolutionError::Invalid(
                        "child predates its parent".to_owned(),
                    ));
                }
                let spawn = state
                    .spawn_records
                    .iter()
                    .find(|record| record.child_id == node.tentacle_id)
                    .ok_or_else(|| {
                        EvolutionError::Invalid(
                            "child is missing its immutable spawn record".to_owned(),
                        )
                    })?;
                if spawn.parent_id != *parent_id
                    || node.nature.parent_nature_id.as_deref()
                        != Some(spawn.parent_nature_id.as_str())
                {
                    return Err(EvolutionError::Invalid(
                        "child Nature does not reference its parent Nature at spawn".to_owned(),
                    ));
                }
                if let TentacleLifecycle::Absorbed { at_ms, .. } = &parent.lifecycle
                    && node.spawned_at_ms > *at_ms
                {
                    return Err(EvolutionError::Invalid(
                        "child was spawned after its parent was absorbed".to_owned(),
                    ));
                }
            }
        }
        for child_id in &node.children {
            let child = state
                .nodes
                .get(child_id)
                .ok_or_else(|| EvolutionError::Invalid(format!("missing child {child_id}")))?;
            if child.parent_id.as_deref() != Some(node.tentacle_id.as_str()) {
                return Err(EvolutionError::Invalid(
                    "child edge does not point back to its parent".to_owned(),
                ));
            }
        }
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    visit_lineage(&state.root_id, state, &mut visiting, &mut visited)?;
    if visited.len() != state.nodes.len() {
        return Err(EvolutionError::Invalid(
            "lineage contains a disconnected node or cycle".to_owned(),
        ));
    }

    let mut spawned_children = BTreeSet::new();
    for record in &state.spawn_records {
        validate_id(&record.parent_id, "spawn parent ID")?;
        validate_id(&record.child_id, "spawn child ID")?;
        validate_sha256(
            &record.authorization_judgment_id,
            "spawn authorization judgment ID",
        )?;
        validate_id(
            &record.authorization_operator_id,
            "spawn authorization operator ID",
        )?;
        validate_sha256(
            &record.authorization_event_id_sha256,
            "spawn authorization event ID digest",
        )?;
        if !spawned_children.insert(&record.child_id) || record.child_id == state.root_id {
            return Err(EvolutionError::Invalid(
                "duplicate or root child in spawn records".to_owned(),
            ));
        }
        let child = state.nodes.get(&record.child_id).ok_or_else(|| {
            EvolutionError::Invalid("spawn record references a missing child".to_owned())
        })?;
        if child.parent_id.as_deref() != Some(record.parent_id.as_str())
            || child.generation != record.generation
            || child.spawned_at_ms != record.at_ms
            || child.nature.nature_id != record.child_nature_id
            || child.nature.parent_nature_id.as_deref() != Some(record.parent_nature_id.as_str())
        {
            return Err(EvolutionError::Invalid(
                "spawn record disagrees with its child node".to_owned(),
            ));
        }
    }

    validate_absorptions(state)?;
    Ok(())
}

fn visit_lineage(
    id: &str,
    state: &LineageState,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> Result<(), EvolutionError> {
    if visited.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.to_owned()) {
        return Err(EvolutionError::Invalid("lineage cycle detected".to_owned()));
    }
    let node = state
        .nodes
        .get(id)
        .ok_or_else(|| EvolutionError::Invalid(format!("missing node {id}")))?;
    for child in &node.children {
        visit_lineage(child, state, visiting, visited)?;
    }
    visiting.remove(id);
    visited.insert(id.to_owned());
    Ok(())
}

fn validate_absorptions(state: &LineageState) -> Result<(), EvolutionError> {
    let mut active: BTreeSet<_> = state.nodes.keys().cloned().collect();
    let mut previous_time = 0;
    let mut absorbed = BTreeMap::new();
    for record in &state.absorption_records {
        validate_id(&record.source_id, "absorption source ID")?;
        validate_id(&record.target_id, "absorption target ID")?;
        validate_hashes(&record.knowledge_hashes)?;
        if record.source_id == record.target_id {
            return Err(EvolutionError::Invalid(
                "self-absorption is not permitted".to_owned(),
            ));
        }
        if record.at_ms < previous_time {
            return Err(EvolutionError::Invalid(
                "absorption records are not chronological".to_owned(),
            ));
        }
        previous_time = record.at_ms;
        let source = state
            .nodes
            .get(&record.source_id)
            .ok_or_else(|| EvolutionError::Invalid("absorption source is missing".to_owned()))?;
        let local_target = state.nodes.get(&record.target_id);
        let external_parent_target = record.source_id == state.root_id
            && state.external_parent_id.as_deref() == Some(record.target_id.as_str());
        if local_target.is_none() && !external_parent_target {
            return Err(EvolutionError::Invalid(
                "absorption target is neither local nor the root's external parent".to_owned(),
            ));
        }
        if record.at_ms < source.spawned_at_ms
            || local_target.is_some_and(|target| record.at_ms < target.spawned_at_ms)
        {
            return Err(EvolutionError::Invalid(
                "absorption predates a referenced Tentacle".to_owned(),
            ));
        }
        if !active.remove(&record.source_id)
            || local_target.is_some() && !active.contains(&record.target_id)
        {
            return Err(EvolutionError::Invalid(
                "absorption source or target was already inactive".to_owned(),
            ));
        }
        absorbed.insert(
            record.source_id.clone(),
            (record.target_id.clone(), record.at_ms),
        );
    }
    for node in state.nodes.values() {
        match (&node.lifecycle, absorbed.get(&node.tentacle_id)) {
            (TentacleLifecycle::Active, None) => {}
            (TentacleLifecycle::Absorbed { into, at_ms }, Some((target, recorded_at)))
                if into == target && at_ms == recorded_at => {}
            _ => {
                return Err(EvolutionError::Invalid(
                    "node lifecycle disagrees with absorption records".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_id(value: &str, description: &str) -> Result<(), EvolutionError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(EvolutionError::Invalid(format!(
            "{description} is empty, oversized, or contains unsafe characters"
        )));
    }
    Ok(())
}

fn validate_hashes(hashes: &[String]) -> Result<(), EvolutionError> {
    if hashes.len() > MAX_ABSORBED_KNOWLEDGE_HASHES {
        return Err(EvolutionError::Limit("knowledge hash"));
    }
    let mut unique = BTreeSet::new();
    for hash in hashes {
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(EvolutionError::Invalid(
                "knowledge hashes must be lowercase SHA-256 hex".to_owned(),
            ));
        }
        if !unique.insert(hash) {
            return Err(EvolutionError::Invalid(
                "duplicate knowledge hash".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_sha256(value: &str, description: &str) -> Result<(), EvolutionError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(EvolutionError::Invalid(format!(
            "{description} must be lowercase SHA-256 hex"
        )));
    }
    Ok(())
}

fn require_real_directory(path: &Path, description: &str) -> Result<(), EvolutionError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(EvolutionError::Invalid(format!(
            "{description} must be a real directory"
        )));
    }
    Ok(())
}

fn ensure_real_private_directory(path: &Path) -> Result<(), EvolutionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(EvolutionError::Invalid(format!(
                "{} must be a real directory",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(path)?,
        Err(error) => return Err(error.into()),
    }
    restrict_directory(path)?;
    Ok(())
}

fn reject_non_regular_if_present(path: &Path) -> Result<(), EvolutionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            EvolutionError::Invalid(format!("{} must be a regular file", path.display())),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn atomic_write_json<T: Serialize + ?Sized>(
    directory: &Path,
    path: &Path,
    value: &T,
) -> Result<(), EvolutionError> {
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    restrict_open_file(temporary.as_file())?;
    serde_json::to_writer_pretty(&mut temporary, value)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| EvolutionError::Io(error.error))?;
    restrict_file_path(path)?;
    sync_directory(directory)?;
    Ok(())
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn restrict_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn restrict_open_file(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

fn restrict_file_path(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn founder() -> TentacleNature {
        TentacleNature::random().unwrap()
    }

    fn spawnable_founder() -> TentacleNature {
        let mut nature = founder();
        if nature.sacred_ban == SacredBan::Spawning {
            nature.sacred_ban = SacredBan::Governance;
        }
        nature
    }

    fn grant(index: u64) -> SpawnAuthorization {
        SpawnAuthorization {
            judgment_id: format!("{index:064x}"),
            operator_id: "operator-root".to_owned(),
            event_id_sha256: format!("{:064x}", index.saturating_add(1_000)),
        }
    }

    #[test]
    fn tracks_multiple_generations_and_family_relationships() {
        let mut lineage = Lineage::new("root", spawnable_founder(), 1).unwrap();
        lineage
            .spawn_child("root", "root", "child-a", 2, grant(1))
            .unwrap();
        lineage
            .spawn_child("root", "root", "child-b", 3, grant(2))
            .unwrap();
        lineage
            .state
            .nodes
            .get_mut("child-a")
            .unwrap()
            .nature
            .sacred_ban = SacredBan::Governance;
        lineage
            .spawn_child("child-a", "child-a", "grandchild", 4, grant(3))
            .unwrap();

        assert_eq!(lineage.node("grandchild").unwrap().generation, 2);
        assert_eq!(
            lineage
                .node("grandchild")
                .unwrap()
                .nature
                .parent_nature_id
                .as_deref(),
            Some(lineage.node("child-a").unwrap().nature.nature_id.as_str())
        );
        assert_eq!(
            lineage.ancestors("grandchild").unwrap(),
            vec!["child-a", "root"]
        );
        assert_eq!(
            lineage.family("child-a").unwrap(),
            Family {
                parent: Some("root".to_owned()),
                children: vec!["grandchild".to_owned()],
                siblings: vec!["child-b".to_owned()],
            }
        );
        assert_eq!(
            lineage.state().spawn_records[0].authorization_operator_id,
            "operator-root"
        );
        assert_eq!(
            lineage.state().spawn_records[0].authorization_event_id_sha256,
            format!("{:064x}", 1_001)
        );
        validate_state(lineage.state()).unwrap();
    }

    #[test]
    fn signed_root_reroll_preserves_a_childs_immutable_parent_nature() {
        let mut lineage = Lineage::new("root", spawnable_founder(), 1).unwrap();
        let parent_nature_at_spawn = lineage.node("root").unwrap().nature.nature_id.clone();
        lineage
            .spawn_child("root", "root", "child", 2, grant(1))
            .unwrap();
        let child_parent_nature = lineage
            .node("child")
            .unwrap()
            .nature
            .parent_nature_id
            .clone()
            .unwrap();
        assert_eq!(child_parent_nature, parent_nature_at_spawn);

        let rerolled = lineage.node("root").unwrap().nature.reroll().unwrap();
        assert_ne!(rerolled.nature_id, parent_nature_at_spawn);
        lineage
            .update_root_nature("root", "root", rerolled)
            .unwrap();
        validate_state(lineage.state()).unwrap();
        assert_eq!(
            lineage.state().spawn_records[0].parent_nature_id,
            parent_nature_at_spawn
        );
        assert_eq!(
            lineage
                .node("child")
                .unwrap()
                .nature
                .parent_nature_id
                .as_deref(),
            Some(child_parent_nature.as_str())
        );
    }

    #[test]
    fn requires_identity_binding_while_absorption_can_be_autonomous() {
        let mut lineage = Lineage::new("root", spawnable_founder(), 1).unwrap();
        assert!(matches!(
            lineage.spawn_child("impostor", "root", "child", 2, grant(1)),
            Err(EvolutionError::Unauthorized(_))
        ));
        lineage
            .spawn_child("root", "root", "child", 2, grant(1))
            .unwrap();
        lineage
            .record_absorption("root", "root", "child", 3, vec![], false)
            .unwrap();
        assert!(matches!(
            lineage.record_absorption("impostor", "root", "child", 3, vec![], true),
            Err(EvolutionError::Unauthorized(_))
        ));
    }

    #[test]
    fn absorption_is_audited_and_never_reuses_inactive_nodes() {
        let mut lineage = Lineage::new("root", spawnable_founder(), 1).unwrap();
        lineage
            .spawn_child("root", "root", "child", 2, grant(1))
            .unwrap();
        let digest = "a".repeat(64);
        let record = lineage
            .record_absorption("root", "root", "child", 3, vec![digest.clone()], true)
            .unwrap();
        assert_eq!(record.knowledge_hashes, vec![digest]);
        assert_eq!(
            lineage.node("child").unwrap().lifecycle,
            TentacleLifecycle::Absorbed {
                into: "root".to_owned(),
                at_ms: 3,
            }
        );
        assert!(
            lineage
                .record_absorption("child", "child", "root", 4, vec![], true)
                .is_err()
        );
        validate_state(lineage.state()).unwrap();
    }

    #[test]
    fn lineage_selects_binding_death_absorption_candidates() {
        let mut lineage = Lineage::new("root", spawnable_founder(), 1).unwrap();
        assert_eq!(
            lineage
                .lifecycle_decision("root", LifecycleSignal::Death)
                .unwrap(),
            LifecycleDecision::BeginDeath {
                absorption_candidates: vec![]
            }
        );
        lineage
            .spawn_child("root", "root", "child", 2, grant(1))
            .unwrap();
        assert_eq!(
            lineage
                .lifecycle_decision("child", LifecycleSignal::Death)
                .unwrap(),
            LifecycleDecision::BeginDeath {
                absorption_candidates: vec!["root".to_owned()]
            }
        );
    }

    fn survival_binding() -> SurvivalSpendBinding {
        SurvivalSpendBinding {
            expenditure_basis_points: 500,
            chain_id: 8_453,
            token_contract: [2; 20],
            treasury_address: [3; 20],
            configuration_identity: [4; 32],
            exact_amount: ExactTokenAmount {
                total_supply_whole: 1_000_000_000,
                token_decimals: 18,
                basis_points: 500,
                raw_amount: exact_raw_token_amount(1_000_000_000, 18, 500).unwrap(),
            },
        }
    }

    #[test]
    fn death_actions_are_durable_idempotent_and_shutdown_wins_at_grace_expiry() {
        let mut state = LifecycleState::new("root", true).unwrap();
        state
            .schedule_death(&"a".repeat(64), 1_000, 86_400_000, Some(survival_binding()))
            .unwrap();
        state
            .enqueue_absorption(
                1_000,
                "root".to_owned(),
                "sibling".to_owned(),
                "a".repeat(64),
            )
            .unwrap();
        assert!(matches!(
            state.next_due_action().unwrap().action,
            LifecycleAction::Absorb { .. }
        ));
        let absorption_id = state.next_due_action().unwrap().action_id.clone();
        state
            .acknowledge_action(LifecycleReceipt {
                action_id: absorption_id,
                completed_at_ms: 2_000,
                status: LifecycleReceiptStatus::Failed,
                external_reference: None,
                detail: Some("transfer attempt failed".to_owned()),
                confirmed_chain_receipt: None,
                confirmed_transfer_receipt: None,
                provision_receipt: None,
            })
            .unwrap();
        state
            .reconcile_expired_death(86_401_000, Some("sibling".to_owned()))
            .unwrap();
        let shutdown = state.next_due_action().unwrap();
        assert!(matches!(shutdown.action, LifecycleAction::Shutdown { .. }));
        let shutdown_id = shutdown.action_id.clone();
        let receipt = LifecycleReceipt {
            action_id: shutdown_id,
            completed_at_ms: 86_401_000,
            status: LifecycleReceiptStatus::Succeeded,
            external_reference: Some("process-controller-ack".to_owned()),
            detail: None,
            confirmed_chain_receipt: None,
            confirmed_transfer_receipt: None,
            provision_receipt: None,
        };
        assert!(state.acknowledge_action(receipt.clone()).unwrap().1);
        assert!(!state.acknowledge_action(receipt).unwrap().1);
        assert_eq!(state.shutdown_completed_at_ms, Some(86_401_000));
        state.validate().unwrap();
    }

    #[test]
    fn only_confirmed_in_grace_survival_receipt_cancels_death() {
        let mut state = LifecycleState::new("root", true).unwrap();
        state
            .schedule_death(&"b".repeat(64), 1_000, 10_000, Some(survival_binding()))
            .unwrap();
        let action_id = state.next_due_action().unwrap().action_id.clone();
        let receipt = LifecycleReceipt {
            action_id,
            completed_at_ms: 5_000,
            status: LifecycleReceiptStatus::Succeeded,
            external_reference: None,
            detail: None,
            confirmed_chain_receipt: Some(ConfirmedChainReceipt {
                chain_id: 8_453,
                transaction_hash: format!("0x{}", "c".repeat(64)),
                block_number: 42,
                block_timestamp_unix_seconds: 5,
                token_contract: [2; 20],
                from_address: [3; 20],
                burn_destination: [0; 20],
                configuration_identity: [4; 32],
                exact_amount: survival_binding().exact_amount,
                operation: TokenSpendOperation::Burn,
            }),
            confirmed_transfer_receipt: None,
            provision_receipt: None,
        };
        state.acknowledge_action(receipt).unwrap();
        assert!(!state.death_pending());
        state.validate().unwrap();
    }

    #[test]
    fn receipt_time_and_exact_death_binding_prevent_cross_death_replay() {
        let mut state = LifecycleState::new("root", true).unwrap();
        let first_judgment = "1".repeat(64);
        state
            .schedule_death(&first_judgment, 1_000, 10_000, Some(survival_binding()))
            .unwrap();
        let first_spend = state
            .intents
            .values()
            .find(|intent| matches!(intent.action, LifecycleAction::SpendForSurvival { .. }))
            .unwrap()
            .clone();
        let mut first_receipt = LifecycleReceipt {
            action_id: first_spend.action_id,
            completed_at_ms: 999,
            status: LifecycleReceiptStatus::Succeeded,
            external_reference: None,
            detail: None,
            confirmed_chain_receipt: Some(ConfirmedChainReceipt {
                chain_id: 8_453,
                transaction_hash: format!("0x{}", "2".repeat(64)),
                block_number: 7,
                block_timestamp_unix_seconds: 2,
                token_contract: [2; 20],
                from_address: [3; 20],
                burn_destination: [0; 20],
                configuration_identity: [4; 32],
                exact_amount: survival_binding().exact_amount,
                operation: TokenSpendOperation::Burn,
            }),
            confirmed_transfer_receipt: None,
            provision_receipt: None,
        };
        assert!(state.acknowledge_action(first_receipt.clone()).is_err());
        first_receipt.completed_at_ms = 2_000;
        assert!(state.acknowledge_action(first_receipt.clone()).unwrap().1);
        assert!(!state.death_pending());

        let second_judgment = "3".repeat(64);
        state
            .schedule_death(&second_judgment, 20_000, 10_000, Some(survival_binding()))
            .unwrap();
        state
            .enqueue_absorption(
                20_000,
                "root".to_owned(),
                "sibling".to_owned(),
                second_judgment.clone(),
            )
            .unwrap();
        assert!(!state.acknowledge_action(first_receipt).unwrap().1);
        assert_eq!(
            state.pending_death.as_ref().unwrap().judgment_id,
            second_judgment
        );
        let second_absorption = state
            .intents
            .values()
            .find(|intent| {
                matches!(
                    &intent.action,
                    LifecycleAction::Absorb { judgment_id, .. }
                        if judgment_id == &second_judgment
                )
            })
            .unwrap();
        assert!(
            !state
                .canceled_action_ids
                .contains(&second_absorption.action_id)
        );
        let second_spend = state
            .intents
            .values()
            .find(|intent| {
                matches!(
                    &intent.action,
                    LifecycleAction::SpendForSurvival { judgment_id, .. }
                        if judgment_id == &second_judgment
                )
            })
            .unwrap();
        let mut replayed_transaction = state
            .receipts
            .iter()
            .find(|receipt| receipt.action_id != second_spend.action_id)
            .unwrap()
            .clone();
        replayed_transaction.action_id = second_spend.action_id.clone();
        replayed_transaction.completed_at_ms = 21_000;
        assert!(
            state
                .acknowledge_action(replayed_transaction.clone())
                .unwrap_err()
                .to_string()
                .contains("exact burn action")
        );
        replayed_transaction
            .confirmed_chain_receipt
            .as_mut()
            .unwrap()
            .block_timestamp_unix_seconds = 21;
        assert!(
            state
                .acknowledge_action(replayed_transaction)
                .unwrap_err()
                .to_string()
                .contains("already consumed")
        );
        state.validate().unwrap();
    }

    #[test]
    fn expired_death_attempts_absorption_then_reaches_shutdown_without_a_receipt() {
        let mut state = LifecycleState::new("root", true).unwrap();
        let judgment_id = "4".repeat(64);
        state
            .schedule_death(&judgment_id, 1_000, 1_000, Some(survival_binding()))
            .unwrap();
        let absorption = state
            .enqueue_absorption(1_000, "root".to_owned(), "sibling".to_owned(), judgment_id)
            .unwrap()
            .clone();
        state
            .enqueue_spawn(
                1_000,
                "root".to_owned(),
                "pending-child".to_owned(),
                "6".repeat(64),
                spawnable_founder().inherit().unwrap().nature,
                "evolution-runtime".to_owned(),
                "7".repeat(64),
            )
            .unwrap();
        let spend = state
            .intents
            .values()
            .find(|intent| matches!(intent.action, LifecycleAction::SpendForSurvival { .. }))
            .unwrap()
            .action_id
            .clone();
        let predeadline_excluded = BTreeSet::from([absorption.action_id.clone(), spend]);
        assert!(
            state
                .next_due_action_excluding(&predeadline_excluded)
                .is_none(),
            "pending Spawn must not provision while its parent is dying"
        );
        state
            .reconcile_expired_death(2_000, Some("sibling".to_owned()))
            .unwrap();

        assert_eq!(
            state.next_due_action().unwrap().action_id,
            absorption.action_id
        );
        let excluded = BTreeSet::from([absorption.action_id.clone()]);
        let shutdown = state.next_due_action_excluding(&excluded).unwrap();
        assert!(matches!(
            &shutdown.action,
            LifecycleAction::Shutdown {
                after_action_id: Some(dependency),
                ..
            } if dependency == &absorption.action_id
        ));
        assert!(state.intents.values().any(|intent| {
            matches!(intent.action, LifecycleAction::SpendForSurvival { .. })
                && state.canceled_action_ids.contains(&intent.action_id)
        }));
        state.validate().unwrap();
    }

    #[test]
    fn inherited_child_can_be_a_local_root_without_fabricating_its_parent_node() {
        let parent = spawnable_founder();
        let inherited = parent.inherit().unwrap().nature;
        let lineage = Lineage::new_child_root("child", "parent", inherited.clone(), 10).unwrap();
        assert_eq!(
            lineage.state().external_parent_id.as_deref(),
            Some("parent")
        );
        assert_eq!(lineage.node("child").unwrap().nature, inherited);
        assert_eq!(
            lineage.family("child").unwrap().parent.as_deref(),
            Some("parent")
        );
        validate_state(lineage.state()).unwrap();
    }

    #[test]
    fn inherited_root_can_record_absorption_into_its_external_parent() {
        let inherited = spawnable_founder().inherit().unwrap().nature;
        let mut lineage =
            Lineage::new_child_root("child", "external-parent", inherited, 10).unwrap();
        let record = lineage
            .record_external_parent_absorption("child", "external-parent", 20, vec!["5".repeat(64)])
            .unwrap();
        assert_eq!(record.target_id, "external-parent");
        assert_eq!(
            lineage.node("child").unwrap().lifecycle,
            TentacleLifecycle::Absorbed {
                into: "external-parent".to_owned(),
                at_ms: 20,
            }
        );
        validate_state(lineage.state()).unwrap();
    }

    #[test]
    fn hostile_cycles_are_rejected_on_load() {
        let mut lineage = Lineage::new("root", spawnable_founder(), 1).unwrap();
        lineage
            .spawn_child("root", "root", "child", 2, grant(1))
            .unwrap();
        let mut state = lineage.state().clone();
        state.nodes.get_mut("root").unwrap().parent_id = Some("child".to_owned());
        state
            .nodes
            .get_mut("child")
            .unwrap()
            .children
            .push("root".to_owned());
        assert!(Lineage::from_state(state).is_err());
    }

    #[test]
    fn hostile_absorption_cycles_are_rejected_on_load() {
        let mut lineage = Lineage::new("root", spawnable_founder(), 1).unwrap();
        lineage
            .spawn_child("root", "root", "child", 2, grant(1))
            .unwrap();
        let mut state = lineage.state().clone();
        state.nodes.get_mut("root").unwrap().lifecycle = TentacleLifecycle::Absorbed {
            into: "child".to_owned(),
            at_ms: 3,
        };
        state.nodes.get_mut("child").unwrap().lifecycle = TentacleLifecycle::Absorbed {
            into: "root".to_owned(),
            at_ms: 4,
        };
        state.absorption_records = vec![
            AbsorptionRecord {
                source_id: "root".to_owned(),
                target_id: "child".to_owned(),
                at_ms: 3,
                knowledge_hashes: vec![],
                operator_confirmed: true,
            },
            AbsorptionRecord {
                source_id: "child".to_owned(),
                target_id: "root".to_owned(),
                at_ms: 4,
                knowledge_hashes: vec![],
                operator_confirmed: true,
            },
        ];
        state.revision = 3;
        assert!(Lineage::from_state(state).is_err());
    }

    #[test]
    fn lineage_state_round_trips_atomically() {
        let root = tempfile::tempdir().unwrap();
        let store = LineageStore::new(root.path()).unwrap();
        let mut lineage = Lineage::new("root", spawnable_founder(), 1).unwrap();
        lineage
            .spawn_child("root", "root", "child", 2, grant(1))
            .unwrap();
        store.save(&lineage).unwrap();
        let loaded = store.load().unwrap().unwrap();
        assert_eq!(loaded.node("child").unwrap().generation, 1);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(root.path().join("state"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn persistence_rejects_symlinked_state() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        fs::write(&target, b"{}").unwrap();
        fs::create_dir(root.path().join("state")).unwrap();
        symlink(&target, root.path().join("state/lineage.json")).unwrap();
        assert!(LineageStore::new(root.path()).is_err());
    }
}
