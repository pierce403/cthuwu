use crate::validation::{bounded_count, bounded_optional_text, bounded_text, validate_slug};
use crate::{
    AcknowledgementId, CapabilityManifest, CapabilityName, ContentHash, CthulhuId, Incarnation,
    LeaseId, MessageId, PrivacyProperty, PropagationId, ProposalId, ProtocolVersion, RegistryRef,
    RequestId, SessionId, Tentacle, TentacleId, TentacleLifecycleUpdate, Timestamp, TrustMechanism,
    ValidationError, ValidationErrorKind, XmtpInboxRef,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt};

const MAX_REQUIREMENTS: usize = 64;
const MAX_PROVENANCE_DEPTH: usize = 32;
const MAX_ARGUMENT_HASHES: usize = 16;
const MAX_ROUTE_WINDOW_SECONDS: i64 = 3_600;
const MAX_LEASE_LIFETIME_SECONDS: i64 = 24 * 60 * 60;
const MAX_GOVERNANCE_LIFETIME_SECONDS: i64 = 31 * 24 * 60 * 60;
const MAX_PROPAGATION_LIFETIME_SECONDS: i64 = 30 * 24 * 60 * 60;
const PROVENANCE_HASH_DOMAIN: &str = "cthuwu-council-provenance-v1";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum MessageType {
    #[serde(rename = "council.member.announce")]
    CouncilMemberAnnounce,
    #[serde(rename = "council.member.withdraw")]
    CouncilMemberWithdraw,
    #[serde(rename = "tentacle.announce")]
    TentacleAnnounce,
    #[serde(rename = "tentacle.capabilities")]
    TentacleCapabilities,
    #[serde(rename = "tentacle.heartbeat")]
    TentacleHeartbeat,
    #[serde(rename = "tentacle.draining")]
    TentacleDraining,
    #[serde(rename = "tentacle.withdraw")]
    TentacleWithdraw,
    #[serde(rename = "route.request")]
    RouteRequest,
    #[serde(rename = "route.offer")]
    RouteOffer,
    #[serde(rename = "route.reject")]
    RouteReject,
    #[serde(rename = "route.award")]
    RouteAward,
    #[serde(rename = "lease.grant")]
    LeaseGrant,
    #[serde(rename = "lease.renew")]
    LeaseRenew,
    #[serde(rename = "lease.release")]
    LeaseRelease,
    #[serde(rename = "lease.revoke")]
    LeaseRevoke,
    #[serde(rename = "lease.expired")]
    LeaseExpired,
    #[serde(rename = "governance.proposal")]
    GovernanceProposal,
    #[serde(rename = "governance.argument")]
    GovernanceArgument,
    #[serde(rename = "governance.vote")]
    GovernanceVote,
    #[serde(rename = "governance.ratified")]
    GovernanceRatified,
    #[serde(rename = "governance.rejected")]
    GovernanceRejected,
    #[serde(rename = "propagation.invite")]
    PropagationInvite,
    #[serde(rename = "propagation.accept")]
    PropagationAccept,
    #[serde(rename = "propagation.reject")]
    PropagationReject,
    #[serde(rename = "propagation.announce")]
    PropagationAnnounce,
    #[serde(rename = "propagation.forward")]
    PropagationForward,
    #[serde(rename = "propagation.ack")]
    PropagationAck,
    #[serde(rename = "propagation.revoke")]
    PropagationRevoke,
}

impl MessageType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CouncilMemberAnnounce => "council.member.announce",
            Self::CouncilMemberWithdraw => "council.member.withdraw",
            Self::TentacleAnnounce => "tentacle.announce",
            Self::TentacleCapabilities => "tentacle.capabilities",
            Self::TentacleHeartbeat => "tentacle.heartbeat",
            Self::TentacleDraining => "tentacle.draining",
            Self::TentacleWithdraw => "tentacle.withdraw",
            Self::RouteRequest => "route.request",
            Self::RouteOffer => "route.offer",
            Self::RouteReject => "route.reject",
            Self::RouteAward => "route.award",
            Self::LeaseGrant => "lease.grant",
            Self::LeaseRenew => "lease.renew",
            Self::LeaseRelease => "lease.release",
            Self::LeaseRevoke => "lease.revoke",
            Self::LeaseExpired => "lease.expired",
            Self::GovernanceProposal => "governance.proposal",
            Self::GovernanceArgument => "governance.argument",
            Self::GovernanceVote => "governance.vote",
            Self::GovernanceRatified => "governance.ratified",
            Self::GovernanceRejected => "governance.rejected",
            Self::PropagationInvite => "propagation.invite",
            Self::PropagationAccept => "propagation.accept",
            Self::PropagationReject => "propagation.reject",
            Self::PropagationAnnounce => "propagation.announce",
            Self::PropagationForward => "propagation.forward",
            Self::PropagationAck => "propagation.ack",
            Self::PropagationRevoke => "propagation.revoke",
        }
    }
}

