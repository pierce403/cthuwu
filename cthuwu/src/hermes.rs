//! Decentralized, opportunistic knowledge gossip between Tentacles.
//!
//! Hermes is a protocol pattern, not a router or privileged node. Every
//! `HermesNode` keeps its own bounded peer set, verifies messages against the
//! transport-authenticated peer, and reconciles state through anti-entropy.
//! HMAC keys are configured out of band and are never persisted here. Because
//! HMAC is symmetric, transport identity binding remains mandatory.

use crate::personality::SacredBan;
use crate::storage::{constant_time_eq, hmac_sha256};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub const HERMES_SCHEMA_VERSION: u32 = 1;
pub const HERMES_ENVELOPE_VERSION: u32 = 1;
pub const MAX_GOSSIP_PEERS: usize = 128;
pub const MAX_KNOWLEDGE_ITEMS: usize = 2_048;
pub const MAX_PENDING_OUTBOUND: usize = 8_192;
pub const MAX_PATH_HOPS: usize = 16;
pub const MAX_ANTI_ENTROPY_ENTRIES: usize = MAX_KNOWLEDGE_ITEMS;
pub const MAX_OUTBOUND_BATCH: usize = 64;
pub const MAX_SKILL_BYTES: usize = 16 * 1024;
const MAX_ENVELOPE_BYTES: usize = 24 * 1024;
const MAX_STATE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ID_BYTES: usize = 128;
const MAX_TOOL_NAME_BYTES: usize = 64;
const MAX_SKILL_NAME_BYTES: usize = 64;
const MAX_SKILL_LINES: usize = 512;
const MAX_SKILL_LINE_BYTES: usize = 1_024;
const MIN_KEY_BYTES: usize = 32;
const MAX_KEY_BYTES: usize = 4 * 1024;
const KNOWLEDGE_ID_DOMAIN: &[u8] = b"cthuwu-hermes-knowledge-id-v1\0";
const AUTHOR_SIGNATURE_DOMAIN: &[u8] = b"cthuwu-hermes-author-v1\0";
const RELAY_SIGNATURE_DOMAIN: &[u8] = b"cthuwu-hermes-relay-v1\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureAuthority {
    Peer,
    Operator,
}

impl SignatureAuthority {
    const fn rank(self) -> u8 {
        match self {
            Self::Peer => 0,
            Self::Operator => 1,
        }
    }
}

/// A runtime-only HMAC identity. Its key is intentionally neither serializable
/// nor included in `Debug` output.
#[derive(Clone)]
pub struct SigningIdentity {
    key_id: String,
    secret: Vec<u8>,
}

impl SigningIdentity {
    pub fn new(key_id: impl Into<String>, secret: Vec<u8>) -> Result<Self, HermesError> {
        let key_id = key_id.into();
        validate_id(&key_id, "key ID")?;
        validate_secret(&secret)?;
        Ok(Self { key_id, secret })
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }
}

impl fmt::Debug for SigningIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SigningIdentity")
            .field("key_id", &self.key_id)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone)]
struct TrustedKey {
    secret: Vec<u8>,
    authority: SignatureAuthority,
}

/// Runtime trust configuration. It is deliberately not serializable.
#[derive(Clone, Default)]
pub struct TrustedKeyring {
    keys: BTreeMap<String, TrustedKey>,
}

