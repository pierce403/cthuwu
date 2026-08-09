//! Bounded lineage and graceful lifecycle bookkeeping for Tentacles.
//!
//! This module deliberately has no process-control capability. A lifecycle
//! decision is a recommendation which an authenticated operator must confirm;
//! recording an absorption also requires explicit operator confirmation.

use crate::personality::{SacredBan, TentacleNature};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub const LINEAGE_SCHEMA_VERSION: u32 = 1;
pub const MAX_LINEAGE_NODES: usize = 4_096;
pub const MAX_CHILDREN_PER_TENTACLE: usize = 256;
pub const MAX_LINEAGE_GENERATION: u64 = 1_024;
pub const MAX_ABSORPTION_RECORDS: usize = 8_192;
pub const MAX_ABSORBED_KNOWLEDGE_HASHES: usize = 256;
const MAX_LINEAGE_STATE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ID_BYTES: usize = 128;

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
    pub operator_confirmed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineageState {
    pub schema_version: u32,
    pub revision: u64,
    pub root_id: String,
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
    DeathRecommended,
}

/// A side-effect-free result. The runtime or operator remains responsible for
/// any routing, absorption, or graceful shutdown work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleDecision {
    Continue,
    Warn,
    AwaitOperatorAbsorption { candidates: Vec<String> },
    AwaitOperatorShutdown,
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
            nodes: BTreeMap::from([(founder_id, node)]),
            spawn_records: Vec::new(),
            absorption_records: Vec::new(),
        };
        Ok(Self { state })
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
        if nature.generation != 0 || nature.parent_nature_id.is_some() {
            return Err(EvolutionError::Invalid(
                "the lineage root must retain founder metadata".to_owned(),
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

    /// Creates a child using `TentacleNature::inherit`. `authenticated_parent`
    /// must come from the authenticated transport/runtime identity, never from
    /// an untrusted lineage message.
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
        if self.state.nodes.len() >= MAX_LINEAGE_NODES {
            return Err(EvolutionError::Limit("node count"));
        }
        if self.state.nodes.contains_key(&child_id) {
            return Err(EvolutionError::Conflict(format!(
                "Tentacle {child_id} already exists"
            )));
        }
        if self
            .state
            .spawn_records
            .iter()
            .any(|record| record.authorization_judgment_id == authorization.judgment_id)
        {
            return Err(EvolutionError::Conflict(
                "the final propagation judgment was already consumed".to_owned(),
            ));
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
            if parent.nature.sacred_ban == SacredBan::Spawning {
                return Err(EvolutionError::Conflict(
                    "the parent Nature has a sacred spawning ban".to_owned(),
                ));
            }
            if parent.children.len() >= MAX_CHILDREN_PER_TENTACLE {
                return Err(EvolutionError::Limit("children per Tentacle"));
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
        if generation > MAX_LINEAGE_GENERATION {
            return Err(EvolutionError::Limit("generation"));
        }

        let inherited = parent_nature
            .inherit()
            .map_err(|error| EvolutionError::Nature(error.to_string()))?;
        let child_nature = inherited.nature;
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

    /// Records a confirmed knowledge absorption. This never stops either
    /// process and does not itself copy knowledge; hashes are audit references.
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
        if !operator_confirmed {
            return Err(EvolutionError::Unauthorized(
                "absorption requires explicit operator confirmation".to_owned(),
            ));
        }
        validate_id(authenticated_target, "target ID")?;
        validate_id(source_id, "source ID")?;
        if authenticated_target == source_id {
            return Err(EvolutionError::Conflict(
                "a Tentacle cannot absorb itself".to_owned(),
            ));
        }
        if self.state.absorption_records.len() >= MAX_ABSORPTION_RECORDS {
            return Err(EvolutionError::Limit("absorption record"));
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
            parent: node.parent_id.clone(),
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
            if result.len() >= MAX_LINEAGE_GENERATION as usize {
                return Err(EvolutionError::Invalid(
                    "ancestor traversal exceeded the generation bound".to_owned(),
                ));
            }
            result.push(parent_id.clone());
            current =
                self.state.nodes.get(parent_id).ok_or_else(|| {
                    EvolutionError::Invalid(format!("missing parent {parent_id}"))
                })?;
        }
        Ok(result)
    }

    /// Produces a recommendation only. It performs no routing or process exit.
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
            LifecycleSignal::DeathRecommended => {
                let mut candidates = BTreeSet::new();
                let family = self.family(tentacle_id)?;
                candidates.extend(family.parent);
                candidates.extend(family.siblings);
                candidates.extend(family.children);
                let candidates: Vec<_> = candidates
                    .into_iter()
                    .filter(|id| {
                        self.state.nodes.get(id).is_some_and(|candidate| {
                            candidate.lifecycle == TentacleLifecycle::Active
                        })
                    })
                    .collect();
                if candidates.is_empty() {
                    LifecycleDecision::AwaitOperatorShutdown
                } else {
                    LifecycleDecision::AwaitOperatorAbsorption { candidates }
                }
            }
        })
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
        let encoded = serde_json::to_vec_pretty(lineage.state())?;
        if encoded.len() as u64 > MAX_LINEAGE_STATE_BYTES {
            return Err(EvolutionError::Limit("serialized state size"));
        }
        atomic_write(&self.state_directory, &self.path, &encoded)?;
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
        if metadata.len() > MAX_LINEAGE_STATE_BYTES {
            return Err(EvolutionError::Limit("serialized state size"));
        }
        restrict_file_path(&self.path)?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        File::open(&self.path)?
            .take(MAX_LINEAGE_STATE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_LINEAGE_STATE_BYTES {
            return Err(EvolutionError::Limit("serialized state size"));
        }
        let state: LineageState = serde_json::from_slice(&bytes)?;
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
    if state.nodes.is_empty() || state.nodes.len() > MAX_LINEAGE_NODES {
        return Err(EvolutionError::Limit("node count"));
    }
    if state.spawn_records.len() != state.nodes.len().saturating_sub(1) {
        return Err(EvolutionError::Invalid(
            "spawn records must describe every non-root node exactly once".to_owned(),
        ));
    }
    if state.absorption_records.len() > MAX_ABSORPTION_RECORDS {
        return Err(EvolutionError::Limit("absorption record"));
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
    if root.parent_id.is_some() || root.generation != 0 {
        return Err(EvolutionError::Invalid(
            "root must have no parent and generation zero".to_owned(),
        ));
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
        if node.generation > MAX_LINEAGE_GENERATION {
            return Err(EvolutionError::Limit("generation"));
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
        if node.children.len() > MAX_CHILDREN_PER_TENTACLE {
            return Err(EvolutionError::Limit("children per Tentacle"));
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
                if node.nature.parent_nature_id.is_some() {
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
    let mut consumed_judgments = BTreeSet::new();
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
        if !consumed_judgments.insert(&record.authorization_judgment_id) {
            return Err(EvolutionError::Invalid(
                "a propagation judgment was consumed more than once".to_owned(),
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
        if !record.operator_confirmed {
            return Err(EvolutionError::Invalid(
                "unconfirmed absorption in persisted state".to_owned(),
            ));
        }
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
        let target = state
            .nodes
            .get(&record.target_id)
            .ok_or_else(|| EvolutionError::Invalid("absorption target is missing".to_owned()))?;
        if record.at_ms < source.spawned_at_ms || record.at_ms < target.spawned_at_ms {
            return Err(EvolutionError::Invalid(
                "absorption predates a referenced Tentacle".to_owned(),
            ));
        }
        if !active.remove(&record.source_id) || !active.contains(&record.target_id) {
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

fn atomic_write(directory: &Path, path: &Path, bytes: &[u8]) -> Result<(), EvolutionError> {
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    restrict_open_file(temporary.as_file())?;
    temporary.write_all(bytes)?;
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
    fn requires_identity_binding_and_operator_confirmation() {
        let mut lineage = Lineage::new("root", spawnable_founder(), 1).unwrap();
        assert!(matches!(
            lineage.spawn_child("impostor", "root", "child", 2, grant(1)),
            Err(EvolutionError::Unauthorized(_))
        ));
        lineage
            .spawn_child("root", "root", "child", 2, grant(1))
            .unwrap();
        assert!(matches!(
            lineage.record_absorption("root", "root", "child", 3, vec![], false),
            Err(EvolutionError::Unauthorized(_))
        ));
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
    fn graceful_lifecycle_only_returns_operator_decisions() {
        let mut lineage = Lineage::new("root", spawnable_founder(), 1).unwrap();
        assert_eq!(
            lineage
                .lifecycle_decision("root", LifecycleSignal::DeathRecommended)
                .unwrap(),
            LifecycleDecision::AwaitOperatorShutdown
        );
        lineage
            .spawn_child("root", "root", "child", 2, grant(1))
            .unwrap();
        assert_eq!(
            lineage
                .lifecycle_decision("child", LifecycleSignal::DeathRecommended)
                .unwrap(),
            LifecycleDecision::AwaitOperatorAbsorption {
                candidates: vec!["root".to_owned()]
            }
        );
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