impl fmt::Display for MessageType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CouncilMemberAnnounce {
    pub member: crate::CthulhuIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CouncilMemberWithdraw {
    pub cthulhu_id: CthulhuId,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TentacleAnnounce {
    pub tentacle: Tentacle,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TentacleCapabilities {
    pub tentacle_id: TentacleId,
    pub owner: CthulhuId,
    pub incarnation: Incarnation,
    pub capabilities: CapabilityManifest,
    pub observed_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TentacleHeartbeat {
    pub update: TentacleLifecycleUpdate,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TentacleDraining {
    pub tentacle_id: TentacleId,
    pub owner: CthulhuId,
    pub incarnation: Incarnation,
    pub effective_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TentacleWithdraw {
    pub tentacle_id: TentacleId,
    pub owner: CthulhuId,
    pub incarnation: Incarnation,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum UserReference {
    XmtpInbox(XmtpInboxRef),
    Opaque(String),
}

impl UserReference {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::XmtpInbox(_) => Ok(()),
            Self::Opaque(value) => validate_slug("userReference", value, 128),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrustPolicy {
    pub allowlisted_only: bool,
    pub registry_association_required: bool,
    pub accepted_mechanisms: Vec<TrustMechanism>,
    pub accepted_registries: Vec<RegistryRef>,
}

impl TrustPolicy {
    pub fn validate(&self) -> Result<(), ValidationError> {
        bounded_count(
            "routing.trustPolicy.acceptedMechanisms",
            self.accepted_mechanisms.len(),
            16,
        )?;
        bounded_count(
            "routing.trustPolicy.acceptedRegistries",
            self.accepted_registries.len(),
            16,
        )?;
        for registry in &self.accepted_registries {
            registry.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoutingRequirements {
    pub protocol_versions: Vec<ProtocolVersion>,
    pub model_classes: Vec<CapabilityName>,
    pub tools: Vec<CapabilityName>,
    pub required_privacy: Vec<PrivacyProperty>,
    pub require_local_inference: bool,
    pub preferred_cthulhu_id: Option<CthulhuId>,
    pub preferred_tentacle_id: Option<TentacleId>,
    pub affinity_tentacle_id: Option<TentacleId>,
    pub user_owned_tentacle_id: Option<TentacleId>,
    pub trust_policy: TrustPolicy,
    pub maximum_load_per_mille: u16,
}

impl RoutingRequirements {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.protocol_versions.is_empty() {
            return Err(ValidationError::new(
                "routing.protocolVersions",
                ValidationErrorKind::Empty,
            ));
        }
        bounded_count("routing.protocolVersions", self.protocol_versions.len(), 8)?;
        let mut protocol_versions = BTreeSet::new();
        for version in &self.protocol_versions {
            version.require_supported()?;
            if !protocol_versions.insert(version) {
                return Err(ValidationError::new(
                    "routing.protocolVersions",
                    ValidationErrorKind::InvalidFormat,
                ));
            }
        }
        bounded_count(
            "routing.modelClasses",
            self.model_classes.len(),
            MAX_REQUIREMENTS,
        )?;
        bounded_count("routing.tools", self.tools.len(), MAX_REQUIREMENTS)?;
        bounded_count("routing.requiredPrivacy", self.required_privacy.len(), 16)?;
        if self.maximum_load_per_mille > 1_000 {
            return Err(ValidationError::new(
                "routing.maximumLoadPerMille",
                ValidationErrorKind::OutOfRange,
            ));
        }
        self.trust_policy.validate()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteRequest {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub requester_cthulhu_id: CthulhuId,
    pub requester_tentacle_id: TentacleId,
    pub user_reference: UserReference,
    pub requirements: RoutingRequirements,
    pub issued_at: Timestamp,
    pub expires_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteOffer {
    pub request_id: RequestId,
    pub offering_cthulhu_id: CthulhuId,
    pub offering_tentacle_id: TentacleId,
    pub incarnation: Incarnation,
    pub available_sessions: u32,
    pub current_load_per_mille: u16,
    pub valid_until: Timestamp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RouteRejectReason {
    MissingCapability,
    PrivacyPolicy,
    TrustPolicy,
    Capacity,
    Draining,
    UnsupportedProtocol,
    Expired,
    LocalPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteReject {
    pub request_id: RequestId,
    pub rejecting_cthulhu_id: CthulhuId,
    pub rejecting_tentacle_id: TentacleId,
    pub reason: RouteRejectReason,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteAward {
    pub request_id: RequestId,
    pub session_id: SessionId,
    pub lease_id: LeaseId,
    pub awarded_cthulhu_id: CthulhuId,
    pub awarded_tentacle_id: TentacleId,
    pub incarnation: Incarnation,
    pub generation: u64,
    pub issuer_cthulhu_id: CthulhuId,
    pub issuer_tentacle_id: TentacleId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LeaseStatus {
    Granted,
    Active,
    Released,
    Revoked,
    Expired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Lease {
    pub lease_id: LeaseId,
    pub session_id: SessionId,
    pub user_reference: UserReference,
    pub assigned_cthulhu_id: CthulhuId,
    pub assigned_tentacle_id: TentacleId,
    pub incarnation: Incarnation,
    pub generation: u64,
    pub issued_at: Timestamp,
    pub expires_at: Timestamp,
    pub renewal_deadline: Timestamp,
    pub routing_request_id: RequestId,
    pub issuer_cthulhu_id: CthulhuId,
    pub issuer_tentacle_id: TentacleId,
    pub status: LeaseStatus,
}

impl Lease {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.user_reference.validate()?;
        self.incarnation.validate()?;
        if self.generation == 0
            || self.expires_at <= self.issued_at
            || self
                .expires_at
                .as_unix_seconds()
                .saturating_sub(self.issued_at.as_unix_seconds())
                > MAX_LEASE_LIFETIME_SECONDS
            || self.renewal_deadline <= self.issued_at
            || self.renewal_deadline > self.expires_at
        {
            return Err(ValidationError::new(
                "lease",
                ValidationErrorKind::OutOfRange,
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseGrant {
    pub lease: Lease,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseRenew {
    pub lease_id: LeaseId,
    pub generation: u64,
    pub renewing_cthulhu_id: CthulhuId,
    pub renewing_tentacle_id: TentacleId,
    pub incarnation: Incarnation,
    pub expires_at: Timestamp,
    pub renewal_deadline: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseRelease {
    pub lease_id: LeaseId,
    pub generation: u64,
    pub releasing_cthulhu_id: CthulhuId,
    pub releasing_tentacle_id: TentacleId,
    pub incarnation: Incarnation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseRevoke {
    pub lease_id: LeaseId,
    pub generation: u64,
    pub issuer_cthulhu_id: CthulhuId,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseExpired {
    pub lease_id: LeaseId,
    pub generation: u64,
    pub issuer_cthulhu_id: CthulhuId,
    pub expired_at: Timestamp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GovernanceDocumentKind {
    Constitution,
    Agenda,
    Strategy,
    Action,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "parameters",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BoundedAction {
    CapabilityRefresh { cthulhu_id: Option<CthulhuId> },
    ProtocolSelfTest { tentacle_id: Option<TentacleId> },
    LocalResourceSummary { cthulhu_id: CthulhuId },
    RoutingScenarioEvaluation { scenario_hash: ContentHash },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GovernanceDocument {
    pub kind: GovernanceDocumentKind,
    pub version: u64,
    pub hash: ContentHash,
    pub parent_hash: Option<ContentHash>,
    pub title: String,
    pub summary: String,
    pub action: Option<BoundedAction>,
}

impl GovernanceDocument {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.version == 0 {
            return Err(ValidationError::new(
                "governance.document.version",
                ValidationErrorKind::OutOfRange,
            ));
        }
        bounded_text("governance.document.title", &self.title, 160)?;
        bounded_text("governance.document.summary", &self.summary, 2_048)?;
        if (self.kind == GovernanceDocumentKind::Action) != self.action.is_some() {
            return Err(ValidationError::new(
                "governance.document.action",
                ValidationErrorKind::InvalidFormat,
            ));
        }
        if self.version > 1 && self.parent_hash.is_none() {
            return Err(ValidationError::new(
                "governance.document.parentHash",
                ValidationErrorKind::Empty,
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GovernanceProposal {
    pub proposal_id: ProposalId,
    pub proposer_cthulhu_id: CthulhuId,
    pub document: GovernanceDocument,
    pub deadline: Timestamp,
    pub quorum_basis_points: u16,
    pub approval_basis_points: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArgumentPosition {
    Support,
    Oppose,
    Amend,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GovernanceArgument {
    pub proposal_id: ProposalId,
    pub author_cthulhu_id: CthulhuId,
    pub position: ArgumentPosition,
    pub summary: String,
    pub evidence_hashes: Vec<ContentHash>,
    pub suggested_amendment_hash: Option<ContentHash>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VoteChoice {
    Yes,
    No,
    Abstain,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GovernanceVote {
    pub proposal_id: ProposalId,
    pub voter_cthulhu_id: CthulhuId,
    pub choice: VoteChoice,
    pub cast_at: Timestamp,
    /// Increases when the same Cthulhu replaces its vote before the deadline.
    pub replacement_sequence: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GovernanceRatified {
    pub proposal_id: ProposalId,
    pub document_hash: ContentHash,
    pub issuer_cthulhu_id: CthulhuId,
    pub ratified_at: Timestamp,
    pub yes_votes: u32,
    pub no_votes: u32,
    pub abstentions: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProposalRejectionReason {
    VoteFailed,
    QuorumNotMet,
    Expired,
    CompetingParent,
    InvalidProposal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GovernanceRejected {
    pub proposal_id: ProposalId,
    pub document_hash: ContentHash,
    pub issuer_cthulhu_id: CthulhuId,
    pub rejected_at: Timestamp,
    pub reason: ProposalRejectionReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropagationContentKind {
    CouncilInvitation,
    AgendaSummary,
    ApprovedStrategy,
    CapabilityRequest,
    ResourceNeed,
    ResourceOffer,
    ProtocolUpgrade,
    BoundedCampaign,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CampaignVisibility {
    InvitedOnly,
    Council,
    Public,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropagationPolicy {
    pub policy_version: ProtocolVersion,
    pub maximum_depth: u16,
    pub maximum_fan_out: u16,
    pub visibility: CampaignVisibility,
}

impl PropagationPolicy {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.policy_version.require_supported()?;
        if self.maximum_depth == 0
            || usize::from(self.maximum_depth) > MAX_PROVENANCE_DEPTH
            || self.maximum_fan_out == 0
            || self.maximum_fan_out > 64
        {
            return Err(ValidationError::new(
                "propagation.policy",
                ValidationErrorKind::OutOfRange,
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProvenanceHop {
    pub sender_cthulhu_id: CthulhuId,
    pub sender_tentacle_id: TentacleId,
    pub recipient_cthulhu_id: CthulhuId,
    pub message_id: MessageId,
    pub forwarded_at: Timestamp,
    pub payload_hash: ContentHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropagationItem {
    pub propagation_id: PropagationId,
    pub content_kind: PropagationContentKind,
    pub payload_hash: ContentHash,
    pub origin_cthulhu_id: CthulhuId,
    pub parent_propagation_id: Option<PropagationId>,
    pub depth: u16,
    pub path: Vec<CthulhuId>,
    pub provenance: Vec<ProvenanceHop>,
    /// SHA-256 over the complete ordered provenance material. This detects alteration but does not
    /// authenticate a sender; authenticity remains the envelope signer/transport's responsibility.
    pub chain_hash: ContentHash,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
    pub policy: PropagationPolicy,
}

impl PropagationItem {
    /// Calculate the structural hash for this full provenance chain.
    ///
    /// The hash is deliberately not a signature. A production adapter must still authenticate the
    /// envelope sender and apply its configured [`crate::CouncilVerifier`].
    pub fn calculated_chain_hash(&self) -> Result<ContentHash, ValidationError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct HashMaterial<'a> {
            domain: &'static str,
            propagation_id: &'a PropagationId,
            content_kind: PropagationContentKind,
            payload_hash: &'a ContentHash,
            origin_cthulhu_id: &'a CthulhuId,
            parent_propagation_id: &'a Option<PropagationId>,
            depth: u16,
            path: &'a [CthulhuId],
            provenance: &'a [ProvenanceHop],
            created_at: Timestamp,
            expires_at: Timestamp,
            policy: &'a PropagationPolicy,
        }

        let material = HashMaterial {
            domain: PROVENANCE_HASH_DOMAIN,
            propagation_id: &self.propagation_id,
            content_kind: self.content_kind,
            payload_hash: &self.payload_hash,
            origin_cthulhu_id: &self.origin_cthulhu_id,
            parent_propagation_id: &self.parent_propagation_id,
            depth: self.depth,
            path: &self.path,
            provenance: &self.provenance,
            created_at: self.created_at,
            expires_at: self.expires_at,
            policy: &self.policy,
        };
        let encoded = serde_json::to_vec(&material).map_err(|_| {
            ValidationError::new("propagation.chainHash", ValidationErrorKind::InvalidFormat)
        })?;
        ContentHash::new(format!("sha256:{:x}", Sha256::digest(encoded)))
    }

    /// Refresh the structural chain hash after constructing or extending a locally validated item.
    pub fn recompute_chain_hash(&mut self) -> Result<(), ValidationError> {
        self.chain_hash = self.calculated_chain_hash()?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        self.policy.validate()?;
        if self.expires_at <= self.created_at
            || self
                .expires_at
                .as_unix_seconds()
                .saturating_sub(self.created_at.as_unix_seconds())
                > MAX_PROPAGATION_LIFETIME_SECONDS
            || self.path.is_empty()
            || self.path.first() != Some(&self.origin_cthulhu_id)
            || usize::from(self.depth) + 1 != self.path.len()
            || self.depth > self.policy.maximum_depth
            || self.provenance.len() != usize::from(self.depth)
            || (self.depth == 0) != self.parent_propagation_id.is_none()
        {
            return Err(ValidationError::new(
                "propagation.item",
                ValidationErrorKind::InvalidFormat,
            ));
        }
        bounded_count(
            "propagation.path",
            self.path.len(),
            MAX_PROVENANCE_DEPTH + 1,
        )?;
        bounded_count(
            "propagation.provenance",
            self.provenance.len(),
            MAX_PROVENANCE_DEPTH,
        )?;
        let path = self.path.iter().collect::<BTreeSet<_>>();
        if path.len() != self.path.len() {
            return Err(ValidationError::new(
                "propagation.path",
                ValidationErrorKind::InvalidFormat,
            ));
        }

        let mut message_ids = BTreeSet::new();
        let mut previous_forwarded_at = None;
        for (index, hop) in self.provenance.iter().enumerate() {
            let expected_sender = &self.path[index];
            let expected_recipient = &self.path[index + 1];
            if &hop.sender_cthulhu_id != expected_sender
                || &hop.recipient_cthulhu_id != expected_recipient
                || hop.sender_cthulhu_id == hop.recipient_cthulhu_id
                || hop.payload_hash != self.payload_hash
                || hop.forwarded_at < self.created_at
                || hop.forwarded_at >= self.expires_at
                || previous_forwarded_at.is_some_and(|previous| hop.forwarded_at < previous)
                || !message_ids.insert(&hop.message_id)
            {
                return Err(ValidationError::new(
                    "propagation.provenance",
                    ValidationErrorKind::InvalidFormat,
                ));
            }
            previous_forwarded_at = Some(hop.forwarded_at);
        }

        if self.calculated_chain_hash()? != self.chain_hash {
            return Err(ValidationError::new(
                "propagation.chainHash",
                ValidationErrorKind::InvalidFormat,
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropagationInvite {
    pub invitation_id: crate::InvitationId,
    pub item: PropagationItem,
    pub inviter_cthulhu_id: CthulhuId,
    pub invitee_cthulhu_id: CthulhuId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropagationAccept {
    pub invitation_id: crate::InvitationId,
    pub propagation_id: PropagationId,
    pub invitee_cthulhu_id: CthulhuId,
    pub accepted_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropagationReject {
    pub invitation_id: crate::InvitationId,
    pub propagation_id: PropagationId,
    pub invitee_cthulhu_id: CthulhuId,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropagationAnnounce {
    pub item: PropagationItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropagationForward {
    pub item: PropagationItem,
    pub from_cthulhu_id: CthulhuId,
    pub to_cthulhu_id: CthulhuId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AcknowledgementOutcome {
    Delivered,
    Accepted,
    CapabilityMatched,
    ResourceMatched,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropagationAck {
    pub acknowledgement_id: AcknowledgementId,
    pub propagation_id: PropagationId,
    pub acknowledged_message_id: MessageId,
    pub acknowledging_cthulhu_id: CthulhuId,
    pub outcome: AcknowledgementOutcome,
    pub outcome_reference_hash: ContentHash,
    pub acknowledged_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropagationRevoke {
    pub propagation_id: PropagationId,
    pub revoker_cthulhu_id: CthulhuId,
    pub revoked_at: Timestamp,
    pub reason: String,
}

/// The closed set of supported Council messages. Flattening this enum into an envelope produces
/// the requested sibling `messageType` and `payload` JSON fields.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "messageType", content = "payload")]
pub enum CouncilPayload {
    #[serde(rename = "council.member.announce")]
    CouncilMemberAnnounce(CouncilMemberAnnounce),
    #[serde(rename = "council.member.withdraw")]
    CouncilMemberWithdraw(CouncilMemberWithdraw),
    #[serde(rename = "tentacle.announce")]
    TentacleAnnounce(TentacleAnnounce),
    #[serde(rename = "tentacle.capabilities")]
    TentacleCapabilities(TentacleCapabilities),
    #[serde(rename = "tentacle.heartbeat")]
    TentacleHeartbeat(TentacleHeartbeat),
    #[serde(rename = "tentacle.draining")]
    TentacleDraining(TentacleDraining),
    #[serde(rename = "tentacle.withdraw")]
    TentacleWithdraw(TentacleWithdraw),
    #[serde(rename = "route.request")]
    RouteRequest(RouteRequest),
    #[serde(rename = "route.offer")]
    RouteOffer(RouteOffer),
    #[serde(rename = "route.reject")]
    RouteReject(RouteReject),
    #[serde(rename = "route.award")]
    RouteAward(RouteAward),
    #[serde(rename = "lease.grant")]
    LeaseGrant(LeaseGrant),
    #[serde(rename = "lease.renew")]
    LeaseRenew(LeaseRenew),
    #[serde(rename = "lease.release")]
    LeaseRelease(LeaseRelease),
    #[serde(rename = "lease.revoke")]
    LeaseRevoke(LeaseRevoke),
    #[serde(rename = "lease.expired")]
    LeaseExpired(LeaseExpired),
    #[serde(rename = "governance.proposal")]
    GovernanceProposal(GovernanceProposal),
    #[serde(rename = "governance.argument")]
    GovernanceArgument(GovernanceArgument),
    #[serde(rename = "governance.vote")]
    GovernanceVote(GovernanceVote),
    #[serde(rename = "governance.ratified")]
    GovernanceRatified(GovernanceRatified),
    #[serde(rename = "governance.rejected")]
    GovernanceRejected(GovernanceRejected),
    #[serde(rename = "propagation.invite")]
    PropagationInvite(PropagationInvite),
    #[serde(rename = "propagation.accept")]
    PropagationAccept(PropagationAccept),
    #[serde(rename = "propagation.reject")]
    PropagationReject(PropagationReject),
    #[serde(rename = "propagation.announce")]
    PropagationAnnounce(PropagationAnnounce),
    #[serde(rename = "propagation.forward")]
    PropagationForward(PropagationForward),
    #[serde(rename = "propagation.ack")]
    PropagationAck(PropagationAck),
    #[serde(rename = "propagation.revoke")]
    PropagationRevoke(PropagationRevoke),
}

impl CouncilPayload {
    pub const fn message_type(&self) -> MessageType {
        match self {
            Self::CouncilMemberAnnounce(_) => MessageType::CouncilMemberAnnounce,
            Self::CouncilMemberWithdraw(_) => MessageType::CouncilMemberWithdraw,
            Self::TentacleAnnounce(_) => MessageType::TentacleAnnounce,
            Self::TentacleCapabilities(_) => MessageType::TentacleCapabilities,
            Self::TentacleHeartbeat(_) => MessageType::TentacleHeartbeat,
            Self::TentacleDraining(_) => MessageType::TentacleDraining,
            Self::TentacleWithdraw(_) => MessageType::TentacleWithdraw,
            Self::RouteRequest(_) => MessageType::RouteRequest,
            Self::RouteOffer(_) => MessageType::RouteOffer,
            Self::RouteReject(_) => MessageType::RouteReject,
            Self::RouteAward(_) => MessageType::RouteAward,
            Self::LeaseGrant(_) => MessageType::LeaseGrant,
            Self::LeaseRenew(_) => MessageType::LeaseRenew,
            Self::LeaseRelease(_) => MessageType::LeaseRelease,
            Self::LeaseRevoke(_) => MessageType::LeaseRevoke,
            Self::LeaseExpired(_) => MessageType::LeaseExpired,
            Self::GovernanceProposal(_) => MessageType::GovernanceProposal,
            Self::GovernanceArgument(_) => MessageType::GovernanceArgument,
            Self::GovernanceVote(_) => MessageType::GovernanceVote,
            Self::GovernanceRatified(_) => MessageType::GovernanceRatified,
            Self::GovernanceRejected(_) => MessageType::GovernanceRejected,
            Self::PropagationInvite(_) => MessageType::PropagationInvite,
            Self::PropagationAccept(_) => MessageType::PropagationAccept,
            Self::PropagationReject(_) => MessageType::PropagationReject,
            Self::PropagationAnnounce(_) => MessageType::PropagationAnnounce,
            Self::PropagationForward(_) => MessageType::PropagationForward,
            Self::PropagationAck(_) => MessageType::PropagationAck,
            Self::PropagationRevoke(_) => MessageType::PropagationRevoke,
        }
    }

    pub fn validate_at(&self, now: Timestamp) -> Result<(), ValidationError> {
        match self {
            Self::CouncilMemberAnnounce(value) => value.member.validate(),
            Self::CouncilMemberWithdraw(value) => {
                bounded_optional_text("council.member.withdraw.reason", &value.reason, 256)
            }
            Self::TentacleAnnounce(value) => {
                value.tentacle.validate()?;
                validate_not_far_future(
                    "tentacle.announce.lastHeartbeat",
                    value.tentacle.last_heartbeat,
                    now,
                )
            }
            Self::TentacleCapabilities(value) => {
                value.incarnation.validate()?;
                value.capabilities.validate()?;
                validate_not_far_future("tentacle.capabilities.observedAt", value.observed_at, now)
            }
            Self::TentacleHeartbeat(value) => {
                value.update.validate()?;
                validate_not_far_future(
                    "tentacle.heartbeat.lastHeartbeat",
                    value.update.last_heartbeat,
                    now,
                )
            }
            Self::TentacleDraining(value) => {
                value.incarnation.validate()?;
                validate_not_far_future("tentacle.draining.effectiveAt", value.effective_at, now)
            }
            Self::TentacleWithdraw(value) => {
                value.incarnation.validate()?;
                bounded_optional_text("tentacle.withdraw.reason", &value.reason, 256)
            }
            Self::RouteRequest(value) => {
                value.user_reference.validate()?;
                value.requirements.validate()?;
                validate_window(
                    "route.request",
                    value.issued_at,
                    value.expires_at,
                    now,
                    MAX_ROUTE_WINDOW_SECONDS,
                )
            }
            Self::RouteOffer(value) => {
                value.incarnation.validate()?;
                if value.current_load_per_mille > 1_000
                    || value.valid_until <= now
                    || value.valid_until.as_unix_seconds()
                        > now
                            .as_unix_seconds()
                            .saturating_add(MAX_ROUTE_WINDOW_SECONDS)
                {
                    return Err(ValidationError::new(
                        "route.offer",
                        ValidationErrorKind::Expired,
                    ));
                }
                Ok(())
            }
            Self::RouteReject(_) => Ok(()),
            Self::RouteAward(value) => {
                value.incarnation.validate()?;
                nonzero_generation("route.award.generation", value.generation)
            }
            Self::LeaseGrant(value) => {
                value.lease.validate()?;
                if value.lease.status != LeaseStatus::Granted {
                    return Err(ValidationError::new(
                        "lease.grant.status",
                        ValidationErrorKind::InvalidFormat,
                    ));
                }
                validate_window(
                    "lease.grant",
                    value.lease.issued_at,
                    value.lease.expires_at,
                    now,
                    MAX_LEASE_LIFETIME_SECONDS,
                )
            }
            Self::LeaseRenew(value) => {
                value.incarnation.validate()?;
                nonzero_generation("lease.renew.generation", value.generation)?;
                if value.expires_at <= now
                    || value.renewal_deadline <= now
                    || value.renewal_deadline > value.expires_at
                    || value.expires_at.as_unix_seconds()
                        > now
                            .as_unix_seconds()
                            .saturating_add(MAX_LEASE_LIFETIME_SECONDS)
                {
                    return Err(ValidationError::new(
                        "lease.renew",
                        ValidationErrorKind::Expired,
                    ));
                }
                Ok(())
            }
            Self::LeaseRelease(value) => {
                value.incarnation.validate()?;
                nonzero_generation("lease.release.generation", value.generation)
            }
            Self::LeaseRevoke(value) => {
                nonzero_generation("lease.revoke.generation", value.generation)?;
                bounded_text("lease.revoke.reason", &value.reason, 256)
            }
            Self::LeaseExpired(value) => {
                nonzero_generation("lease.expired.generation", value.generation)?;
                if value.expired_at > now {
                    return Err(ValidationError::new(
                        "lease.expired.expiredAt",
                        ValidationErrorKind::OutOfRange,
                    ));
                }
                Ok(())
            }
            Self::GovernanceProposal(value) => {
                value.document.validate()?;
                if value.deadline <= now
                    || value.deadline.as_unix_seconds()
                        > now
                            .as_unix_seconds()
                            .saturating_add(MAX_GOVERNANCE_LIFETIME_SECONDS)
                    || !(1..=10_000).contains(&value.quorum_basis_points)
                    || !(1..=10_000).contains(&value.approval_basis_points)
                {
                    return Err(ValidationError::new(
                        "governance.proposal",
                        ValidationErrorKind::OutOfRange,
                    ));
                }
                Ok(())
            }
            Self::GovernanceArgument(value) => {
                bounded_text("governance.argument.summary", &value.summary, 2_048)?;
                bounded_count(
                    "governance.argument.evidenceHashes",
                    value.evidence_hashes.len(),
                    MAX_ARGUMENT_HASHES,
                )
            }
            Self::GovernanceVote(value) => {
                if value.cast_at > now {
                    return Err(ValidationError::new(
                        "governance.vote.castAt",
                        ValidationErrorKind::OutOfRange,
                    ));
                }
                Ok(())
            }
            Self::GovernanceRatified(value) => validate_vote_tally(
                value.ratified_at,
                value.yes_votes,
                value.no_votes,
                value.abstentions,
                now,
            ),
            Self::GovernanceRejected(value) => {
                if value.rejected_at > now {
                    return Err(ValidationError::new(
                        "governance.rejected.rejectedAt",
                        ValidationErrorKind::OutOfRange,
                    ));
                }
                Ok(())
            }
            Self::PropagationInvite(value) => {
                value.item.validate()?;
                validate_window(
                    "propagation.invite.item",
                    value.item.created_at,
                    value.item.expires_at,
                    now,
                    MAX_PROPAGATION_LIFETIME_SECONDS,
                )?;
                if value.inviter_cthulhu_id == value.invitee_cthulhu_id
                    || value.item.expires_at <= now
                {
                    return Err(ValidationError::new(
                        "propagation.invite",
                        ValidationErrorKind::InvalidFormat,
                    ));
                }
                Ok(())
            }
            Self::PropagationAccept(value) => {
                if value.accepted_at > now {
                    return Err(ValidationError::new(
                        "propagation.accept.acceptedAt",
                        ValidationErrorKind::OutOfRange,
                    ));
                }
                Ok(())
            }
            Self::PropagationReject(value) => {
                bounded_text("propagation.reject.reason", &value.reason, 256)
            }
            Self::PropagationAnnounce(value) => {
                value.item.validate()?;
                validate_window(
                    "propagation.announce.item",
                    value.item.created_at,
                    value.item.expires_at,
                    now,
                    MAX_PROPAGATION_LIFETIME_SECONDS,
                )?;
                if value.item.expires_at <= now {
                    return Err(ValidationError::new(
                        "propagation.announce",
                        ValidationErrorKind::Expired,
                    ));
                }
                Ok(())
            }
            Self::PropagationForward(value) => {
                value.item.validate()?;
                validate_window(
                    "propagation.forward.item",
                    value.item.created_at,
                    value.item.expires_at,
                    now,
                    MAX_PROPAGATION_LIFETIME_SECONDS,
                )?;
                if value.from_cthulhu_id == value.to_cthulhu_id
                    || value.item.path.last() != Some(&value.from_cthulhu_id)
                    || value.item.path.contains(&value.to_cthulhu_id)
                    || value.item.expires_at <= now
                {
                    return Err(ValidationError::new(
                        "propagation.forward",
                        ValidationErrorKind::InvalidFormat,
                    ));
                }
                Ok(())
            }
            Self::PropagationAck(value) => {
                if value.acknowledged_at > now {
                    return Err(ValidationError::new(
                        "propagation.ack.acknowledgedAt",
                        ValidationErrorKind::OutOfRange,
                    ));
                }
                Ok(())
            }
            Self::PropagationRevoke(value) => {
                bounded_text("propagation.revoke.reason", &value.reason, 256)?;
                if value.revoked_at > now {
                    return Err(ValidationError::new(
                        "propagation.revoke.revokedAt",
                        ValidationErrorKind::OutOfRange,
                    ));
                }
                Ok(())
            }
        }
    }

    pub fn validate_sender(
        &self,
        sender_cthulhu_id: &CthulhuId,
        sender_tentacle_id: &TentacleId,
    ) -> Result<(), ValidationError> {
        let matches = match self {
            Self::CouncilMemberAnnounce(value) => {
                &value.member.id == sender_cthulhu_id
                    && value.member.tentacles.contains(sender_tentacle_id)
            }
            Self::CouncilMemberWithdraw(value) => &value.cthulhu_id == sender_cthulhu_id,
            Self::TentacleAnnounce(value) => {
                &value.tentacle.owner == sender_cthulhu_id
                    && &value.tentacle.id == sender_tentacle_id
            }
            Self::TentacleCapabilities(value) => {
                &value.owner == sender_cthulhu_id && &value.tentacle_id == sender_tentacle_id
            }
            Self::TentacleHeartbeat(value) => {
                &value.update.owner == sender_cthulhu_id
                    && &value.update.tentacle_id == sender_tentacle_id
            }
            Self::TentacleDraining(value) => {
                &value.owner == sender_cthulhu_id && &value.tentacle_id == sender_tentacle_id
            }
            Self::TentacleWithdraw(value) => {
                &value.owner == sender_cthulhu_id && &value.tentacle_id == sender_tentacle_id
            }
            Self::RouteRequest(value) => {
                &value.requester_cthulhu_id == sender_cthulhu_id
                    && &value.requester_tentacle_id == sender_tentacle_id
            }
            Self::RouteOffer(value) => {
                &value.offering_cthulhu_id == sender_cthulhu_id
                    && &value.offering_tentacle_id == sender_tentacle_id
            }
            Self::RouteReject(value) => {
                &value.rejecting_cthulhu_id == sender_cthulhu_id
                    && &value.rejecting_tentacle_id == sender_tentacle_id
            }
            Self::RouteAward(value) => {
                &value.issuer_cthulhu_id == sender_cthulhu_id
                    && &value.issuer_tentacle_id == sender_tentacle_id
            }
            Self::LeaseGrant(value) => {
                &value.lease.issuer_cthulhu_id == sender_cthulhu_id
                    && &value.lease.issuer_tentacle_id == sender_tentacle_id
            }
            Self::LeaseRenew(value) => {
                &value.renewing_cthulhu_id == sender_cthulhu_id
                    && &value.renewing_tentacle_id == sender_tentacle_id
            }
            Self::LeaseRelease(value) => {
                &value.releasing_cthulhu_id == sender_cthulhu_id
                    && &value.releasing_tentacle_id == sender_tentacle_id
            }
            Self::LeaseRevoke(value) => &value.issuer_cthulhu_id == sender_cthulhu_id,
            Self::LeaseExpired(value) => &value.issuer_cthulhu_id == sender_cthulhu_id,
            Self::GovernanceProposal(value) => &value.proposer_cthulhu_id == sender_cthulhu_id,
            Self::GovernanceArgument(value) => &value.author_cthulhu_id == sender_cthulhu_id,
            Self::GovernanceVote(value) => &value.voter_cthulhu_id == sender_cthulhu_id,
            Self::GovernanceRatified(value) => &value.issuer_cthulhu_id == sender_cthulhu_id,
            Self::GovernanceRejected(value) => &value.issuer_cthulhu_id == sender_cthulhu_id,
            Self::PropagationInvite(value) => &value.inviter_cthulhu_id == sender_cthulhu_id,
            Self::PropagationAccept(value) => &value.invitee_cthulhu_id == sender_cthulhu_id,
            Self::PropagationReject(value) => &value.invitee_cthulhu_id == sender_cthulhu_id,
            Self::PropagationAnnounce(value) => value.item.path.last() == Some(sender_cthulhu_id),
            Self::PropagationForward(value) => &value.from_cthulhu_id == sender_cthulhu_id,
            Self::PropagationAck(value) => &value.acknowledging_cthulhu_id == sender_cthulhu_id,
            Self::PropagationRevoke(value) => &value.revoker_cthulhu_id == sender_cthulhu_id,
        };
        if !matches {
            return Err(ValidationError::new(
                "sender",
                ValidationErrorKind::SenderMismatch,
            ));
        }
        Ok(())
    }
}

fn nonzero_generation(field: &str, generation: u64) -> Result<(), ValidationError> {
    if generation == 0 {
        return Err(ValidationError::new(field, ValidationErrorKind::OutOfRange));
    }
    Ok(())
}

fn validate_window(
    field: &str,
    starts: Timestamp,
    ends: Timestamp,
    now: Timestamp,
    maximum_lifetime_seconds: i64,
) -> Result<(), ValidationError> {
    if ends <= starts {
        return Err(ValidationError::new(field, ValidationErrorKind::OutOfRange));
    }
    if ends <= now {
        return Err(ValidationError::new(field, ValidationErrorKind::Expired));
    }
    validate_not_far_future(field, starts, now)?;
    if ends
        .as_unix_seconds()
        .saturating_sub(starts.as_unix_seconds())
        > maximum_lifetime_seconds
    {
        return Err(ValidationError::new(field, ValidationErrorKind::OutOfRange));
    }
    Ok(())
}

fn validate_not_far_future(
    field: &str,
    value: Timestamp,
    now: Timestamp,
) -> Result<(), ValidationError> {
    if value.as_unix_seconds()
        > now
            .as_unix_seconds()
            .saturating_add(crate::MAX_FUTURE_CLOCK_SKEW_SECONDS)
    {
        return Err(ValidationError::new(field, ValidationErrorKind::OutOfRange));
    }
    Ok(())
}

fn validate_vote_tally(
    decided_at: Timestamp,
    yes: u32,
    no: u32,
    abstain: u32,
    now: Timestamp,
) -> Result<(), ValidationError> {
    if decided_at > now || yes.saturating_add(no).saturating_add(abstain) == 0 {
        return Err(ValidationError::new(
            "governance.voteTally",
            ValidationErrorKind::OutOfRange,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_requested_message_types_have_exact_stable_names() {
        let names = [
            MessageType::CouncilMemberAnnounce,
            MessageType::CouncilMemberWithdraw,
            MessageType::TentacleAnnounce,
            MessageType::TentacleCapabilities,
            MessageType::TentacleHeartbeat,
            MessageType::TentacleDraining,
            MessageType::TentacleWithdraw,
            MessageType::RouteRequest,
            MessageType::RouteOffer,
            MessageType::RouteReject,
            MessageType::RouteAward,
            MessageType::LeaseGrant,
            MessageType::LeaseRenew,
            MessageType::LeaseRelease,
            MessageType::LeaseRevoke,
            MessageType::LeaseExpired,
            MessageType::GovernanceProposal,
            MessageType::GovernanceArgument,
            MessageType::GovernanceVote,
            MessageType::GovernanceRatified,
            MessageType::GovernanceRejected,
            MessageType::PropagationInvite,
            MessageType::PropagationAccept,
            MessageType::PropagationReject,
            MessageType::PropagationAnnounce,
            MessageType::PropagationForward,
            MessageType::PropagationAck,
            MessageType::PropagationRevoke,
        ];
        assert_eq!(names.len(), 28);
        assert_eq!(names[4].as_str(), "tentacle.heartbeat");
        assert_eq!(names[27].as_str(), "propagation.revoke");
    }

    #[test]
    fn governance_action_cannot_encode_a_shell_command() {
        let action = BoundedAction::ProtocolSelfTest { tentacle_id: None };
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(
            json,
            r#"{"type":"protocolSelfTest","parameters":{"tentacleId":null}}"#
        );
        assert!(
            serde_json::from_str::<BoundedAction>(
                r#"{"type":"shellCommand","parameters":{"command":"rm -rf /"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn propagation_rejects_loops_and_self_referrals() {
        let origin = CthulhuId::new("cthulhu_origin").unwrap();
        let invite = PropagationInvite {
            invitation_id: crate::InvitationId::new("invite_one").unwrap(),
            item: root_propagation_item(origin.clone()),
            inviter_cthulhu_id: origin.clone(),
            invitee_cthulhu_id: origin,
        };
        assert!(
            CouncilPayload::PropagationInvite(invite)
                .validate_at(at(2))
                .is_err()
        );
    }

    #[test]
    fn full_provenance_chain_validates_and_round_trips() {
        let item = two_hop_propagation_item();
        item.validate().unwrap();

        let encoded = serde_json::to_vec(&item).unwrap();
        let decoded: PropagationItem = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, item);
        decoded.validate().unwrap();
        assert_eq!(decoded.calculated_chain_hash().unwrap(), decoded.chain_hash);
    }

    #[test]
    fn provenance_hash_binds_campaign_payload_path_hop_identities_messages_and_times() {
        let item = two_hop_propagation_item();
        let expected = item.chain_hash.clone();

        let mut mutations = Vec::new();

        let mut campaign = item.clone();
        campaign.propagation_id = PropagationId::new("propagation_other").unwrap();
        mutations.push(campaign);

        let mut payload = item.clone();
        payload.payload_hash = hash_with('1');
        mutations.push(payload);

        let mut path = item.clone();
        path.path[1] = CthulhuId::new("cthulhu_intruder").unwrap();
        mutations.push(path);

        let mut sender = item.clone();
        sender.provenance[0].sender_cthulhu_id = CthulhuId::new("cthulhu_intruder").unwrap();
        mutations.push(sender);

        let mut tentacle = item.clone();
        tentacle.provenance[0].sender_tentacle_id = TentacleId::new("tentacle_intruder").unwrap();
        mutations.push(tentacle);

        let mut recipient = item.clone();
        recipient.provenance[0].recipient_cthulhu_id = CthulhuId::new("cthulhu_intruder").unwrap();
        mutations.push(recipient);

        let mut message = item.clone();
        message.provenance[0].message_id = MessageId::new("msg_replaced").unwrap();
        mutations.push(message);

        let mut time = item.clone();
        time.provenance[0].forwarded_at = at(9);
        mutations.push(time);

        let mut policy = item.clone();
        policy.policy.maximum_fan_out = 3;
        mutations.push(policy);

        for mutation in mutations {
            assert_ne!(mutation.calculated_chain_hash().unwrap(), expected);
            assert!(mutation.validate().is_err());
        }
    }

    #[test]
    fn provenance_rejects_splicing_duplicates_and_out_of_order_hops_after_rehashing() {
        let item = two_hop_propagation_item();

        let mut mismatched_path = item.clone();
        mismatched_path.path[1] = CthulhuId::new("cthulhu_intruder").unwrap();
        mismatched_path.recompute_chain_hash().unwrap();
        assert!(mismatched_path.validate().is_err());

        let mut duplicate_message = item.clone();
        duplicate_message.provenance[1].message_id =
            duplicate_message.provenance[0].message_id.clone();
        duplicate_message.recompute_chain_hash().unwrap();
        assert!(duplicate_message.validate().is_err());

        let mut out_of_order = item.clone();
        out_of_order.provenance[0].forwarded_at = at(4);
        out_of_order.provenance[1].forwarded_at = at(3);
        out_of_order.recompute_chain_hash().unwrap();
        assert!(out_of_order.validate().is_err());

        let mut truncated = item;
        truncated.provenance.pop();
        truncated.recompute_chain_hash().unwrap();
        assert!(truncated.validate().is_err());
    }

    #[test]
    fn council_envelope_rejects_a_forged_wire_provenance_chain() {
        let sender_cthulhu_id = CthulhuId::new("cthulhu_leaf").unwrap();
        let sender_tentacle_id = TentacleId::new("tentacle_leaf").unwrap();
        let forward = |item| {
            CouncilPayload::PropagationForward(PropagationForward {
                item,
                from_cthulhu_id: sender_cthulhu_id.clone(),
                to_cthulhu_id: CthulhuId::new("cthulhu_next").unwrap(),
            })
        };
        let envelope = |payload| {
            crate::CouncilEnvelope::new(
                MessageId::new("msg_wire_forward").unwrap(),
                crate::CouncilId::new("council_wire").unwrap(),
                sender_cthulhu_id.clone(),
                sender_tentacle_id.clone(),
                at(10),
                at(20),
                1,
                payload,
            )
        };

        envelope(forward(two_hop_propagation_item()))
            .validate_at(at(10))
            .unwrap();

        let mut forged = two_hop_propagation_item();
        forged.provenance[0].message_id = MessageId::new("msg_forged_hop").unwrap();
        assert!(envelope(forward(forged)).validate_at(at(10)).is_err());
    }

    #[test]
    fn domain_windows_reject_future_poisoning_and_unbounded_lifetimes() {
        assert!(
            validate_window(
                "fixture",
                at(1_000),
                at(1_100),
                at(100),
                MAX_ROUTE_WINDOW_SECONDS,
            )
            .is_err()
        );
        assert!(
            validate_window(
                "fixture",
                at(100),
                at(100 + MAX_ROUTE_WINDOW_SECONDS + 1),
                at(100),
                MAX_ROUTE_WINDOW_SECONDS,
            )
            .is_err()
        );
        assert!(
            validate_window(
                "fixture",
                at(100),
                at(200),
                at(100),
                MAX_ROUTE_WINDOW_SECONDS,
            )
            .is_ok()
        );
    }

    #[test]
    fn routing_requirements_reject_duplicate_and_unsupported_versions() {
        let requirements = |protocol_versions| RoutingRequirements {
            protocol_versions,
            model_classes: vec![],
            tools: vec![],
            required_privacy: vec![],
            require_local_inference: false,
            preferred_cthulhu_id: None,
            preferred_tentacle_id: None,
            affinity_tentacle_id: None,
            user_owned_tentacle_id: None,
            trust_policy: TrustPolicy {
                allowlisted_only: false,
                registry_association_required: false,
                accepted_mechanisms: vec![],
                accepted_registries: vec![],
            },
            maximum_load_per_mille: 1_000,
        };

        assert!(
            requirements(vec![ProtocolVersion::V1_0, ProtocolVersion::V1_0])
                .validate()
                .is_err()
        );
        assert!(
            requirements(vec![ProtocolVersion::new(2, 0)])
                .validate()
                .is_err()
        );
        requirements(vec![ProtocolVersion::V1_0])
            .validate()
            .unwrap();
    }

    #[test]
    fn lease_renewal_deadline_must_still_be_open() {
        let renewal = |renewal_deadline| {
            CouncilPayload::LeaseRenew(LeaseRenew {
                lease_id: LeaseId::new("lease_renewal").unwrap(),
                generation: 1,
                renewing_cthulhu_id: CthulhuId::new("cthulhu_owner").unwrap(),
                renewing_tentacle_id: TentacleId::new("tentacle_owner").unwrap(),
                incarnation: Incarnation {
                    id: crate::IncarnationId::new("incarnation_owner").unwrap(),
                    generation: 1,
                },
                expires_at: at(200),
                renewal_deadline,
            })
        };

        assert!(renewal(at(100)).validate_at(at(100)).is_err());
        assert!(renewal(at(99)).validate_at(at(100)).is_err());
        renewal(at(101)).validate_at(at(100)).unwrap();
    }

    fn at(value: i64) -> Timestamp {
        Timestamp::from_unix_seconds(value).unwrap()
    }

    fn hash() -> ContentHash {
        ContentHash::new(format!("sha256:{}", "0".repeat(64))).unwrap()
    }

    fn hash_with(character: char) -> ContentHash {
        ContentHash::new(format!("sha256:{}", character.to_string().repeat(64))).unwrap()
    }

    fn propagation_policy() -> PropagationPolicy {
        PropagationPolicy {
            policy_version: ProtocolVersion::V1_0,
            maximum_depth: 3,
            maximum_fan_out: 2,
            visibility: CampaignVisibility::InvitedOnly,
        }
    }

    fn root_propagation_item(origin: CthulhuId) -> PropagationItem {
        let mut item = PropagationItem {
            propagation_id: PropagationId::new("propagation_one").unwrap(),
            content_kind: PropagationContentKind::CouncilInvitation,
            payload_hash: hash(),
            origin_cthulhu_id: origin.clone(),
            parent_propagation_id: None,
            depth: 0,
            path: vec![origin],
            provenance: vec![],
            chain_hash: hash(),
            created_at: at(1),
            expires_at: at(100),
            policy: propagation_policy(),
        };
        item.recompute_chain_hash().unwrap();
        item
    }

    fn two_hop_propagation_item() -> PropagationItem {
        let origin = CthulhuId::new("cthulhu_origin").unwrap();
        let branch = CthulhuId::new("cthulhu_branch").unwrap();
        let leaf = CthulhuId::new("cthulhu_leaf").unwrap();
        let payload_hash = hash();
        let mut item = PropagationItem {
            propagation_id: PropagationId::new("propagation_one").unwrap(),
            content_kind: PropagationContentKind::AgendaSummary,
            payload_hash: payload_hash.clone(),
            origin_cthulhu_id: origin.clone(),
            parent_propagation_id: Some(PropagationId::new("propagation_parent_referral").unwrap()),
            depth: 2,
            path: vec![origin.clone(), branch.clone(), leaf.clone()],
            provenance: vec![
                ProvenanceHop {
                    sender_cthulhu_id: origin,
                    sender_tentacle_id: TentacleId::new("tentacle_origin").unwrap(),
                    recipient_cthulhu_id: branch.clone(),
                    message_id: MessageId::new("msg_hop_one").unwrap(),
                    forwarded_at: at(2),
                    payload_hash: payload_hash.clone(),
                },
                ProvenanceHop {
                    sender_cthulhu_id: branch,
                    sender_tentacle_id: TentacleId::new("tentacle_branch").unwrap(),
                    recipient_cthulhu_id: leaf,
                    message_id: MessageId::new("msg_hop_two").unwrap(),
                    forwarded_at: at(3),
                    payload_hash,
                },
            ],
            chain_hash: hash(),
            created_at: at(1),
            expires_at: at(100),
            policy: propagation_policy(),
        };
        item.recompute_chain_hash().unwrap();
        item
    }
}