impl fmt::Debug for TrustedKeyring {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrustedKeyring")
            .field("key_ids", &self.keys.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl TrustedKeyring {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trust(
        &mut self,
        identity: &SigningIdentity,
        authority: SignatureAuthority,
    ) -> Result<(), HermesError> {
        if self.keys.len() >= MAX_GOSSIP_PEERS.saturating_mul(2)
            && !self.keys.contains_key(identity.key_id())
        {
            return Err(HermesError::Limit("trusted key count"));
        }
        match self.keys.get(identity.key_id()) {
            Some(existing)
                if existing.authority != authority
                    || !constant_time_eq(&existing.secret, &identity.secret) =>
            {
                Err(HermesError::Conflict(
                    "trusted key ID is already bound differently".to_owned(),
                ))
            }
            Some(_) => Ok(()),
            None => {
                self.keys.insert(
                    identity.key_id.clone(),
                    TrustedKey {
                        secret: identity.secret.clone(),
                        authority,
                    },
                );
                Ok(())
            }
        }
    }

    pub fn authority(&self, key_id: &str) -> Option<SignatureAuthority> {
        self.keys.get(key_id).map(|key| key.authority)
    }

    fn trusted(&self, identity: &SigningIdentity) -> bool {
        self.keys
            .get(identity.key_id())
            .is_some_and(|trusted| constant_time_eq(&trusted.secret, &identity.secret))
    }

    fn verify(&self, key_id: &str, message: &[u8], signature: &str) -> bool {
        let Some(key) = self.keys.get(key_id) else {
            return false;
        };
        let Ok(signature) = decode_hex_32(signature) else {
            return false;
        };
        constant_time_eq(&hmac_sha256(&key.secret, message), &signature)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionPatternKind {
    FirstTurnEngagement,
    ReturnEngagement,
    Clarification,
    ConsentBoundary,
    TopicChange,
}

impl InteractionPatternKind {
    const fn code(self) -> &'static str {
        match self {
            Self::FirstTurnEngagement => "first-turn-engagement",
            Self::ReturnEngagement => "return-engagement",
            Self::Clarification => "clarification",
            Self::ConsentBoundary => "consent-boundary",
            Self::TopicChange => "topic-change",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationStrategyKind {
    AnswerFirst,
    OneQuestionAtATime,
    AskForClarification,
    RespectTopicChange,
    ExplainUncertainty,
}

impl ConversationStrategyKind {
    const fn code(self) -> &'static str {
        match self {
            Self::AnswerFirst => "answer-first",
            Self::OneQuestionAtATime => "one-question-at-a-time",
            Self::AskForClarification => "ask-for-clarification",
            Self::RespectTopicChange => "respect-topic-change",
            Self::ExplainUncertainty => "explain-uncertainty",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOperationKind {
    Discover,
    Read,
    Search,
    Transform,
    Validate,
}

impl ToolOperationKind {
    const fn code(self) -> &'static str {
        match self {
            Self::Discover => "discover",
            Self::Read => "read",
            Self::Search => "search",
            Self::Transform => "transform",
            Self::Validate => "validate",
        }
    }
}

/// Aggregates only. There is intentionally no user identifier or text field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnonymizedInteractionPattern {
    pub pattern: InteractionPatternKind,
    pub observations: u32,
    pub successes: u32,
}

/// A closed strategy category plus aggregate results, never a transcript.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationStrategy {
    pub strategy: ConversationStrategyKind,
    pub attempts: u32,
    pub successes: u32,
}

/// Tool names are bounded identifiers; arguments, output, paths, and user
/// content are intentionally not representable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolUsagePattern {
    pub tool_name: String,
    pub operation: ToolOperationKind,
    pub attempts: u32,
    pub successes: u32,
}

/// Operator-authored reusable guidance. Construction and inbound validation
/// reject common private-memory, contact, credential, and user-ID forms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorSkill {
    pub name: String,
    pub version: u32,
    pub instructions: String,
}

impl OperatorSkill {
    pub fn new(
        name: impl Into<String>,
        version: u32,
        instructions: impl Into<String>,
    ) -> Result<Self, HermesError> {
        let skill = Self {
            name: name.into(),
            version,
            instructions: instructions.into(),
        };
        skill.validate()?;
        Ok(skill)
    }

    fn validate(&self) -> Result<(), HermesError> {
        validate_slug(&self.name, MAX_SKILL_NAME_BYTES, "skill name")?;
        if self.version == 0 {
            return Err(HermesError::Invalid(
                "skill version must be positive".to_owned(),
            ));
        }
        validate_public_skill_text(&self.instructions)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "knowledge_type", content = "value", rename_all = "snake_case")]
pub enum KnowledgePayload {
    AnonymizedInteractionPattern(AnonymizedInteractionPattern),
    ConversationStrategy(ConversationStrategy),
    ToolUsagePattern(ToolUsagePattern),
    OperatorCreatedSkill(OperatorSkill),
}

impl KnowledgePayload {
    fn validate(&self) -> Result<(), HermesError> {
        match self {
            Self::AnonymizedInteractionPattern(pattern) => validate_counts(
                pattern.observations,
                pattern.successes,
                "interaction pattern",
            ),
            Self::ConversationStrategy(strategy) => validate_counts(
                strategy.attempts,
                strategy.successes,
                "conversation strategy",
            ),
            Self::ToolUsagePattern(pattern) => {
                validate_slug(&pattern.tool_name, MAX_TOOL_NAME_BYTES, "tool name")?;
                validate_counts(pattern.attempts, pattern.successes, "tool pattern")
            }
            Self::OperatorCreatedSkill(skill) => skill.validate(),
        }
    }

    fn logical_key(&self) -> String {
        match self {
            Self::AnonymizedInteractionPattern(pattern) => {
                format!("interaction:{}", pattern.pattern.code())
            }
            Self::ConversationStrategy(strategy) => {
                format!("strategy:{}", strategy.strategy.code())
            }
            Self::ToolUsagePattern(pattern) => {
                format!("tool:{}:{}", pattern.tool_name, pattern.operation.code())
            }
            Self::OperatorCreatedSkill(skill) => format!("skill:{}", skill.name),
        }
    }
}

/// The ID is derived from a closed logical key, so callers cannot smuggle a
/// user or contact identifier into gossip metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeItem {
    pub id: String,
    pub payload: KnowledgePayload,
}

impl KnowledgeItem {
    pub fn new(payload: KnowledgePayload) -> Result<Self, HermesError> {
        payload.validate()?;
        let id = knowledge_id(&payload.logical_key());
        Ok(Self { id, payload })
    }

    pub fn validate(&self) -> Result<(), HermesError> {
        self.payload.validate()?;
        let expected = knowledge_id(&self.payload.logical_key());
        if self.id != expected {
            return Err(HermesError::Invalid(
                "knowledge ID does not match its privacy-safe logical key".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredKnowledge {
    pub envelope_version: u32,
    pub item: KnowledgeItem,
    pub created_at_ms: u64,
    pub author_key_id: String,
    pub author_signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeEnvelope {
    pub envelope_version: u32,
    pub authored: AuthoredKnowledge,
    pub path: Vec<String>,
    pub relay_peer_id: String,
    pub relay_key_id: String,
    pub relay_signature: String,
}

impl KnowledgeEnvelope {
    pub fn knowledge_id(&self) -> &str {
        &self.authored.item.id
    }

    pub fn digest(&self) -> Result<String, HermesError> {
        authored_digest(&self.authored)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredKnowledge {
    pub authored: AuthoredKnowledge,
    pub digest: String,
    pub best_path: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerState {
    pub peer_id: String,
    pub relay_key_id: String,
    pub last_sync_ms: Option<u64>,
    pub known_digests: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOutbound {
    pub peer_id: String,
    pub knowledge_id: String,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HermesState {
    pub schema_version: u32,
    pub local_peer_id: String,
    pub local_relay_key_id: String,
    pub peers: BTreeMap<String, PeerState>,
    pub knowledge: BTreeMap<String, StoredKnowledge>,
    pub pending_outbound: Vec<PendingOutbound>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeDigest {
    pub knowledge_id: String,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AntiEntropySummary {
    pub schema_version: u32,
    pub peer_id: String,
    pub entries: Vec<KnowledgeDigest>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AntiEntropyPlan {
    pub offer_to_peer: Vec<String>,
    pub request_from_peer: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    Inserted,
    Replaced,
    Unchanged,
}

#[derive(Debug)]
pub enum HermesError {
    Invalid(String),
    Unauthorized(String),
    Conflict(String),
    UnknownPeer(String),
    Limit(&'static str),
    Privacy(String),
    Io(io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for HermesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid Hermes data: {message}"),
            Self::Unauthorized(message) => {
                write!(formatter, "Hermes authentication failed: {message}")
            }
            Self::Conflict(message) => write!(formatter, "Hermes conflict: {message}"),
            Self::UnknownPeer(peer) => write!(formatter, "unknown Hermes peer {peer}"),
            Self::Limit(name) => write!(formatter, "Hermes {name} limit exceeded"),
            Self::Privacy(message) => write!(
                formatter,
                "Hermes privacy boundary rejected data: {message}"
            ),
            Self::Io(error) => write!(formatter, "Hermes state I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "Hermes state JSON is invalid: {error}"),
        }
    }
}

impl Error for HermesError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for HermesError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for HermesError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Clone, Debug)]
pub struct HermesNode {
    state: HermesState,
    relay_identity: SigningIdentity,
    keyring: TrustedKeyring,
    memory_sharing_banned: bool,
}

impl HermesNode {
    pub fn new(
        local_peer_id: impl Into<String>,
        relay_identity: SigningIdentity,
        keyring: TrustedKeyring,
        sacred_ban: SacredBan,
    ) -> Result<Self, HermesError> {
        let local_peer_id = local_peer_id.into();
        validate_id(&local_peer_id, "local peer ID")?;
        if !keyring.trusted(&relay_identity) {
            return Err(HermesError::Unauthorized(
                "local relay key is not in the trusted keyring".to_owned(),
            ));
        }
        let state = HermesState {
            schema_version: HERMES_SCHEMA_VERSION,
            local_peer_id,
            local_relay_key_id: relay_identity.key_id.clone(),
            peers: BTreeMap::new(),
            knowledge: BTreeMap::new(),
            pending_outbound: Vec::new(),
        };
        Ok(Self {
            state,
            relay_identity,
            keyring,
            memory_sharing_banned: sacred_ban == SacredBan::MemorySharing,
        })
    }

    pub fn from_state(
        state: HermesState,
        relay_identity: SigningIdentity,
        keyring: TrustedKeyring,
        sacred_ban: SacredBan,
    ) -> Result<Self, HermesError> {
        if !keyring.trusted(&relay_identity) {
            return Err(HermesError::Unauthorized(
                "local relay key is not in the trusted keyring".to_owned(),
            ));
        }
        validate_state(&state, &keyring)?;
        if state.local_relay_key_id != relay_identity.key_id {
            return Err(HermesError::Unauthorized(
                "persisted local peer is bound to a different relay key".to_owned(),
            ));
        }
        let mut node = Self {
            state,
            relay_identity,
            keyring,
            memory_sharing_banned: sacred_ban == SacredBan::MemorySharing,
        };
        if node.memory_sharing_banned {
            node.state.pending_outbound.clear();
        }
        Ok(node)
    }

    pub fn state(&self) -> &HermesState {
        &self.state
    }

    pub fn local_peer_id(&self) -> &str {
        &self.state.local_peer_id
    }

    pub fn knowledge(&self, id: &str) -> Option<&StoredKnowledge> {
        self.state.knowledge.get(id)
    }

    pub fn operator_skill(&self, name: &str) -> Option<&OperatorSkill> {
        let id = knowledge_id(&format!("skill:{name}"));
        self.state.knowledge.get(&id).and_then(|record| {
            if let KnowledgePayload::OperatorCreatedSkill(skill) = &record.authored.item.payload {
                Some(skill)
            } else {
                None
            }
        })
    }

    pub fn set_sacred_ban(&mut self, sacred_ban: SacredBan) {
        self.memory_sharing_banned = sacred_ban == SacredBan::MemorySharing;
        if self.memory_sharing_banned {
            self.state.pending_outbound.clear();
        } else {
            self.schedule_all_for_all_peers();
        }
    }

    pub fn can_send(&self) -> bool {
        !self.memory_sharing_banned
    }

    /// Adds one direct opportunistic peer. There is no global peer registry or
    /// central route table in this state machine.
    pub fn connect_peer(
        &mut self,
        peer_id: impl Into<String>,
        relay_key_id: impl Into<String>,
    ) -> Result<(), HermesError> {
        let peer_id = peer_id.into();
        let relay_key_id = relay_key_id.into();
        validate_id(&peer_id, "peer ID")?;
        validate_id(&relay_key_id, "relay key ID")?;
        if peer_id == self.state.local_peer_id {
            return Err(HermesError::Conflict(
                "a node cannot gossip with itself".to_owned(),
            ));
        }
        if self.keyring.authority(&relay_key_id).is_none() {
            return Err(HermesError::Unauthorized(
                "peer relay key is not trusted".to_owned(),
            ));
        }
        if let Some(existing) = self.state.peers.get(&peer_id) {
            if existing.relay_key_id == relay_key_id {
                return Ok(());
            }
            return Err(HermesError::Conflict(
                "peer ID is already bound to another relay key".to_owned(),
            ));
        }
        if self.state.peers.len() >= MAX_GOSSIP_PEERS {
            return Err(HermesError::Limit("peer count"));
        }
        self.state.peers.insert(
            peer_id.clone(),
            PeerState {
                peer_id: peer_id.clone(),
                relay_key_id,
                last_sync_ms: None,
                known_digests: BTreeMap::new(),
            },
        );
        if self.can_send() {
            let items: Vec<_> = self
                .state
                .knowledge
                .iter()
                .map(|(id, record)| (id.clone(), record.digest.clone()))
                .collect();
            for (id, digest) in items {
                self.schedule(&peer_id, &id, &digest);
            }
        }
        Ok(())
    }

    pub fn publish(
        &mut self,
        item: KnowledgeItem,
        created_at_ms: u64,
        author: &SigningIdentity,
    ) -> Result<MergeOutcome, HermesError> {
        item.validate()?;
        if !self.keyring.trusted(author) {
            return Err(HermesError::Unauthorized(
                "author key is not trusted".to_owned(),
            ));
        }
        let authority = self
            .keyring
            .authority(author.key_id())
            .ok_or_else(|| HermesError::Unauthorized("author key is unknown".to_owned()))?;
        if matches!(&item.payload, KnowledgePayload::OperatorCreatedSkill(_))
            && authority != SignatureAuthority::Operator
        {
            return Err(HermesError::Unauthorized(
                "operator-created skills require an operator-authority signature".to_owned(),
            ));
        }
        let mut authored = AuthoredKnowledge {
            envelope_version: HERMES_ENVELOPE_VERSION,
            item,
            created_at_ms,
            author_key_id: author.key_id.clone(),
            author_signature: String::new(),
        };
        authored.author_signature = sign_author(&authored, author)?;
        let digest = authored_digest(&authored)?;
        let record = StoredKnowledge {
            authored,
            digest,
            best_path: vec![self.state.local_peer_id.clone()],
        };
        self.merge_record(record, authority, None)
    }

    /// Returns `None` under the memory-sharing Sacred Ban; even hashes are not
    /// emitted by a receive-only Tentacle.
    pub fn outbound_summary(&self) -> Option<AntiEntropySummary> {
        if !self.can_send() {
            return None;
        }
        Some(AntiEntropySummary {
            schema_version: HERMES_SCHEMA_VERSION,
            peer_id: self.state.local_peer_id.clone(),
            entries: self
                .state
                .knowledge
                .iter()
                .map(|(knowledge_id, record)| KnowledgeDigest {
                    knowledge_id: knowledge_id.clone(),
                    digest: record.digest.clone(),
                })
                .collect(),
        })
    }

    /// Applies a direct peer summary. The returned requests are empty when the
    /// local Nature forbids sending, so the node remains strictly receive-only.
    pub fn apply_summary(
        &mut self,
        authenticated_peer: &str,
        summary: &AntiEntropySummary,
        received_at_ms: u64,
    ) -> Result<AntiEntropyPlan, HermesError> {
        self.require_peer(authenticated_peer)?;
        validate_summary(summary)?;
        if summary.peer_id != authenticated_peer {
            return Err(HermesError::Unauthorized(
                "transport peer does not match summary peer".to_owned(),
            ));
        }
        let remote: BTreeMap<_, _> = summary
            .entries
            .iter()
            .map(|entry| (entry.knowledge_id.clone(), entry.digest.clone()))
            .collect();
        let local: Vec<_> = self
            .state
            .knowledge
            .iter()
            .map(|(id, record)| (id.clone(), record.digest.clone()))
            .collect();

        {
            let peer = self
                .state
                .peers
                .get_mut(authenticated_peer)
                .expect("peer was checked above");
            peer.last_sync_ms = Some(received_at_ms);
            peer.known_digests = remote.clone();
        }

        if !self.can_send() {
            return Ok(AntiEntropyPlan::default());
        }
        let mut plan = AntiEntropyPlan::default();
        for (id, digest) in &local {
            if remote.get(id) != Some(digest) {
                plan.offer_to_peer.push(id.clone());
                self.schedule(authenticated_peer, id, digest);
            }
        }
        for (id, digest) in &remote {
            if self
                .state
                .knowledge
                .get(id)
                .is_none_or(|record| &record.digest != digest)
            {
                plan.request_from_peer.push(id.clone());
            }
        }
        Ok(plan)
    }

    /// Produces, but does not dequeue, a retry-safe bounded batch. Call
    /// `acknowledge` only after the transport peer confirms receipt.
    pub fn outbound_batch(
        &self,
        peer_id: &str,
        requested_limit: usize,
    ) -> Result<Vec<KnowledgeEnvelope>, HermesError> {
        self.require_peer(peer_id)?;
        if !self.can_send() || requested_limit == 0 {
            return Ok(Vec::new());
        }
        let limit = requested_limit.min(MAX_OUTBOUND_BATCH);
        let mut batch = Vec::new();
        let mut included = BTreeSet::new();
        for pending in self
            .state
            .pending_outbound
            .iter()
            .filter(|pending| pending.peer_id == peer_id)
        {
            if batch.len() == limit {
                break;
            }
            let Some(record) = self.state.knowledge.get(&pending.knowledge_id) else {
                continue;
            };
            if record.digest != pending.digest {
                continue;
            }
            let reset_path = record.best_path.iter().any(|hop| hop == peer_id);
            batch.push(self.relay_envelope(record, reset_path)?);
            included.insert(pending.knowledge_id.clone());
        }

        // A capped pending queue is an optimization, not a durability boundary.
        // Periodic opportunities also scan the peer's bounded digest view, so
        // queue pressure cannot permanently strand knowledge (including when a
        // receive-only peer is unable to request a backfill).
        if batch.len() < limit {
            let peer = self.require_peer(peer_id)?;
            for (id, record) in &self.state.knowledge {
                if batch.len() == limit {
                    break;
                }
                if included.contains(id) || peer.known_digests.get(id) == Some(&record.digest) {
                    continue;
                }
                let reset_path = record.best_path.iter().any(|hop| hop == peer_id);
                batch.push(self.relay_envelope(record, reset_path)?);
                included.insert(id.clone());
            }
        }
        Ok(batch)
    }

    pub fn acknowledge(
        &mut self,
        authenticated_peer: &str,
        acknowledgements: &[KnowledgeDigest],
        received_at_ms: u64,
    ) -> Result<(), HermesError> {
        self.require_peer(authenticated_peer)?;
        if acknowledgements.len() > MAX_OUTBOUND_BATCH {
            return Err(HermesError::Limit("acknowledgement batch"));
        }
        let mut accepted = BTreeMap::new();
        for acknowledgement in acknowledgements {
            validate_knowledge_digest(acknowledgement)?;
            let current = self
                .state
                .knowledge
                .get(&acknowledgement.knowledge_id)
                .ok_or_else(|| {
                    HermesError::Invalid("acknowledgement references unknown knowledge".to_owned())
                })?;
            if current.digest != acknowledgement.digest {
                return Err(HermesError::Invalid(
                    "acknowledgement digest is stale or false".to_owned(),
                ));
            }
            accepted.insert(
                acknowledgement.knowledge_id.clone(),
                acknowledgement.digest.clone(),
            );
        }
        let peer = self
            .state
            .peers
            .get_mut(authenticated_peer)
            .expect("peer was checked above");
        peer.last_sync_ms = Some(received_at_ms);
        peer.known_digests.extend(accepted.clone());
        self.state.pending_outbound.retain(|pending| {
            pending.peer_id != authenticated_peer
                || accepted.get(&pending.knowledge_id) != Some(&pending.digest)
        });
        Ok(())
    }

    pub fn receive(
        &mut self,
        authenticated_peer: &str,
        envelope: KnowledgeEnvelope,
        received_at_ms: u64,
    ) -> Result<MergeOutcome, HermesError> {
        let expected_key = self.require_peer(authenticated_peer)?.relay_key_id.clone();
        validate_envelope_shape(&envelope)?;
        if envelope.relay_peer_id != authenticated_peer
            || envelope.path.last().map(String::as_str) != Some(authenticated_peer)
        {
            return Err(HermesError::Unauthorized(
                "transport peer does not match the signed relay path".to_owned(),
            ));
        }
        if envelope.relay_key_id != expected_key {
            return Err(HermesError::Unauthorized(
                "relay key is not the key pinned for this peer".to_owned(),
            ));
        }
        if envelope
            .path
            .iter()
            .any(|hop| hop == &self.state.local_peer_id)
        {
            return Err(HermesError::Invalid(
                "gossip path loops through the receiving node".to_owned(),
            ));
        }
        verify_authored(&envelope.authored, &self.keyring)?;
        verify_relay(&envelope, &self.keyring)?;
        let authority = self
            .keyring
            .authority(&envelope.authored.author_key_id)
            .ok_or_else(|| HermesError::Unauthorized("author key is unknown".to_owned()))?;
        if matches!(
            &envelope.authored.item.payload,
            KnowledgePayload::OperatorCreatedSkill(_)
        ) && authority != SignatureAuthority::Operator
        {
            return Err(HermesError::Unauthorized(
                "operator-created skill lacks operator authority".to_owned(),
            ));
        }

        let digest = authored_digest(&envelope.authored)?;
        {
            let peer = self
                .state
                .peers
                .get_mut(authenticated_peer)
                .expect("peer was checked above");
            peer.last_sync_ms = Some(received_at_ms);
            peer.known_digests
                .insert(envelope.authored.item.id.clone(), digest.clone());
        }
        let source = authenticated_peer.to_owned();
        let record = StoredKnowledge {
            authored: envelope.authored,
            digest,
            best_path: envelope.path,
        };
        let outcome = self.merge_record(record, authority, Some(&source))?;

        // If the incoming revision lost conflict resolution, anti-entropy sends
        // the selected revision back to the source (unless receive-only).
        if self.can_send() {
            let id = self.state.peers.get(authenticated_peer).and_then(|peer| {
                peer.known_digests.iter().find_map(|(id, remote_digest)| {
                    self.state
                        .knowledge
                        .get(id)
                        .filter(|local| &local.digest != remote_digest)
                        .map(|_| id.clone())
                })
            });
            if let Some(id) = id
                && let Some(record) = self.state.knowledge.get(&id)
            {
                let digest = record.digest.clone();
                self.schedule(authenticated_peer, &id, &digest);
            }
        }
        Ok(outcome)
    }

    fn merge_record(
        &mut self,
        incoming: StoredKnowledge,
        incoming_authority: SignatureAuthority,
        source_peer: Option<&str>,
    ) -> Result<MergeOutcome, HermesError> {
        let id = incoming.authored.item.id.clone();
        let outcome = match self.state.knowledge.get(&id) {
            None => {
                if self.state.knowledge.len() >= MAX_KNOWLEDGE_ITEMS {
                    return Err(HermesError::Limit("knowledge item count"));
                }
                self.state.knowledge.insert(id.clone(), incoming);
                MergeOutcome::Inserted
            }
            Some(current) => {
                let current_authority = self
                    .keyring
                    .authority(&current.authored.author_key_id)
                    .ok_or_else(|| {
                        HermesError::Unauthorized(
                            "stored author key is no longer trusted".to_owned(),
                        )
                    })?;
                match compare_records(current, current_authority, &incoming, incoming_authority) {
                    Ordering::Less => {
                        self.state.knowledge.insert(id.clone(), incoming);
                        MergeOutcome::Replaced
                    }
                    Ordering::Equal | Ordering::Greater => {
                        if current.digest == incoming.digest
                            && compare_paths(&incoming.best_path, &current.best_path)
                                == Ordering::Less
                        {
                            self.state
                                .knowledge
                                .get_mut(&id)
                                .expect("stored record exists")
                                .best_path = incoming.best_path;
                        }
                        MergeOutcome::Unchanged
                    }
                }
            }
        };
        if matches!(outcome, MergeOutcome::Inserted | MergeOutcome::Replaced) && self.can_send() {
            let (digest, path) = {
                let selected = self
                    .state
                    .knowledge
                    .get(&id)
                    .expect("selected record exists");
                (selected.digest.clone(), selected.best_path.clone())
            };
            self.state
                .pending_outbound
                .retain(|pending| pending.knowledge_id != id);
            let peers: Vec<_> = self.state.peers.keys().cloned().collect();
            for peer in peers {
                if source_peer == Some(peer.as_str()) || path.iter().any(|hop| hop == &peer) {
                    continue;
                }
                self.schedule(&peer, &id, &digest);
            }
        }
        Ok(outcome)
    }

    fn relay_envelope(
        &self,
        record: &StoredKnowledge,
        reset_path: bool,
    ) -> Result<KnowledgeEnvelope, HermesError> {
        let mut path = if reset_path {
            vec![self.state.local_peer_id.clone()]
        } else {
            record.best_path.clone()
        };
        if path.last() != Some(&self.state.local_peer_id) {
            if path.len() >= MAX_PATH_HOPS {
                return Err(HermesError::Limit("gossip path"));
            }
            if path.iter().any(|hop| hop == &self.state.local_peer_id) {
                return Err(HermesError::Invalid(
                    "stored path contains a non-terminal local hop".to_owned(),
                ));
            }
            path.push(self.state.local_peer_id.clone());
        }
        let mut envelope = KnowledgeEnvelope {
            envelope_version: HERMES_ENVELOPE_VERSION,
            authored: record.authored.clone(),
            path,
            relay_peer_id: self.state.local_peer_id.clone(),
            relay_key_id: self.relay_identity.key_id.clone(),
            relay_signature: String::new(),
        };
        envelope.relay_signature = sign_relay(&envelope, &self.relay_identity)?;
        validate_envelope_size(&envelope)?;
        Ok(envelope)
    }

    fn require_peer(&self, peer_id: &str) -> Result<&PeerState, HermesError> {
        self.state
            .peers
            .get(peer_id)
            .ok_or_else(|| HermesError::UnknownPeer(peer_id.to_owned()))
    }

    fn schedule(&mut self, peer_id: &str, knowledge_id: &str, digest: &str) {
        if !self.can_send() {
            return;
        }
        if self
            .state
            .peers
            .get(peer_id)
            .and_then(|peer| peer.known_digests.get(knowledge_id))
            .is_some_and(|known| known == digest)
        {
            return;
        }
        if let Some(existing) = self
            .state
            .pending_outbound
            .iter_mut()
            .find(|pending| pending.peer_id == peer_id && pending.knowledge_id == knowledge_id)
        {
            existing.digest = digest.to_owned();
            return;
        }
        if self.state.pending_outbound.len() >= MAX_PENDING_OUTBOUND {
            return;
        }
        self.state.pending_outbound.push(PendingOutbound {
            peer_id: peer_id.to_owned(),
            knowledge_id: knowledge_id.to_owned(),
            digest: digest.to_owned(),
        });
    }

    fn schedule_all_for_all_peers(&mut self) {
        if !self.can_send() {
            return;
        }
        let peers: Vec<_> = self.state.peers.keys().cloned().collect();
        let knowledge: Vec<_> = self
            .state
            .knowledge
            .iter()
            .map(|(id, record)| (id.clone(), record.digest.clone()))
            .collect();
        for peer in peers {
            for (id, digest) in &knowledge {
                self.schedule(&peer, id, digest);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct HermesStore {
    state_directory: PathBuf,
    path: PathBuf,
}

impl HermesStore {
    pub fn new(data_dir: &Path) -> Result<Self, HermesError> {
        require_real_directory(data_dir, "data directory")?;
        restrict_directory(data_dir)?;
        let state_directory = data_dir.join("state");
        ensure_real_private_directory(&state_directory)?;
        let path = state_directory.join("hermes_gossip.json");
        reject_non_regular_if_present(&path)?;
        Ok(Self {
            state_directory,
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn save(&self, node: &HermesNode) -> Result<(), HermesError> {
        validate_state(node.state(), &node.keyring)?;
        reject_non_regular_if_present(&self.path)?;
        let encoded = serde_json::to_vec_pretty(node.state())?;
        if encoded.len() as u64 > MAX_STATE_BYTES {
            return Err(HermesError::Limit("serialized state size"));
        }
        atomic_write(&self.state_directory, &self.path, &encoded)
    }

    pub fn load(&self, keyring: &TrustedKeyring) -> Result<Option<HermesState>, HermesError> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(HermesError::Invalid(format!(
                "{} must be a regular file",
                self.path.display()
            )));
        }
        if metadata.len() > MAX_STATE_BYTES {
            return Err(HermesError::Limit("serialized state size"));
        }
        restrict_file_path(&self.path)?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        File::open(&self.path)?
            .take(MAX_STATE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(HermesError::Limit("serialized state size"));
        }
        let state: HermesState = serde_json::from_slice(&bytes)?;
        validate_state(&state, keyring)?;
        Ok(Some(state))
    }
}

fn compare_records(
    current: &StoredKnowledge,
    current_authority: SignatureAuthority,
    incoming: &StoredKnowledge,
    incoming_authority: SignatureAuthority,
) -> Ordering {
    current_authority
        .rank()
        .cmp(&incoming_authority.rank())
        .then_with(|| {
            current
                .authored
                .created_at_ms
                .cmp(&incoming.authored.created_at_ms)
        })
        .then_with(|| current.digest.cmp(&incoming.digest))
}

fn compare_paths(left: &[String], right: &[String]) -> Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn sign_author(
    authored: &AuthoredKnowledge,
    identity: &SigningIdentity,
) -> Result<String, HermesError> {
    let payload = author_signing_bytes(authored)?;
    Ok(encode_hex(&hmac_sha256(&identity.secret, &payload)))
}

fn sign_relay(
    envelope: &KnowledgeEnvelope,
    identity: &SigningIdentity,
) -> Result<String, HermesError> {
    let payload = relay_signing_bytes(envelope)?;
    Ok(encode_hex(&hmac_sha256(&identity.secret, &payload)))
}

fn verify_authored(
    authored: &AuthoredKnowledge,
    keyring: &TrustedKeyring,
) -> Result<(), HermesError> {
    authored.item.validate()?;
    validate_id(&authored.author_key_id, "author key ID")?;
    validate_hex_digest(&authored.author_signature, "author signature")?;
    if authored.envelope_version != HERMES_ENVELOPE_VERSION {
        return Err(HermesError::Invalid(
            "unsupported authored-envelope version".to_owned(),
        ));
    }
    let bytes = author_signing_bytes(authored)?;
    if !keyring.verify(&authored.author_key_id, &bytes, &authored.author_signature) {
        return Err(HermesError::Unauthorized(
            "invalid author signature".to_owned(),
        ));
    }
    Ok(())
}

fn verify_relay(envelope: &KnowledgeEnvelope, keyring: &TrustedKeyring) -> Result<(), HermesError> {
    let bytes = relay_signing_bytes(envelope)?;
    if !keyring.verify(&envelope.relay_key_id, &bytes, &envelope.relay_signature) {
        return Err(HermesError::Unauthorized(
            "invalid relay signature".to_owned(),
        ));
    }
    Ok(())
}

#[derive(Serialize)]
struct AuthorSigningView<'a> {
    domain: &'static str,
    envelope_version: u32,
    item: &'a KnowledgeItem,
    created_at_ms: u64,
    author_key_id: &'a str,
}

#[derive(Serialize)]
struct RelaySigningView<'a> {
    domain: &'static str,
    envelope_version: u32,
    authored: &'a AuthoredKnowledge,
    path: &'a [String],
    relay_peer_id: &'a str,
    relay_key_id: &'a str,
}

fn author_signing_bytes(authored: &AuthoredKnowledge) -> Result<Vec<u8>, HermesError> {
    let view = AuthorSigningView {
        domain: std::str::from_utf8(AUTHOR_SIGNATURE_DOMAIN).expect("signature domain is UTF-8"),
        envelope_version: authored.envelope_version,
        item: &authored.item,
        created_at_ms: authored.created_at_ms,
        author_key_id: &authored.author_key_id,
    };
    Ok(serde_json::to_vec(&view)?)
}

fn relay_signing_bytes(envelope: &KnowledgeEnvelope) -> Result<Vec<u8>, HermesError> {
    let view = RelaySigningView {
        domain: std::str::from_utf8(RELAY_SIGNATURE_DOMAIN).expect("signature domain is UTF-8"),
        envelope_version: envelope.envelope_version,
        authored: &envelope.authored,
        path: &envelope.path,
        relay_peer_id: &envelope.relay_peer_id,
        relay_key_id: &envelope.relay_key_id,
    };
    Ok(serde_json::to_vec(&view)?)
}

fn authored_digest(authored: &AuthoredKnowledge) -> Result<String, HermesError> {
    let bytes = serde_json::to_vec(authored)?;
    Ok(encode_hex(&Sha256::digest(bytes)))
}

fn knowledge_id(logical_key: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(KNOWLEDGE_ID_DOMAIN);
    digest.update(logical_key.as_bytes());
    encode_hex(&digest.finalize())
}

fn validate_state(state: &HermesState, keyring: &TrustedKeyring) -> Result<(), HermesError> {
    if state.schema_version != HERMES_SCHEMA_VERSION {
        return Err(HermesError::Invalid(format!(
            "unsupported state schema version {}",
            state.schema_version
        )));
    }
    validate_id(&state.local_peer_id, "local peer ID")?;
    validate_id(&state.local_relay_key_id, "local relay key ID")?;
    if keyring.authority(&state.local_relay_key_id).is_none() {
        return Err(HermesError::Unauthorized(
            "persisted local relay key is no longer trusted".to_owned(),
        ));
    }
    if state.peers.len() > MAX_GOSSIP_PEERS {
        return Err(HermesError::Limit("peer count"));
    }
    if state.knowledge.len() > MAX_KNOWLEDGE_ITEMS {
        return Err(HermesError::Limit("knowledge item count"));
    }
    if state.pending_outbound.len() > MAX_PENDING_OUTBOUND {
        return Err(HermesError::Limit("pending outbound count"));
    }
    for (key, peer) in &state.peers {
        validate_id(key, "peer map key")?;
        validate_id(&peer.peer_id, "peer ID")?;
        validate_id(&peer.relay_key_id, "peer relay key ID")?;
        if key != &peer.peer_id || peer.peer_id == state.local_peer_id {
            return Err(HermesError::Invalid(
                "peer map identity is inconsistent".to_owned(),
            ));
        }
        if keyring.authority(&peer.relay_key_id).is_none() {
            return Err(HermesError::Unauthorized(
                "persisted peer relay key is no longer trusted".to_owned(),
            ));
        }
        if peer.known_digests.len() > MAX_KNOWLEDGE_ITEMS {
            return Err(HermesError::Limit("peer digest count"));
        }
        for (id, digest) in &peer.known_digests {
            validate_hex_digest(id, "knowledge ID")?;
            validate_hex_digest(digest, "knowledge digest")?;
        }
    }
    for (key, record) in &state.knowledge {
        validate_hex_digest(key, "knowledge map key")?;
        if key != &record.authored.item.id {
            return Err(HermesError::Invalid(
                "knowledge map key does not match the signed item".to_owned(),
            ));
        }
        verify_authored(&record.authored, keyring)?;
        let digest = authored_digest(&record.authored)?;
        if digest != record.digest {
            return Err(HermesError::Invalid(
                "stored knowledge digest does not match its signed body".to_owned(),
            ));
        }
        validate_path(&record.best_path)?;
        let authority = keyring
            .authority(&record.authored.author_key_id)
            .expect("verified author key exists");
        if matches!(
            &record.authored.item.payload,
            KnowledgePayload::OperatorCreatedSkill(_)
        ) && authority != SignatureAuthority::Operator
        {
            return Err(HermesError::Unauthorized(
                "persisted skill lacks operator authority".to_owned(),
            ));
        }
    }
    let mut unique_pending = BTreeSet::new();
    for pending in &state.pending_outbound {
        validate_id(&pending.peer_id, "pending peer ID")?;
        validate_hex_digest(&pending.knowledge_id, "pending knowledge ID")?;
        validate_hex_digest(&pending.digest, "pending digest")?;
        if !state.peers.contains_key(&pending.peer_id) {
            return Err(HermesError::Invalid(
                "pending outbound references an unknown peer".to_owned(),
            ));
        }
        let record = state.knowledge.get(&pending.knowledge_id).ok_or_else(|| {
            HermesError::Invalid("pending outbound references missing knowledge".to_owned())
        })?;
        if record.digest != pending.digest {
            return Err(HermesError::Invalid(
                "pending outbound digest is stale".to_owned(),
            ));
        }
        if !unique_pending.insert((&pending.peer_id, &pending.knowledge_id)) {
            return Err(HermesError::Invalid(
                "duplicate pending outbound entry".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_envelope_shape(envelope: &KnowledgeEnvelope) -> Result<(), HermesError> {
    validate_envelope_size(envelope)?;
    if envelope.envelope_version != HERMES_ENVELOPE_VERSION {
        return Err(HermesError::Invalid(
            "unsupported relay-envelope version".to_owned(),
        ));
    }
    validate_id(&envelope.relay_peer_id, "relay peer ID")?;
    validate_id(&envelope.relay_key_id, "relay key ID")?;
    validate_hex_digest(&envelope.relay_signature, "relay signature")?;
    validate_path(&envelope.path)?;
    if envelope.path.last() != Some(&envelope.relay_peer_id) {
        return Err(HermesError::Invalid(
            "relay must be the terminal path hop".to_owned(),
        ));
    }
    Ok(())
}

fn validate_envelope_size(envelope: &KnowledgeEnvelope) -> Result<(), HermesError> {
    if serde_json::to_vec(envelope)?.len() > MAX_ENVELOPE_BYTES {
        return Err(HermesError::Limit("encoded envelope size"));
    }
    Ok(())
}

fn validate_path(path: &[String]) -> Result<(), HermesError> {
    if path.is_empty() || path.len() > MAX_PATH_HOPS {
        return Err(HermesError::Limit("gossip path"));
    }
    let mut unique = BTreeSet::new();
    for peer in path {
        validate_id(peer, "path peer ID")?;
        if !unique.insert(peer) {
            return Err(HermesError::Invalid(
                "gossip path contains a cycle".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_summary(summary: &AntiEntropySummary) -> Result<(), HermesError> {
    if summary.schema_version != HERMES_SCHEMA_VERSION {
        return Err(HermesError::Invalid(
            "unsupported anti-entropy schema version".to_owned(),
        ));
    }
    validate_id(&summary.peer_id, "summary peer ID")?;
    if summary.entries.len() > MAX_ANTI_ENTROPY_ENTRIES {
        return Err(HermesError::Limit("anti-entropy entry count"));
    }
    let mut unique = BTreeSet::new();
    for entry in &summary.entries {
        validate_knowledge_digest(entry)?;
        if !unique.insert(&entry.knowledge_id) {
            return Err(HermesError::Invalid(
                "duplicate knowledge ID in anti-entropy summary".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_knowledge_digest(digest: &KnowledgeDigest) -> Result<(), HermesError> {
    validate_hex_digest(&digest.knowledge_id, "knowledge ID")?;
    validate_hex_digest(&digest.digest, "knowledge digest")
}

fn validate_counts(total: u32, successes: u32, description: &str) -> Result<(), HermesError> {
    if total == 0 || successes > total || total > 1_000_000_000 {
        return Err(HermesError::Invalid(format!(
            "{description} aggregate counts are invalid"
        )));
    }
    Ok(())
}

fn validate_id(value: &str, description: &str) -> Result<(), HermesError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(HermesError::Invalid(format!(
            "{description} is empty, oversized, or contains unsafe characters"
        )));
    }
    Ok(())
}

fn validate_slug(value: &str, max: usize, description: &str) -> Result<(), HermesError> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || value.starts_with('-')
        || value.ends_with('-')
    {
        return Err(HermesError::Invalid(format!(
            "{description} must be a bounded lowercase slug"
        )));
    }
    Ok(())
}

fn validate_hex_digest(value: &str, description: &str) -> Result<(), HermesError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(HermesError::Invalid(format!(
            "{description} must be lowercase SHA-256 hex"
        )));
    }
    Ok(())
}

fn validate_secret(secret: &[u8]) -> Result<(), HermesError> {
    if !(MIN_KEY_BYTES..=MAX_KEY_BYTES).contains(&secret.len()) {
        return Err(HermesError::Invalid(format!(
            "HMAC key must contain {MIN_KEY_BYTES} to {MAX_KEY_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_public_skill_text(text: &str) -> Result<(), HermesError> {
    if text.is_empty() || text.len() > MAX_SKILL_BYTES {
        return Err(HermesError::Limit("operator skill size"));
    }
    let lines: Vec<_> = text.lines().collect();
    if lines.len() > MAX_SKILL_LINES || lines.iter().any(|line| line.len() > MAX_SKILL_LINE_BYTES) {
        return Err(HermesError::Limit("operator skill line count or length"));
    }
    if text.chars().any(|character| {
        character == '\0' || (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    }) {
        return Err(HermesError::Privacy(
            "skill contains unsafe control characters".to_owned(),
        ));
    }
    let lower = text.to_ascii_lowercase();
    const PRIVATE_MARKERS: &[&str] = &[
        "contacts/",
        "contact note",
        "private memory",
        "state/agent/operators",
        "senderinboxid",
        "sender_inbox_id",
        "inbox id",
        "begin private key",
        "authorization: bearer",
        "api_key=",
        "api-key=",
        "secret_key=",
        "xmtp database",
    ];
    if PRIVATE_MARKERS.iter().any(|marker| lower.contains(marker)) {
        return Err(HermesError::Privacy(
            "skill resembles contact notes, private memory, credentials, or user identity data"
                .to_owned(),
        ));
    }
    if contains_long_hex_identifier(text.as_bytes(), 64)
        || contains_prefixed_hex_identifier(text.as_bytes(), 40)
        || contains_email_like_identifier(text)
    {
        return Err(HermesError::Privacy(
            "skill contains a likely inbox, wallet, or email identifier".to_owned(),
        ));
    }
    Ok(())
}

fn contains_long_hex_identifier(bytes: &[u8], length: usize) -> bool {
    bytes
        .windows(length)
        .any(|window| window.iter().all(u8::is_ascii_hexdigit))
}

fn contains_prefixed_hex_identifier(bytes: &[u8], hex_length: usize) -> bool {
    bytes
        .windows(hex_length + 2)
        .any(|window| window.starts_with(b"0x") && window[2..].iter().all(u8::is_ascii_hexdigit))
}

fn contains_email_like_identifier(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        let token = token.trim_matches(|character: char| {
            matches!(
                character,
                '<' | '>' | '(' | ')' | '[' | ']' | ',' | ';' | ':' | '"' | '\''
            )
        });
        let Some((local, domain)) = token.split_once('@') else {
            return false;
        };
        !local.is_empty()
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
    })
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex_32(value: &str) -> Result<[u8; 32], HermesError> {
    validate_hex_digest(value, "signature")?;
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = decode_nibble(pair[0])
            .ok_or_else(|| HermesError::Invalid("signature contains invalid hex".to_owned()))?;
        let low = decode_nibble(pair[1])
            .ok_or_else(|| HermesError::Invalid("signature contains invalid hex".to_owned()))?;
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn require_real_directory(path: &Path, description: &str) -> Result<(), HermesError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(HermesError::Invalid(format!(
            "{description} must be a real directory"
        )));
    }
    Ok(())
}

fn ensure_real_private_directory(path: &Path) -> Result<(), HermesError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(HermesError::Invalid(format!(
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

fn reject_non_regular_if_present(path: &Path) -> Result<(), HermesError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            HermesError::Invalid(format!("{} must be a regular file", path.display())),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn atomic_write(directory: &Path, path: &Path, bytes: &[u8]) -> Result<(), HermesError> {
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    restrict_open_file(temporary.as_file())?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| HermesError::Io(error.error))?;
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

    fn identity(name: &str, byte: u8) -> SigningIdentity {
        SigningIdentity::new(name, vec![byte; 32]).unwrap()
    }

    fn keyring(keys: &[(&SigningIdentity, SignatureAuthority)]) -> TrustedKeyring {
        let mut keyring = TrustedKeyring::new();
        for (identity, authority) in keys {
            keyring.trust(identity, *authority).unwrap();
        }
        keyring
    }

    fn strategy(kind: ConversationStrategyKind, attempts: u32, successes: u32) -> KnowledgeItem {
        KnowledgeItem::new(KnowledgePayload::ConversationStrategy(
            ConversationStrategy {
                strategy: kind,
                attempts,
                successes,
            },
        ))
        .unwrap()
    }

    fn digests(batch: &[KnowledgeEnvelope]) -> Vec<KnowledgeDigest> {
        batch
            .iter()
            .map(|envelope| KnowledgeDigest {
                knowledge_id: envelope.knowledge_id().to_owned(),
                digest: envelope.digest().unwrap(),
            })
            .collect()
    }

    fn deliver(
        sender: &mut HermesNode,
        receiver: &mut HermesNode,
        now: u64,
    ) -> Result<usize, HermesError> {
        let batch = sender.outbound_batch(receiver.local_peer_id(), MAX_OUTBOUND_BATCH)?;
        let acknowledgements = digests(&batch);
        let sender_id = sender.local_peer_id().to_owned();
        for envelope in batch {
            receiver.receive(&sender_id, envelope, now)?;
        }
        if !acknowledgements.is_empty() {
            sender.acknowledge(receiver.local_peer_id(), &acknowledgements, now)?;
        }
        Ok(acknowledgements.len())
    }

    fn reconcile(
        left: &mut HermesNode,
        right: &mut HermesNode,
        now: u64,
    ) -> Result<(), HermesError> {
        if let Some(summary) = left.outbound_summary() {
            right.apply_summary(left.local_peer_id(), &summary, now)?;
        }
        if let Some(summary) = right.outbound_summary() {
            left.apply_summary(right.local_peer_id(), &summary, now)?;
        }
        deliver(left, right, now)?;
        deliver(right, left, now)?;
        Ok(())
    }

    #[test]
    fn signed_envelopes_bind_author_relay_path_and_transport_peer() {
        let a = identity("key-a", 1);
        let b = identity("key-b", 2);
        let keys = keyring(&[
            (&a, SignatureAuthority::Peer),
            (&b, SignatureAuthority::Peer),
        ]);
        let mut node_a =
            HermesNode::new("peer-a", a.clone(), keys.clone(), SacredBan::Profit).unwrap();
        let mut node_b = HermesNode::new("peer-b", b.clone(), keys, SacredBan::Profit).unwrap();
        node_a.connect_peer("peer-b", b.key_id()).unwrap();
        node_b.connect_peer("peer-a", a.key_id()).unwrap();
        node_a
            .publish(
                strategy(ConversationStrategyKind::AnswerFirst, 5, 4),
                10,
                &a,
            )
            .unwrap();
        let mut envelope = node_a.outbound_batch("peer-b", 1).unwrap().remove(0);
        envelope.authored.created_at_ms += 1;
        assert!(matches!(
            node_b.receive("peer-a", envelope, 11),
            Err(HermesError::Unauthorized(_))
        ));

        let envelope = node_a.outbound_batch("peer-b", 1).unwrap().remove(0);
        assert!(matches!(
            node_b.receive("spoofed-peer", envelope, 11),
            Err(HermesError::UnknownPeer(_))
        ));
    }

    #[test]
    fn conflict_resolution_prefers_operator_then_timestamp_then_digest() {
        let a = identity("key-a", 1);
        let b = identity("key-b", 2);
        let operator = identity("operator", 9);
        let keys = keyring(&[
            (&a, SignatureAuthority::Peer),
            (&b, SignatureAuthority::Peer),
            (&operator, SignatureAuthority::Operator),
        ]);
        let mut node = HermesNode::new("peer-a", a.clone(), keys, SacredBan::Profit).unwrap();
        let id = strategy(ConversationStrategyKind::AnswerFirst, 10, 8).id;
        node.publish(
            strategy(ConversationStrategyKind::AnswerFirst, 10, 8),
            100,
            &a,
        )
        .unwrap();
        node.publish(
            strategy(ConversationStrategyKind::AnswerFirst, 2, 2),
            1,
            &operator,
        )
        .unwrap();
        let selected = node.knowledge(&id).unwrap();
        assert_eq!(selected.authored.author_key_id, operator.key_id());
        node.publish(
            strategy(ConversationStrategyKind::AnswerFirst, 99, 90),
            10_000,
            &b,
        )
        .unwrap();
        assert_eq!(
            node.knowledge(&id).unwrap().authored.author_key_id,
            operator.key_id()
        );

        // Same-authority, monotonically increasing timestamps always select
        // the latest revision across a property-style range of aggregates.
        let other_id = strategy(ConversationStrategyKind::ExplainUncertainty, 1, 1).id;
        for timestamp in 1..100 {
            node.publish(
                strategy(
                    ConversationStrategyKind::ExplainUncertainty,
                    timestamp as u32,
                    timestamp as u32,
                ),
                timestamp,
                &a,
            )
            .unwrap();
        }
        assert_eq!(
            node.knowledge(&other_id).unwrap().authored.created_at_ms,
            99
        );
    }

    #[test]
    fn memory_sharing_ban_receives_but_never_emits() {
        let a = identity("key-a", 1);
        let b = identity("key-b", 2);
        let keys = keyring(&[
            (&a, SignatureAuthority::Peer),
            (&b, SignatureAuthority::Peer),
        ]);
        let mut sender =
            HermesNode::new("peer-a", a.clone(), keys.clone(), SacredBan::Profit).unwrap();
        let mut receiver =
            HermesNode::new("peer-b", b.clone(), keys, SacredBan::MemorySharing).unwrap();
        sender.connect_peer("peer-b", b.key_id()).unwrap();
        receiver.connect_peer("peer-a", a.key_id()).unwrap();
        let item = strategy(ConversationStrategyKind::RespectTopicChange, 3, 3);
        let id = item.id.clone();
        sender.publish(item, 1, &a).unwrap();
        assert_eq!(deliver(&mut sender, &mut receiver, 2).unwrap(), 1);
        assert!(receiver.knowledge(&id).is_some());
        assert!(receiver.outbound_summary().is_none());
        assert!(receiver.outbound_batch("peer-a", 64).unwrap().is_empty());
        assert!(receiver.state.pending_outbound.is_empty());
    }

    #[test]
    fn operator_skill_propagates_across_siblings_without_a_router() {
        let a = identity("key-a", 1);
        let b = identity("key-b", 2);
        let c = identity("key-c", 3);
        let operator = identity("operator", 9);
        let keys = keyring(&[
            (&a, SignatureAuthority::Peer),
            (&b, SignatureAuthority::Peer),
            (&c, SignatureAuthority::Peer),
            (&operator, SignatureAuthority::Operator),
        ]);
        let mut node_a =
            HermesNode::new("peer-a", a.clone(), keys.clone(), SacredBan::Profit).unwrap();
        let mut node_b =
            HermesNode::new("peer-b", b.clone(), keys.clone(), SacredBan::Profit).unwrap();
        let mut node_c = HermesNode::new("peer-c", c.clone(), keys, SacredBan::Profit).unwrap();
        node_a.connect_peer("peer-b", b.key_id()).unwrap();
        node_b.connect_peer("peer-a", a.key_id()).unwrap();
        node_b.connect_peer("peer-c", c.key_id()).unwrap();
        node_c.connect_peer("peer-b", b.key_id()).unwrap();

        let skill = OperatorSkill::new(
            "bounded-search",
            1,
            "Search only the configured root. Bound results and report truncation.",
        )
        .unwrap();
        node_a
            .publish(
                KnowledgeItem::new(KnowledgePayload::OperatorCreatedSkill(skill)).unwrap(),
                1,
                &operator,
            )
            .unwrap();
        deliver(&mut node_a, &mut node_b, 2).unwrap();
        deliver(&mut node_b, &mut node_c, 3).unwrap();
        assert_eq!(node_c.operator_skill("bounded-search").unwrap().version, 1);
    }

    #[test]
    fn anti_entropy_heals_a_partition_and_converges() {
        let a = identity("key-a", 1);
        let b = identity("key-b", 2);
        let c = identity("key-c", 3);
        let keys = keyring(&[
            (&a, SignatureAuthority::Peer),
            (&b, SignatureAuthority::Peer),
            (&c, SignatureAuthority::Peer),
        ]);
        let mut node_a =
            HermesNode::new("peer-a", a.clone(), keys.clone(), SacredBan::Profit).unwrap();
        let mut node_b =
            HermesNode::new("peer-b", b.clone(), keys.clone(), SacredBan::Profit).unwrap();
        let mut node_c = HermesNode::new("peer-c", c.clone(), keys, SacredBan::Profit).unwrap();
        node_a.connect_peer("peer-b", b.key_id()).unwrap();
        node_b.connect_peer("peer-a", a.key_id()).unwrap();
        node_a
            .publish(strategy(ConversationStrategyKind::AnswerFirst, 4, 4), 1, &a)
            .unwrap();
        node_c
            .publish(
                strategy(ConversationStrategyKind::AskForClarification, 4, 3),
                1,
                &c,
            )
            .unwrap();
        reconcile(&mut node_a, &mut node_b, 2).unwrap();
        assert_eq!(node_a.state.knowledge.len(), 1);
        assert_eq!(node_c.state.knowledge.len(), 1);

        // The partition heals through a direct B-C opportunity; A never needs
        // to know C is present and no central component coordinates the flow.
        node_b.connect_peer("peer-c", c.key_id()).unwrap();
        node_c.connect_peer("peer-b", b.key_id()).unwrap();
        for now in 3..8 {
            reconcile(&mut node_b, &mut node_c, now).unwrap();
            reconcile(&mut node_a, &mut node_b, now).unwrap();
        }
        let ids_a: Vec<_> = node_a.state.knowledge.keys().cloned().collect();
        let ids_b: Vec<_> = node_b.state.knowledge.keys().cloned().collect();
        let ids_c: Vec<_> = node_c.state.knowledge.keys().cloned().collect();
        assert_eq!(ids_a, ids_b);
        assert_eq!(ids_b, ids_c);
        assert_eq!(ids_a.len(), 2);
    }

    #[test]
    fn privacy_shape_rejects_contacts_identifiers_and_private_memory() {
        assert!(OperatorSkill::new("bad-contact", 1, "Read contacts/person.md").is_err());
        assert!(
            OperatorSkill::new("bad-inbox", 1, format!("Remember {}", "a".repeat(64))).is_err()
        );
        assert!(OperatorSkill::new("bad-email", 1, "Message person@example.com").is_err());
        let pattern =
            KnowledgePayload::AnonymizedInteractionPattern(AnonymizedInteractionPattern {
                pattern: InteractionPatternKind::ReturnEngagement,
                observations: 10,
                successes: 8,
            });
        assert!(KnowledgeItem::new(pattern).is_ok());
    }

    #[test]
    fn persistence_round_trips_without_keys_and_is_owner_only() {
        let a = identity("key-a", 0xab);
        let keys = keyring(&[(&a, SignatureAuthority::Peer)]);
        let mut node =
            HermesNode::new("peer-a", a.clone(), keys.clone(), SacredBan::Profit).unwrap();
        node.publish(strategy(ConversationStrategyKind::AnswerFirst, 2, 2), 1, &a)
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let store = HermesStore::new(root.path()).unwrap();
        store.save(&node).unwrap();
        let bytes = fs::read(store.path()).unwrap();
        assert!(
            !bytes
                .windows(32)
                .any(|window| window.iter().all(|byte| *byte == 0xab))
        );
        let state = store.load(&keys).unwrap().unwrap();
        let loaded = HermesNode::from_state(state, a, keys, SacredBan::Profit).unwrap();
        assert_eq!(loaded.state.knowledge.len(), 1);

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
    fn persistence_rejects_symlinked_gossip_state() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        fs::write(&target, b"{}").unwrap();
        fs::create_dir(root.path().join("state")).unwrap();
        symlink(&target, root.path().join("state/hermes_gossip.json")).unwrap();
        assert!(HermesStore::new(root.path()).is_err());
    }
}
