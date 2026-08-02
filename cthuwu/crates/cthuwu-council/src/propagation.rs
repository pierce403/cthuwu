//! Bounded, provenance-preserving Council propagation.
//!
//! This module deliberately propagates only typed Council material. It has no
//! variant for direct-message content, contact memory, credentials, prompts,
//! arbitrary URLs, or executable commands.

use std::collections::{HashMap, HashSet};
use std::fmt;

pub use cthuwu_protocol::{
    AcknowledgementId, MessageId as PropagationItemId, PropagationId as CampaignId,
};
use cthuwu_protocol::{CouncilId, CthulhuId};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MAX_CAMPAIGNS: usize = 256;
pub const MAX_ITEMS: usize = 16_384;
pub const MAX_ACKNOWLEDGEMENTS: usize = 16_384;
pub const MAX_CANDIDATES: usize = 4_096;
pub const MAX_LOCAL_RULES: usize = 16_384;
pub const MAX_LOCAL_POLICY_EVENTS: usize = 16_384;
pub const MAX_DEPTH: u8 = 16;
pub const MAX_FAN_OUT: u16 = 64;
pub const MAX_RATE_LIMIT: u16 = 128;
pub const MAX_CAMPAIGN_LIFETIME_SECONDS: i64 = 30 * 24 * 60 * 60;
pub const MAX_RATE_WINDOW_SECONDS: i64 = 24 * 60 * 60;
pub const MAX_TEXT_BYTES: usize = 4 * 1024;
pub const MAX_SHORT_TEXT_BYTES: usize = 256;
pub const MAX_LIST_ITEMS: usize = 64;
pub const MAX_PROVENANCE_HOPS: usize = MAX_DEPTH as usize;
pub const MAX_CREDIT_PER_OUTCOME: u16 = 5;
pub const MAX_CREDIT_PER_CTHULHU_PER_CAMPAIGN: u16 = 20;
pub const MAX_TOTAL_CAMPAIGN_CREDIT: u16 = 512;

macro_rules! bounded_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: &str) -> Result<Self, PropagationError> {
                let Some(suffix) = value.strip_prefix($prefix) else {
                    return Err(PropagationError::MalformedIdentifier(stringify!($name)));
                };
                if suffix.is_empty()
                    || suffix.len() > 64
                    || suffix.starts_with(['_', '-'])
                    || suffix.ends_with(['_', '-'])
                    || suffix.contains("__")
                    || suffix.contains("--")
                    || !suffix.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'_' | b'-')
                    })
                {
                    return Err(PropagationError::MalformedIdentifier(stringify!($name)));
                }
                Ok(Self(value.to_owned()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Self::parse(&value).map_err(D::Error::custom)
            }
        }
    };
}

bounded_id!(OutcomeId, "outcome_");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PropagationPayload {
    CouncilInvitation {
        council_id: CouncilId,
        summary: String,
    },
    AgendaSummary {
        agenda_hash: String,
        summary: String,
    },
    ApprovedStrategy {
        strategy_hash: String,
        summary: String,
    },
    CapabilityRequest {
        capability_tags: Vec<String>,
        summary: String,
    },
    ResourceNeed {
        categories: Vec<String>,
        summary: String,
    },
    ResourceOffer {
        categories: Vec<String>,
        summary: String,
    },
    ProtocolUpgrade {
        version: String,
        artifact_hash: String,
        summary: String,
    },
    BoundedCampaign {
        name: String,
        summary: String,
    },
}

impl PropagationPayload {
    pub fn validate(&self) -> Result<(), PropagationError> {
        match self {
            Self::CouncilInvitation { summary, .. }
            | Self::AgendaSummary { summary, .. }
            | Self::ApprovedStrategy { summary, .. }
            | Self::CapabilityRequest { summary, .. }
            | Self::ResourceNeed { summary, .. }
            | Self::ResourceOffer { summary, .. }
            | Self::ProtocolUpgrade { summary, .. } => {
                validate_text(summary, MAX_TEXT_BYTES, "payload summary", false)?;
            }
            Self::BoundedCampaign { name, summary } => {
                validate_text(name, MAX_SHORT_TEXT_BYTES, "campaign name", true)?;
                validate_text(summary, MAX_TEXT_BYTES, "payload summary", false)?;
            }
        }
        match self {
            Self::AgendaSummary { agenda_hash, .. } => validate_hash(agenda_hash)?,
            Self::ApprovedStrategy { strategy_hash, .. } => validate_hash(strategy_hash)?,
            Self::CapabilityRequest {
                capability_tags, ..
            } => validate_tags(capability_tags, "capability tags")?,
            Self::ResourceNeed { categories, .. } | Self::ResourceOffer { categories, .. } => {
                validate_tags(categories, "resource categories")?
            }
            Self::ProtocolUpgrade {
                version,
                artifact_hash,
                ..
            } => {
                validate_token(version, "protocol version")?;
                validate_hash(artifact_hash)?;
            }
            Self::CouncilInvitation { .. } | Self::BoundedCampaign { .. } => {}
        }
        let bytes = serde_json::to_vec(self).map_err(PropagationError::Serialization)?;
        if bytes.len() > 16 * 1024 {
            return Err(PropagationError::LimitExceeded("serialized payload"));
        }
        Ok(())
    }

    pub fn content_hash(&self) -> Result<String, PropagationError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(PropagationError::Serialization)?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignVisibility {
    CouncilMembers,
    InvitedBranches,
    PublicMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PropagationStrategy {
    BreadthFirst,
    DepthLimited {
        depth: u8,
    },
    TrustedBranchOnly,
    CapabilityTargeted {
        capability: String,
    },
    GeographicOrLatencyAware {
        preferred_region: Option<String>,
        max_latency_ms: u32,
    },
    ReputationThresholded {
        minimum_bps: u16,
        accepted_sources: Vec<String>,
    },
}

impl PropagationStrategy {
    fn validate(&self) -> Result<(), PropagationError> {
        match self {
            Self::BreadthFirst | Self::TrustedBranchOnly => Ok(()),
            Self::DepthLimited { depth } if *depth > 0 && *depth <= MAX_DEPTH => Ok(()),
            Self::DepthLimited { .. } => Err(PropagationError::InvalidPolicy("strategy depth")),
            Self::CapabilityTargeted { capability } => {
                validate_token(capability, "target capability")
            }
            Self::GeographicOrLatencyAware {
                preferred_region,
                max_latency_ms,
            } => {
                if *max_latency_ms == 0 || *max_latency_ms > 60_000 {
                    return Err(PropagationError::InvalidPolicy("latency bound"));
                }
                if let Some(region) = preferred_region {
                    validate_token(region, "preferred region")?;
                }
                Ok(())
            }
            Self::ReputationThresholded {
                minimum_bps,
                accepted_sources,
            } => {
                if *minimum_bps > 10_000 {
                    return Err(PropagationError::InvalidPolicy("reputation threshold"));
                }
                validate_tags(accepted_sources, "reputation sources")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropagationPolicy {
    pub version: u32,
    pub max_depth: u8,
    pub max_fan_out: u16,
    pub per_sender_rate_limit: u16,
    pub rate_window_seconds: i64,
    pub visibility: CampaignVisibility,
}

impl PropagationPolicy {
    pub fn validate(&self) -> Result<(), PropagationError> {
        if self.version == 0
            || self.max_depth == 0
            || self.max_depth > MAX_DEPTH
            || self.max_fan_out == 0
            || self.max_fan_out > MAX_FAN_OUT
            || self.per_sender_rate_limit == 0
            || self.per_sender_rate_limit > MAX_RATE_LIMIT
            || self.rate_window_seconds <= 0
            || self.rate_window_seconds > MAX_RATE_WINDOW_SECONDS
        {
            return Err(PropagationError::InvalidPolicy("bounds"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Campaign {
    pub id: CampaignId,
    pub council_id: CouncilId,
    pub root: CthulhuId,
    pub payload: PropagationPayload,
    pub payload_hash: String,
    pub strategy: PropagationStrategy,
    pub policy: PropagationPolicy,
    pub created_at: i64,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
    pub revocation_reason: Option<String>,
}

impl Campaign {
    fn validate(&self) -> Result<(), PropagationError> {
        self.payload.validate()?;
        self.strategy.validate()?;
        self.policy.validate()?;
        validate_hash(&self.payload_hash)?;
        if self.payload.content_hash()? != self.payload_hash {
            return Err(PropagationError::PayloadHashMismatch);
        }
        if self.created_at < 0
            || self.expires_at <= self.created_at
            || self.expires_at - self.created_at > MAX_CAMPAIGN_LIFETIME_SECONDS
        {
            return Err(PropagationError::InvalidExpiry);
        }
        if let Some(revoked_at) = self.revoked_at {
            if revoked_at < self.created_at {
                return Err(PropagationError::InvalidTimestamp);
            }
            let reason = self
                .revocation_reason
                .as_deref()
                .ok_or(PropagationError::InvalidPolicy("missing revocation reason"))?;
            validate_text(reason, MAX_SHORT_TEXT_BYTES, "revocation reason", false)?;
        } else if self.revocation_reason.is_some() {
            return Err(PropagationError::InvalidPolicy(
                "revocation reason without revocation",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReputationSignal {
    pub source: String,
    pub value_bps: u16,
    pub observed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateProfile {
    pub cthulhu_id: CthulhuId,
    pub trusted: bool,
    pub council_memberships: Vec<CouncilId>,
    pub capability_tags: Vec<String>,
    pub region: Option<String>,
    pub latency_ms: Option<u32>,
    /// Provenance-bearing signals; deliberately not collapsed into a global score.
    pub reputation_signals: Vec<ReputationSignal>,
}

impl CandidateProfile {
    fn validate(&self) -> Result<(), PropagationError> {
        if self.council_memberships.len() > MAX_LIST_ITEMS {
            return Err(PropagationError::LimitExceeded("Council memberships"));
        }
        if self
            .council_memberships
            .iter()
            .collect::<HashSet<_>>()
            .len()
            != self.council_memberships.len()
        {
            return Err(PropagationError::InvalidPolicy(
                "duplicate Council membership",
            ));
        }
        if !self.capability_tags.is_empty() {
            validate_tags(&self.capability_tags, "candidate capabilities")?;
        }
        if let Some(region) = &self.region {
            validate_token(region, "candidate region")?;
        }
        if self.latency_ms.is_some_and(|latency| latency > 60_000) {
            return Err(PropagationError::InvalidPolicy("candidate latency"));
        }
        if self.reputation_signals.len() > MAX_LIST_ITEMS {
            return Err(PropagationError::LimitExceeded("reputation signals"));
        }
        for signal in &self.reputation_signals {
            validate_token(&signal.source, "reputation source")?;
            if signal.value_bps > 10_000 || signal.observed_at < 0 {
                return Err(PropagationError::InvalidPolicy("reputation signal"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceHop {
    pub item_id: PropagationItemId,
    pub sender: CthulhuId,
    pub recipient: CthulhuId,
    pub sent_at: i64,
    pub local_policy_generation: u64,
    pub sender_profile_hash: String,
    pub recipient_profile_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub root: CthulhuId,
    pub hops: Vec<ProvenanceHop>,
    pub chain_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Pending,
    Accepted,
    Rejected,
    Acknowledged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropagationItem {
    pub id: PropagationItemId,
    pub campaign_id: CampaignId,
    pub parent_item_id: Option<PropagationItemId>,
    pub sender: CthulhuId,
    pub recipient: CthulhuId,
    pub depth: u8,
    pub policy_version: u32,
    pub payload_hash: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub provenance: Provenance,
    pub status: ItemStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acknowledgement {
    pub id: AcknowledgementId,
    pub item_id: PropagationItemId,
    pub actor: CthulhuId,
    pub outcome: ContributionOutcome,
    pub evidence_hash: String,
    pub acknowledged_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RateRecord {
    item_id: PropagationItemId,
    campaign_id: CampaignId,
    sender: CthulhuId,
    sent_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LocalPolicyEventKind {
    OptOut {
        cthulhu_id: CthulhuId,
        enabled: bool,
    },
    Block {
        owner: CthulhuId,
        blocked: CthulhuId,
        enabled: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalPolicyEvent {
    generation: u64,
    event: LocalPolicyEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropagationExplanation {
    pub allowed: bool,
    pub reasons: Vec<String>,
    pub depth: u8,
    pub remaining_fan_out: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryResult {
    Created {
        item: PropagationItem,
        explanation: PropagationExplanation,
    },
    ReplaySuppressed {
        item: PropagationItem,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionOutcome {
    SuccessfulIntroduction,
    UsefulCapabilityReferral,
    AcknowledgedDownstreamDelivery,
    CompletedResourceMatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeClaim {
    pub id: OutcomeId,
    pub campaign_id: CampaignId,
    pub item_id: PropagationItemId,
    pub acknowledgement_id: AcknowledgementId,
    pub contributor: CthulhuId,
    pub beneficiary: CthulhuId,
    pub outcome: ContributionOutcome,
    pub evidence_hash: String,
    pub occurred_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionCredit {
    pub campaign_id: CampaignId,
    pub cthulhu_id: CthulhuId,
    pub points: u16,
    pub credited_outcomes: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditContext<'a> {
    pub campaign: &'a Campaign,
    pub item: &'a PropagationItem,
    pub claim: &'a OutcomeClaim,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedCredit {
    pub points: u16,
    pub reason: String,
}

pub trait IncentiveModel {
    fn propose_credit(&self, context: &CreditContext<'_>) -> ProposedCredit;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SafeOutcomeCredit;

impl IncentiveModel for SafeOutcomeCredit {
    fn propose_credit(&self, context: &CreditContext<'_>) -> ProposedCredit {
        let (points, reason) = match context.claim.outcome {
            ContributionOutcome::SuccessfulIntroduction => {
                (1, "accepted and acknowledged direct introduction")
            }
            ContributionOutcome::UsefulCapabilityReferral => {
                (3, "acknowledged capability referral")
            }
            ContributionOutcome::AcknowledgedDownstreamDelivery => {
                (2, "authenticated downstream acknowledgement")
            }
            ContributionOutcome::CompletedResourceMatch => {
                (5, "acknowledged useful resource match")
            }
        };
        ProposedCredit {
            points,
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditExplanation {
    pub awarded_points: u16,
    pub reason: String,
    pub direct_contributor_only: bool,
    pub contributor_campaign_total: u16,
}

pub trait PropagationPolicyValidator {
    fn validate_hop(
        &self,
        campaign: &Campaign,
        parent: Option<&PropagationItem>,
        sender: &CthulhuId,
        recipient: &CthulhuId,
        now: i64,
    ) -> Result<String, PropagationError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StrictHopValidator;

impl PropagationPolicyValidator for StrictHopValidator {
    fn validate_hop(
        &self,
        campaign: &Campaign,
        parent: Option<&PropagationItem>,
        sender: &CthulhuId,
        _recipient: &CthulhuId,
        now: i64,
    ) -> Result<String, PropagationError> {
        campaign.validate()?;
        if campaign.revoked_at.is_some() {
            return Err(PropagationError::CampaignRevoked);
        }
        if now < campaign.created_at || now >= campaign.expires_at {
            return Err(PropagationError::CampaignExpired);
        }
        match parent {
            None if sender != &campaign.root => Err(PropagationError::SenderMismatch),
            Some(parent) if &parent.recipient != sender => Err(PropagationError::SenderMismatch),
            Some(parent)
                if !matches!(
                    parent.status,
                    ItemStatus::Accepted | ItemStatus::Acknowledged
                ) =>
            {
                Err(PropagationError::ParentNotAccepted)
            }
            _ => Ok("campaign and authenticated hop policy validated locally".into()),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct NoAdditionalHopPolicy;

impl PropagationPolicyValidator for NoAdditionalHopPolicy {
    fn validate_hop(
        &self,
        _campaign: &Campaign,
        _parent: Option<&PropagationItem>,
        _sender: &CthulhuId,
        _recipient: &CthulhuId,
        _now: i64,
    ) -> Result<String, PropagationError> {
        Ok("no additional local propagation restriction".into())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropagationEngine {
    campaigns: HashMap<CampaignId, Campaign>,
    items: HashMap<PropagationItemId, PropagationItem>,
    acknowledgements: HashMap<AcknowledgementId, Acknowledgement>,
    candidate_profiles: HashMap<CthulhuId, CandidateProfile>,
    opted_out: HashSet<CthulhuId>,
    /// `(owner, blocked)`; blocking applies in either direction for forwarding.
    blocked: HashSet<(CthulhuId, CthulhuId)>,
    local_policy_generation: u64,
    local_policy_events: Vec<LocalPolicyEvent>,
    rate_records: Vec<RateRecord>,
    outcomes: HashMap<OutcomeId, OutcomeClaim>,
    used_acknowledgements: HashSet<AcknowledgementId>,
    credits: Vec<ContributionCredit>,
}

impl PropagationEngine {
    #[allow(clippy::too_many_arguments)]
    pub fn create_campaign(
        &mut self,
        id: CampaignId,
        council_id: CouncilId,
        root: CthulhuId,
        payload: PropagationPayload,
        strategy: PropagationStrategy,
        policy: PropagationPolicy,
        created_at: i64,
        expires_at: i64,
    ) -> Result<&Campaign, PropagationError> {
        if self.campaigns.contains_key(&id) {
            return Err(PropagationError::DuplicateCampaign);
        }
        if self.campaigns.len() >= MAX_CAMPAIGNS {
            return Err(PropagationError::LimitExceeded("campaigns"));
        }
        payload.validate()?;
        let payload_hash = payload.content_hash()?;
        let campaign = Campaign {
            id: id.clone(),
            council_id,
            root,
            payload,
            payload_hash,
            strategy,
            policy,
            created_at,
            expires_at,
            revoked_at: None,
            revocation_reason: None,
        };
        campaign.validate()?;
        self.campaigns.insert(id.clone(), campaign);
        self.campaigns
            .get(&id)
            .ok_or(PropagationError::CampaignNotFound)
    }

    pub fn campaign(&self, id: &CampaignId) -> Option<&Campaign> {
        self.campaigns.get(id)
    }

    pub fn item(&self, id: &PropagationItemId) -> Option<&PropagationItem> {
        self.items.get(id)
    }

    pub fn register_candidate(
        &mut self,
        profile: CandidateProfile,
    ) -> Result<(), PropagationError> {
        profile.validate()?;
        if self
            .candidate_profiles
            .get(&profile.cthulhu_id)
            .is_some_and(|existing| existing != &profile)
            && self.items.values().any(|item| {
                item.provenance.hops.iter().any(|hop| {
                    hop.sender == profile.cthulhu_id || hop.recipient == profile.cthulhu_id
                })
            })
        {
            return Err(PropagationError::CandidateProfileInUse);
        }
        if self.candidate_profiles.len() >= MAX_CANDIDATES
            && !self.candidate_profiles.contains_key(&profile.cthulhu_id)
        {
            return Err(PropagationError::LimitExceeded("candidate profiles"));
        }
        self.candidate_profiles
            .insert(profile.cthulhu_id.clone(), profile);
        Ok(())
    }

    pub fn set_opt_out(
        &mut self,
        cthulhu: CthulhuId,
        opted_out: bool,
    ) -> Result<(), PropagationError> {
        if self.opted_out.contains(&cthulhu) == opted_out {
            return Ok(());
        }
        self.ensure_local_policy_event_capacity()?;
        let generation = self.next_local_policy_generation()?;
        if opted_out {
            if self.opted_out.len() >= MAX_LOCAL_RULES && !self.opted_out.contains(&cthulhu) {
                return Err(PropagationError::LimitExceeded("opt-out rules"));
            }
            self.opted_out.insert(cthulhu.clone());
        } else {
            self.opted_out.remove(&cthulhu);
        }
        self.local_policy_events.push(LocalPolicyEvent {
            generation,
            event: LocalPolicyEventKind::OptOut {
                cthulhu_id: cthulhu,
                enabled: opted_out,
            },
        });
        self.local_policy_generation = generation;
        Ok(())
    }

    pub fn set_blocked(
        &mut self,
        owner: CthulhuId,
        blocked: CthulhuId,
        is_blocked: bool,
    ) -> Result<(), PropagationError> {
        if owner == blocked {
            return Err(PropagationError::SelfReferral);
        }
        let pair = (owner.clone(), blocked.clone());
        if self.blocked.contains(&pair) == is_blocked {
            return Ok(());
        }
        self.ensure_local_policy_event_capacity()?;
        let generation = self.next_local_policy_generation()?;
        if is_blocked {
            if self.blocked.len() >= MAX_LOCAL_RULES && !self.blocked.contains(&pair) {
                return Err(PropagationError::LimitExceeded("block rules"));
            }
            self.blocked.insert(pair);
        } else {
            self.blocked.remove(&pair);
        }
        self.local_policy_events.push(LocalPolicyEvent {
            generation,
            event: LocalPolicyEventKind::Block {
                owner,
                blocked,
                enabled: is_blocked,
            },
        });
        self.local_policy_generation = generation;
        Ok(())
    }

    fn ensure_local_policy_event_capacity(&self) -> Result<(), PropagationError> {
        if self.local_policy_events.len() >= MAX_LOCAL_POLICY_EVENTS {
            return Err(PropagationError::LimitExceeded("local policy events"));
        }
        Ok(())
    }

    fn next_local_policy_generation(&self) -> Result<u64, PropagationError> {
        self.local_policy_generation
            .checked_add(1)
            .ok_or(PropagationError::LimitExceeded("local policy generation"))
    }

    pub fn send_initial(
        &mut self,
        campaign_id: &CampaignId,
        item_id: PropagationItemId,
        authenticated_sender: CthulhuId,
        recipient: CthulhuId,
        now: i64,
    ) -> Result<DeliveryResult, PropagationError> {
        self.send_with_validator(
            campaign_id,
            None,
            item_id,
            authenticated_sender,
            recipient,
            now,
            &NoAdditionalHopPolicy,
        )
    }

    pub fn forward(
        &mut self,
        parent_item_id: &PropagationItemId,
        item_id: PropagationItemId,
        authenticated_sender: CthulhuId,
        recipient: CthulhuId,
        now: i64,
    ) -> Result<DeliveryResult, PropagationError> {
        let campaign_id = self
            .items
            .get(parent_item_id)
            .ok_or(PropagationError::ItemNotFound)?
            .campaign_id
            .clone();
        self.send_with_validator(
            &campaign_id,
            Some(parent_item_id),
            item_id,
            authenticated_sender,
            recipient,
            now,
            &NoAdditionalHopPolicy,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_with_validator<V: PropagationPolicyValidator>(
        &mut self,
        campaign_id: &CampaignId,
        parent_item_id: Option<&PropagationItemId>,
        item_id: PropagationItemId,
        authenticated_sender: CthulhuId,
        recipient: CthulhuId,
        now: i64,
        validator: &V,
    ) -> Result<DeliveryResult, PropagationError> {
        if let Some(existing) = self.items.get(&item_id) {
            let same = &existing.campaign_id == campaign_id
                && existing.parent_item_id.as_ref() == parent_item_id
                && existing.sender == authenticated_sender
                && existing.recipient == recipient;
            return if same {
                Ok(DeliveryResult::ReplaySuppressed {
                    item: existing.clone(),
                })
            } else {
                Err(PropagationError::MessageIdConflict)
            };
        }
        if self.items.len() >= MAX_ITEMS {
            return Err(PropagationError::LimitExceeded("propagation items"));
        }
        if authenticated_sender == recipient {
            return Err(PropagationError::SelfReferral);
        }
        let campaign = self
            .campaigns
            .get(campaign_id)
            .ok_or(PropagationError::CampaignNotFound)?
            .clone();
        let parent = parent_item_id
            .map(|id| self.items.get(id).ok_or(PropagationError::ItemNotFound))
            .transpose()?
            .cloned();
        let strict_reason = StrictHopValidator.validate_hop(
            &campaign,
            parent.as_ref(),
            &authenticated_sender,
            &recipient,
            now,
        )?;
        let local_reason = validator.validate_hop(
            &campaign,
            parent.as_ref(),
            &authenticated_sender,
            &recipient,
            now,
        )?;
        validate_text(
            &local_reason,
            MAX_SHORT_TEXT_BYTES,
            "local policy explanation",
            false,
        )?;
        if self.opted_out.contains(&authenticated_sender) || self.opted_out.contains(&recipient) {
            return Err(PropagationError::OptedOut);
        }
        if self
            .blocked
            .contains(&(authenticated_sender.clone(), recipient.clone()))
            || self
                .blocked
                .contains(&(recipient.clone(), authenticated_sender.clone()))
        {
            return Err(PropagationError::Blocked);
        }
        if parent.as_ref().is_some_and(|parent| {
            parent.provenance.hops.iter().any(|hop| {
                self.blocked
                    .contains(&(recipient.clone(), hop.sender.clone()))
                    || self
                        .blocked
                        .contains(&(recipient.clone(), hop.recipient.clone()))
            })
        }) {
            return Err(PropagationError::Blocked);
        }
        if let Some(parent) = &parent
            && (parent.campaign_id != campaign.id
                || parent.policy_version != campaign.policy.version
                || parent.payload_hash != campaign.payload_hash)
        {
            return Err(PropagationError::PolicyMismatch);
        }

        let depth = parent
            .as_ref()
            .map_or(1, |item| item.depth.saturating_add(1));
        let mut reasons = vec![strict_reason, local_reason];
        self.validate_depth_and_strategy(
            &campaign,
            parent.as_ref(),
            &recipient,
            depth,
            &mut reasons,
        )?;
        let sender_profile = self
            .candidate_profiles
            .get(&authenticated_sender)
            .ok_or(PropagationError::CandidateUnknown)?;
        let recipient_profile = self
            .candidate_profiles
            .get(&recipient)
            .ok_or(PropagationError::CandidateUnknown)?;
        let sender_profile_hash = profile_hash(sender_profile)?;
        let recipient_profile_hash = profile_hash(recipient_profile)?;

        let prior_recipients = self
            .items
            .values()
            .any(|item| item.campaign_id == campaign.id && item.recipient == recipient);
        if prior_recipients {
            return Err(PropagationError::DuplicateDelivery);
        }

        let fan_out = self
            .items
            .values()
            .filter(|item| item.campaign_id == campaign.id && item.sender == authenticated_sender)
            .count();
        if fan_out >= campaign.policy.max_fan_out as usize {
            return Err(PropagationError::FanOutExceeded);
        }
        let recent = self
            .rate_records
            .iter()
            .filter(|record| {
                record.campaign_id == campaign.id
                    && record.sender == authenticated_sender
                    && record.sent_at > now - campaign.policy.rate_window_seconds
                    && record.sent_at <= now
            })
            .count();
        if recent >= campaign.policy.per_sender_rate_limit as usize {
            return Err(PropagationError::RateLimited);
        }

        let mut hops = parent
            .as_ref()
            .map_or_else(Vec::new, |item| item.provenance.hops.clone());
        let seen: HashSet<_> = hops
            .iter()
            .flat_map(|hop| [&hop.sender, &hop.recipient])
            .collect();
        if seen.contains(&recipient) {
            return Err(PropagationError::ReferralLoop);
        }
        if parent.is_none() && authenticated_sender != campaign.root {
            return Err(PropagationError::SenderMismatch);
        }
        hops.push(ProvenanceHop {
            item_id: item_id.clone(),
            sender: authenticated_sender.clone(),
            recipient: recipient.clone(),
            sent_at: now,
            local_policy_generation: self.local_policy_generation,
            sender_profile_hash,
            recipient_profile_hash,
        });
        let chain_hash = provenance_hash(
            &campaign.id,
            &campaign.payload_hash,
            campaign.policy.version,
            &hops,
        )?;
        let item = PropagationItem {
            id: item_id.clone(),
            campaign_id: campaign.id.clone(),
            parent_item_id: parent_item_id.cloned(),
            sender: authenticated_sender.clone(),
            recipient,
            depth,
            policy_version: campaign.policy.version,
            payload_hash: campaign.payload_hash.clone(),
            created_at: now,
            expires_at: campaign.expires_at,
            provenance: Provenance {
                root: campaign.root.clone(),
                hops,
                chain_hash,
            },
            status: ItemStatus::Pending,
        };
        self.validate_item_value(&item, now)?;
        self.items.insert(item_id.clone(), item.clone());
        self.rate_records.push(RateRecord {
            item_id,
            campaign_id: campaign.id,
            sender: authenticated_sender,
            sent_at: now,
        });
        reasons.push(
            "duplicate, loop, depth, fan-out, rate, opt-out, and blocking checks passed".into(),
        );
        let explanation = PropagationExplanation {
            allowed: true,
            reasons,
            depth,
            remaining_fan_out: campaign.policy.max_fan_out - fan_out as u16 - 1,
        };
        Ok(DeliveryResult::Created { item, explanation })
    }

    pub fn respond(
        &mut self,
        item_id: &PropagationItemId,
        authenticated_recipient: &CthulhuId,
        accept: bool,
        now: i64,
    ) -> Result<ItemStatus, PropagationError> {
        let campaign_id = self
            .items
            .get(item_id)
            .ok_or(PropagationError::ItemNotFound)?
            .campaign_id
            .clone();
        if self
            .campaigns
            .get(&campaign_id)
            .ok_or(PropagationError::CampaignNotFound)?
            .revoked_at
            .is_some()
        {
            return Err(PropagationError::CampaignRevoked);
        }
        let item = self
            .items
            .get_mut(item_id)
            .ok_or(PropagationError::ItemNotFound)?;
        if &item.recipient != authenticated_recipient {
            return Err(PropagationError::SenderMismatch);
        }
        if now < item.created_at || now >= item.expires_at {
            return Err(PropagationError::CampaignExpired);
        }
        let desired = if accept {
            ItemStatus::Accepted
        } else {
            ItemStatus::Rejected
        };
        match item.status {
            ItemStatus::Pending => item.status = desired,
            current if current == desired => {}
            _ => return Err(PropagationError::ConflictingResponse),
        }
        Ok(item.status)
    }

    pub fn acknowledge(
        &mut self,
        item_id: &PropagationItemId,
        acknowledgement_id: AcknowledgementId,
        authenticated_recipient: &CthulhuId,
        now: i64,
    ) -> Result<&Acknowledgement, PropagationError> {
        let evidence_hash = format!(
            "sha256:{:x}",
            Sha256::digest(format!("delivery:{}", item_id.as_str()).as_bytes())
        );
        self.acknowledge_outcome(
            item_id,
            acknowledgement_id,
            authenticated_recipient,
            ContributionOutcome::AcknowledgedDownstreamDelivery,
            evidence_hash,
            now,
        )
    }

    pub fn acknowledge_outcome(
        &mut self,
        item_id: &PropagationItemId,
        acknowledgement_id: AcknowledgementId,
        authenticated_recipient: &CthulhuId,
        outcome: ContributionOutcome,
        evidence_hash: String,
        now: i64,
    ) -> Result<&Acknowledgement, PropagationError> {
        validate_hash(&evidence_hash)?;
        if let Some(same) = self
            .acknowledgements
            .get(&acknowledgement_id)
            .map(|existing| {
                &existing.item_id == item_id
                    && &existing.actor == authenticated_recipient
                    && existing.outcome == outcome
                    && existing.evidence_hash == evidence_hash
            })
        {
            if !same {
                return Err(PropagationError::AcknowledgementConflict);
            }
            return self
                .acknowledgements
                .get(&acknowledgement_id)
                .ok_or(PropagationError::AcknowledgementNotFound);
        }
        if self.acknowledgements.len() >= MAX_ACKNOWLEDGEMENTS {
            return Err(PropagationError::LimitExceeded("acknowledgements"));
        }
        let campaign_id = self
            .items
            .get(item_id)
            .ok_or(PropagationError::ItemNotFound)?
            .campaign_id
            .clone();
        if self
            .campaigns
            .get(&campaign_id)
            .ok_or(PropagationError::CampaignNotFound)?
            .revoked_at
            .is_some()
        {
            return Err(PropagationError::CampaignRevoked);
        }
        let item = self
            .items
            .get_mut(item_id)
            .ok_or(PropagationError::ItemNotFound)?;
        if &item.recipient != authenticated_recipient {
            return Err(PropagationError::FakeAcknowledgement);
        }
        if item.status != ItemStatus::Accepted || now < item.created_at || now >= item.expires_at {
            return Err(PropagationError::FakeAcknowledgement);
        }
        item.status = ItemStatus::Acknowledged;
        let acknowledgement = Acknowledgement {
            id: acknowledgement_id.clone(),
            item_id: item_id.clone(),
            actor: authenticated_recipient.clone(),
            outcome,
            evidence_hash,
            acknowledged_at: now,
        };
        self.acknowledgements
            .insert(acknowledgement_id.clone(), acknowledgement);
        self.acknowledgements
            .get(&acknowledgement_id)
            .ok_or(PropagationError::AcknowledgementNotFound)
    }

    pub fn revoke_campaign(
        &mut self,
        campaign_id: &CampaignId,
        authenticated_root: &CthulhuId,
        reason: String,
        now: i64,
    ) -> Result<(), PropagationError> {
        validate_text(&reason, MAX_SHORT_TEXT_BYTES, "revocation reason", false)?;
        let campaign = self
            .campaigns
            .get_mut(campaign_id)
            .ok_or(PropagationError::CampaignNotFound)?;
        if &campaign.root != authenticated_root {
            return Err(PropagationError::SenderMismatch);
        }
        if now < campaign.created_at {
            return Err(PropagationError::InvalidTimestamp);
        }
        if campaign.revoked_at.is_none() {
            campaign.revoked_at = Some(now);
            campaign.revocation_reason = Some(reason);
        }
        Ok(())
    }

    pub fn validate_item(
        &self,
        item_id: &PropagationItemId,
        now: i64,
    ) -> Result<PropagationExplanation, PropagationError> {
        let item = self
            .items
            .get(item_id)
            .ok_or(PropagationError::ItemNotFound)?;
        if self
            .campaigns
            .get(&item.campaign_id)
            .ok_or(PropagationError::CampaignNotFound)?
            .revoked_at
            .is_some()
        {
            return Err(PropagationError::CampaignRevoked);
        }
        self.validate_item_value(item, now)?;
        let campaign = self
            .campaigns
            .get(&item.campaign_id)
            .ok_or(PropagationError::CampaignNotFound)?;
        let fan_out = self
            .items
            .values()
            .filter(|other| other.campaign_id == item.campaign_id && other.sender == item.sender)
            .count() as u16;
        Ok(PropagationExplanation {
            allowed: true,
            reasons: vec!["payload hash, policy version, provenance chain, sender continuity, expiry, and path uniqueness validated".into()],
            depth: item.depth,
            remaining_fan_out: campaign.policy.max_fan_out.saturating_sub(fan_out),
        })
    }

    pub fn validate_loaded_state(&self, now: i64) -> Result<(), PropagationError> {
        self.validate_loaded_state_with_policy(now, &NoAdditionalHopPolicy)
    }

    /// Revalidate a restored propagation graph against both the persisted Council policy and the
    /// operator's current local forwarding policy. This deliberately fails closed if current
    /// candidate, opt-out, or block policy no longer admits a recorded branch; callers may retain
    /// the protected snapshot for audit or migrate it explicitly, but must not resume forwarding
    /// from a branch that cannot be revalidated.
    pub fn validate_loaded_state_with_policy<V: PropagationPolicyValidator>(
        &self,
        now: i64,
        validator: &V,
    ) -> Result<(), PropagationError> {
        if now < 0
            || self.campaigns.len() > MAX_CAMPAIGNS
            || self.items.len() > MAX_ITEMS
            || self.acknowledgements.len() > MAX_ACKNOWLEDGEMENTS
            || self.candidate_profiles.len() > MAX_CANDIDATES
            || self.opted_out.len() > MAX_LOCAL_RULES
            || self.blocked.len() > MAX_LOCAL_RULES
            || self.local_policy_events.len() > MAX_LOCAL_POLICY_EVENTS
            || self.rate_records.len() > MAX_ITEMS
            || self.outcomes.len() > MAX_ACKNOWLEDGEMENTS
            || self.used_acknowledgements.len() > MAX_ACKNOWLEDGEMENTS
            || self.credits.len() > MAX_CANDIDATES * MAX_CAMPAIGNS
        {
            return Err(PropagationError::LimitExceeded("persisted state"));
        }
        for (key, campaign) in &self.campaigns {
            if key != &campaign.id {
                return Err(PropagationError::CorruptState("campaign map key"));
            }
            campaign.validate()?;
        }
        for (key, profile) in &self.candidate_profiles {
            if key != &profile.cthulhu_id {
                return Err(PropagationError::CorruptState("candidate map key"));
            }
            profile.validate()?;
        }
        if self.blocked.iter().any(|(owner, blocked)| owner == blocked) {
            return Err(PropagationError::CorruptState("self block rule"));
        }
        let mut recipients = HashSet::new();
        let mut fan_out: HashMap<(CampaignId, CthulhuId), usize> = HashMap::new();
        for (key, item) in &self.items {
            if key != &item.id {
                return Err(PropagationError::CorruptState("item map key"));
            }
            self.validate_item_value(
                item,
                now.min(item.expires_at.saturating_sub(1))
                    .max(item.created_at),
            )?;
            if !recipients.insert((item.campaign_id.clone(), item.recipient.clone())) {
                return Err(PropagationError::DuplicateDelivery);
            }
            *fan_out
                .entry((item.campaign_id.clone(), item.sender.clone()))
                .or_default() += 1;
        }
        self.validate_loaded_policy_history(validator)?;
        for ((campaign_id, _), count) in fan_out {
            let policy = &self
                .campaigns
                .get(&campaign_id)
                .ok_or(PropagationError::CampaignNotFound)?
                .policy;
            if count > policy.max_fan_out as usize {
                return Err(PropagationError::CorruptState("fan-out bound"));
            }
        }
        let mut rate_item_ids = HashSet::new();
        let mut rate_groups: HashMap<(CampaignId, CthulhuId), Vec<i64>> = HashMap::new();
        for record in &self.rate_records {
            let item = self
                .items
                .get(&record.item_id)
                .ok_or(PropagationError::CorruptState("rate item"))?;
            if !rate_item_ids.insert(record.item_id.clone())
                || record.campaign_id != item.campaign_id
                || record.sender != item.sender
                || record.sent_at != item.created_at
            {
                return Err(PropagationError::CorruptState("rate record"));
            }
            rate_groups
                .entry((record.campaign_id.clone(), record.sender.clone()))
                .or_default()
                .push(record.sent_at);
        }
        if rate_item_ids.len() != self.items.len() {
            return Err(PropagationError::CorruptState("missing rate record"));
        }
        for ((campaign_id, _), mut timestamps) in rate_groups {
            let policy = &self
                .campaigns
                .get(&campaign_id)
                .ok_or(PropagationError::CampaignNotFound)?
                .policy;
            timestamps.sort_unstable();
            let mut left = 0;
            for right in 0..timestamps.len() {
                while timestamps[left]
                    <= timestamps[right].saturating_sub(policy.rate_window_seconds)
                {
                    left += 1;
                }
                if right - left + 1 > policy.per_sender_rate_limit as usize {
                    return Err(PropagationError::CorruptState("rate bound"));
                }
            }
        }
        let mut acknowledged_items = HashSet::new();
        for (key, acknowledgement) in &self.acknowledgements {
            if key != &acknowledgement.id {
                return Err(PropagationError::CorruptState("acknowledgement map key"));
            }
            let item = self
                .items
                .get(&acknowledgement.item_id)
                .ok_or(PropagationError::ItemNotFound)?;
            validate_hash(&acknowledgement.evidence_hash)?;
            if !acknowledged_items.insert(acknowledgement.item_id.clone())
                || acknowledgement.actor != item.recipient
                || acknowledgement.acknowledged_at < item.created_at
                || acknowledgement.acknowledged_at >= item.expires_at
                || item.status != ItemStatus::Acknowledged
            {
                return Err(PropagationError::FakeAcknowledgement);
            }
        }
        for item in self.items.values() {
            if (item.status == ItemStatus::Acknowledged) != acknowledged_items.contains(&item.id) {
                return Err(PropagationError::CorruptState("item acknowledgement state"));
            }
        }
        let mut expected_used_acks = HashSet::new();
        for (key, claim) in &self.outcomes {
            if key != &claim.id || claim.contributor == claim.beneficiary {
                return Err(PropagationError::CorruptState("outcome metadata"));
            }
            let campaign = self
                .campaigns
                .get(&claim.campaign_id)
                .ok_or(PropagationError::CampaignNotFound)?;
            let item = self
                .items
                .get(&claim.item_id)
                .ok_or(PropagationError::ItemNotFound)?;
            let acknowledgement = self
                .acknowledgements
                .get(&claim.acknowledgement_id)
                .ok_or(PropagationError::AcknowledgementNotFound)?;
            if !expected_used_acks.insert(claim.acknowledgement_id.clone())
                || item.campaign_id != claim.campaign_id
                || item.sender != claim.contributor
                || item.recipient != claim.beneficiary
                || acknowledgement.item_id != item.id
                || acknowledgement.actor != claim.beneficiary
                || acknowledgement.outcome != claim.outcome
                || acknowledgement.evidence_hash != claim.evidence_hash
                || claim.occurred_at < acknowledgement.acknowledged_at
                || claim.occurred_at >= campaign.expires_at
                || campaign
                    .revoked_at
                    .is_some_and(|revoked_at| claim.occurred_at >= revoked_at)
            {
                return Err(PropagationError::InvalidCreditEvidence);
            }
            validate_hash(&claim.evidence_hash)?;
            validate_outcome_kind(&campaign.payload, claim.outcome)?;
        }
        if expected_used_acks != self.used_acknowledgements {
            return Err(PropagationError::CorruptState("used acknowledgements"));
        }
        let mut credit_keys = HashSet::new();
        for credit in &self.credits {
            if credit.points == 0
                || credit.points > MAX_CREDIT_PER_CTHULHU_PER_CAMPAIGN
                || !credit_keys.insert((credit.campaign_id.clone(), credit.cthulhu_id.clone()))
            {
                return Err(PropagationError::CorruptState("credit bounds"));
            }
            let outcome_count = self
                .outcomes
                .values()
                .filter(|claim| {
                    claim.campaign_id == credit.campaign_id
                        && claim.contributor == credit.cthulhu_id
                })
                .count();
            if outcome_count != credit.credited_outcomes as usize
                || credit.points as usize > outcome_count * MAX_CREDIT_PER_OUTCOME as usize
            {
                return Err(PropagationError::CorruptState("credit evidence"));
            }
        }
        for campaign_id in self.campaigns.keys() {
            let total = self
                .credits
                .iter()
                .filter(|credit| &credit.campaign_id == campaign_id)
                .map(|credit| credit.points)
                .fold(0u16, u16::saturating_add);
            if total > MAX_TOTAL_CAMPAIGN_CREDIT {
                return Err(PropagationError::CorruptState("campaign credit cap"));
            }
        }
        Ok(())
    }

    pub fn record_outcome<M: IncentiveModel>(
        &mut self,
        claim: OutcomeClaim,
        model: &M,
    ) -> Result<CreditExplanation, PropagationError> {
        validate_hash(&claim.evidence_hash)?;
        if self.outcomes.contains_key(&claim.id) {
            return Err(PropagationError::DuplicateOutcome);
        }
        if self
            .used_acknowledgements
            .contains(&claim.acknowledgement_id)
        {
            return Err(PropagationError::DuplicateCreditEvidence);
        }
        if claim.contributor == claim.beneficiary {
            return Err(PropagationError::SelfReferral);
        }
        let campaign = self
            .campaigns
            .get(&claim.campaign_id)
            .ok_or(PropagationError::CampaignNotFound)?;
        if campaign.revoked_at.is_some() || claim.occurred_at >= campaign.expires_at {
            return Err(PropagationError::CampaignRevoked);
        }
        let item = self
            .items
            .get(&claim.item_id)
            .ok_or(PropagationError::ItemNotFound)?;
        let acknowledgement = self
            .acknowledgements
            .get(&claim.acknowledgement_id)
            .ok_or(PropagationError::AcknowledgementNotFound)?;
        if item.campaign_id != claim.campaign_id
            || item.sender != claim.contributor
            || item.recipient != claim.beneficiary
            || acknowledgement.item_id != item.id
            || acknowledgement.actor != claim.beneficiary
            || acknowledgement.outcome != claim.outcome
            || acknowledgement.evidence_hash != claim.evidence_hash
            || item.status != ItemStatus::Acknowledged
            || claim.occurred_at < acknowledgement.acknowledged_at
        {
            return Err(PropagationError::InvalidCreditEvidence);
        }
        validate_outcome_kind(&campaign.payload, claim.outcome)?;

        let proposed = model.propose_credit(&CreditContext {
            campaign,
            item,
            claim: &claim,
        });
        validate_text(
            &proposed.reason,
            MAX_SHORT_TEXT_BYTES,
            "credit reason",
            false,
        )?;
        let existing = self
            .credits
            .iter()
            .find(|credit| {
                credit.campaign_id == claim.campaign_id && credit.cthulhu_id == claim.contributor
            })
            .map_or(0, |credit| credit.points);
        let campaign_total: u16 = self
            .credits
            .iter()
            .filter(|credit| credit.campaign_id == claim.campaign_id)
            .map(|credit| credit.points)
            .fold(0u16, u16::saturating_add);
        let awarded = proposed
            .points
            .min(MAX_CREDIT_PER_OUTCOME)
            .min(MAX_CREDIT_PER_CTHULHU_PER_CAMPAIGN.saturating_sub(existing))
            .min(MAX_TOTAL_CAMPAIGN_CREDIT.saturating_sub(campaign_total));
        if awarded == 0 {
            return Err(PropagationError::CreditCapReached);
        }
        match self.credits.iter_mut().find(|credit| {
            credit.campaign_id == claim.campaign_id && credit.cthulhu_id == claim.contributor
        }) {
            Some(credit) => {
                credit.points = credit.points.saturating_add(awarded);
                credit.credited_outcomes = credit.credited_outcomes.saturating_add(1);
            }
            None => self.credits.push(ContributionCredit {
                campaign_id: claim.campaign_id.clone(),
                cthulhu_id: claim.contributor.clone(),
                points: awarded,
                credited_outcomes: 1,
            }),
        }
        self.used_acknowledgements
            .insert(claim.acknowledgement_id.clone());
        self.outcomes.insert(claim.id.clone(), claim);
        Ok(CreditExplanation {
            awarded_points: awarded,
            reason: format!(
                "{}; no ancestor or recruitment-count credit was assigned",
                proposed.reason
            ),
            direct_contributor_only: true,
            contributor_campaign_total: existing + awarded,
        })
    }

    pub fn credits(&self) -> &[ContributionCredit] {
        &self.credits
    }

    pub fn rank_candidates(
        &self,
        campaign_id: &CampaignId,
        parent_item_id: Option<&PropagationItemId>,
    ) -> Result<Vec<CthulhuId>, PropagationError> {
        let campaign = self
            .campaigns
            .get(campaign_id)
            .ok_or(PropagationError::CampaignNotFound)?;
        let depth = parent_item_id
            .map(|id| self.items.get(id).ok_or(PropagationError::ItemNotFound))
            .transpose()?
            .map_or(1, |item| item.depth.saturating_add(1));
        let mut candidates: Vec<_> = self
            .candidate_profiles
            .values()
            .filter(|profile| {
                let mut reasons = Vec::new();
                self.strategy_allows(campaign, profile, depth, &mut reasons)
                    .is_ok()
            })
            .cloned()
            .collect();
        candidates.sort_by(|left, right| match &campaign.strategy {
            PropagationStrategy::GeographicOrLatencyAware { .. } => left
                .latency_ms
                .unwrap_or(u32::MAX)
                .cmp(&right.latency_ms.unwrap_or(u32::MAX))
                .then_with(|| left.cthulhu_id.cmp(&right.cthulhu_id)),
            PropagationStrategy::ReputationThresholded {
                accepted_sources, ..
            } => reputation_value(right, accepted_sources)
                .cmp(&reputation_value(left, accepted_sources))
                .then_with(|| left.cthulhu_id.cmp(&right.cthulhu_id)),
            _ => left.cthulhu_id.cmp(&right.cthulhu_id),
        });
        Ok(candidates
            .into_iter()
            .map(|profile| profile.cthulhu_id)
            .collect())
    }

    fn validate_depth_and_strategy(
        &self,
        campaign: &Campaign,
        parent: Option<&PropagationItem>,
        recipient: &CthulhuId,
        depth: u8,
        reasons: &mut Vec<String>,
    ) -> Result<(), PropagationError> {
        if depth == 0 || depth > campaign.policy.max_depth || depth as usize > MAX_PROVENANCE_HOPS {
            return Err(PropagationError::DepthExceeded);
        }
        if let Some(parent) = parent
            && parent
                .provenance
                .hops
                .iter()
                .any(|hop| &hop.sender == recipient || &hop.recipient == recipient)
        {
            return Err(PropagationError::ReferralLoop);
        }
        let profile = self
            .candidate_profiles
            .get(recipient)
            .ok_or(PropagationError::CandidateUnknown)?;
        if campaign.policy.visibility == CampaignVisibility::CouncilMembers
            && !profile.council_memberships.contains(&campaign.council_id)
        {
            return Err(PropagationError::VisibilityDenied);
        }
        if matches!(campaign.strategy, PropagationStrategy::TrustedBranchOnly) {
            if !profile.trusted {
                return Err(PropagationError::StrategyRejected);
            }
            if let Some(parent) = parent {
                let branch_is_trusted = parent
                    .provenance
                    .hops
                    .iter()
                    .flat_map(|hop| [&hop.sender, &hop.recipient])
                    .all(|cthulhu| {
                        self.candidate_profiles
                            .get(cthulhu)
                            .is_some_and(|candidate| candidate.trusted)
                    });
                if !branch_is_trusted {
                    return Err(PropagationError::StrategyRejected);
                }
            }
        }
        self.strategy_allows(campaign, profile, depth, reasons)
    }

    fn strategy_allows(
        &self,
        campaign: &Campaign,
        profile: &CandidateProfile,
        depth: u8,
        reasons: &mut Vec<String>,
    ) -> Result<(), PropagationError> {
        match &campaign.strategy {
            PropagationStrategy::BreadthFirst => {
                reasons.push("breadth-first candidate ordered by stable Cthulhu ID".into())
            }
            PropagationStrategy::DepthLimited { depth: limit } => {
                if depth > *limit {
                    return Err(PropagationError::DepthExceeded);
                }
                reasons.push(format!("depth {depth} is within strategy limit {limit}"));
            }
            PropagationStrategy::TrustedBranchOnly => {
                if !profile.trusted {
                    return Err(PropagationError::StrategyRejected);
                }
                reasons.push("recipient is in the locally trusted branch set".into());
            }
            PropagationStrategy::CapabilityTargeted { capability } => {
                if !profile.capability_tags.contains(capability) {
                    return Err(PropagationError::StrategyRejected);
                }
                reasons.push(format!(
                    "recipient advertises required capability {capability}"
                ));
            }
            PropagationStrategy::GeographicOrLatencyAware {
                preferred_region,
                max_latency_ms,
            } => {
                let latency = profile
                    .latency_ms
                    .ok_or(PropagationError::StrategyRejected)?;
                if latency > *max_latency_ms
                    || preferred_region
                        .as_ref()
                        .is_some_and(|region| profile.region.as_ref() != Some(region))
                {
                    return Err(PropagationError::StrategyRejected);
                }
                reasons.push(format!(
                    "recipient location and {latency}ms latency satisfy local bounds"
                ));
            }
            PropagationStrategy::ReputationThresholded {
                minimum_bps,
                accepted_sources,
            } => {
                let value = reputation_value(profile, accepted_sources);
                if value < *minimum_bps {
                    return Err(PropagationError::StrategyRejected);
                }
                reasons.push(format!(
                    "selected provenance-bearing reputation signal {value}bps meets threshold"
                ));
            }
        }
        Ok(())
    }

    fn validate_loaded_policy_history<V: PropagationPolicyValidator>(
        &self,
        validator: &V,
    ) -> Result<(), PropagationError> {
        if self.local_policy_generation != self.local_policy_events.len() as u64 {
            return Err(PropagationError::CorruptState("local policy generation"));
        }

        let mut final_opted_out = HashSet::new();
        let mut final_blocked = HashSet::new();
        for (index, event) in self.local_policy_events.iter().enumerate() {
            if event.generation != index as u64 + 1 {
                return Err(PropagationError::CorruptState("local policy generation"));
            }
            Self::apply_local_policy_event(event, &mut final_opted_out, &mut final_blocked)?;
        }
        if final_opted_out != self.opted_out || final_blocked != self.blocked {
            return Err(PropagationError::CorruptState("local policy state"));
        }

        let mut items = self.items.values().collect::<Vec<_>>();
        items.sort_by(|left, right| {
            let left_generation = left
                .provenance
                .hops
                .last()
                .map_or(u64::MAX, |hop| hop.local_policy_generation);
            let right_generation = right
                .provenance
                .hops
                .last()
                .map_or(u64::MAX, |hop| hop.local_policy_generation);
            left_generation
                .cmp(&right_generation)
                .then_with(|| left.id.cmp(&right.id))
        });
        let mut replayed_opted_out = HashSet::new();
        let mut replayed_blocked = HashSet::new();
        let mut event_index = 0;
        for item in items {
            let generation = item
                .provenance
                .hops
                .last()
                .ok_or(PropagationError::ForgedProvenance)?
                .local_policy_generation;
            if generation > self.local_policy_generation {
                return Err(PropagationError::ForgedProvenance);
            }
            while event_index < self.local_policy_events.len()
                && self.local_policy_events[event_index].generation <= generation
            {
                Self::apply_local_policy_event(
                    &self.local_policy_events[event_index],
                    &mut replayed_opted_out,
                    &mut replayed_blocked,
                )?;
                event_index += 1;
            }
            self.validate_loaded_item_policy(
                item,
                validator,
                &replayed_opted_out,
                &replayed_blocked,
            )?;
        }
        Ok(())
    }

    fn apply_local_policy_event(
        event: &LocalPolicyEvent,
        opted_out: &mut HashSet<CthulhuId>,
        blocked: &mut HashSet<(CthulhuId, CthulhuId)>,
    ) -> Result<(), PropagationError> {
        let changed = match &event.event {
            LocalPolicyEventKind::OptOut {
                cthulhu_id,
                enabled,
            } => {
                if *enabled {
                    opted_out.insert(cthulhu_id.clone())
                } else {
                    opted_out.remove(cthulhu_id)
                }
            }
            LocalPolicyEventKind::Block {
                owner,
                blocked: blocked_cthulhu,
                enabled,
            } => {
                if owner == blocked_cthulhu {
                    return Err(PropagationError::CorruptState("self block rule"));
                }
                let pair = (owner.clone(), blocked_cthulhu.clone());
                if *enabled {
                    blocked.insert(pair)
                } else {
                    blocked.remove(&pair)
                }
            }
        };
        if !changed || opted_out.len() > MAX_LOCAL_RULES || blocked.len() > MAX_LOCAL_RULES {
            return Err(PropagationError::CorruptState("local policy event"));
        }
        Ok(())
    }

    fn validate_loaded_item_policy<V: PropagationPolicyValidator>(
        &self,
        item: &PropagationItem,
        validator: &V,
        opted_out: &HashSet<CthulhuId>,
        blocked: &HashSet<(CthulhuId, CthulhuId)>,
    ) -> Result<(), PropagationError> {
        let campaign = self
            .campaigns
            .get(&item.campaign_id)
            .ok_or(PropagationError::CampaignNotFound)?;
        if campaign
            .revoked_at
            .is_some_and(|revoked_at| item.created_at >= revoked_at)
        {
            return Err(PropagationError::CampaignRevoked);
        }

        let parent = item
            .parent_item_id
            .as_ref()
            .map(|id| self.items.get(id).ok_or(PropagationError::ItemNotFound))
            .transpose()?;
        match parent {
            None if item.sender != campaign.root => {
                return Err(PropagationError::SenderMismatch);
            }
            Some(parent) if parent.recipient != item.sender => {
                return Err(PropagationError::SenderMismatch);
            }
            Some(parent)
                if !matches!(
                    parent.status,
                    ItemStatus::Accepted | ItemStatus::Acknowledged
                ) =>
            {
                return Err(PropagationError::ParentNotAccepted);
            }
            _ => {}
        }

        if opted_out.contains(&item.sender) || opted_out.contains(&item.recipient) {
            return Err(PropagationError::OptedOut);
        }
        if blocked.contains(&(item.sender.clone(), item.recipient.clone()))
            || blocked.contains(&(item.recipient.clone(), item.sender.clone()))
        {
            return Err(PropagationError::Blocked);
        }
        if parent.is_some_and(|parent| {
            parent.provenance.hops.iter().any(|hop| {
                blocked.contains(&(item.recipient.clone(), hop.sender.clone()))
                    || blocked.contains(&(item.recipient.clone(), hop.recipient.clone()))
            })
        }) {
            return Err(PropagationError::Blocked);
        }

        let mut reasons = Vec::new();
        self.validate_depth_and_strategy(
            campaign,
            parent,
            &item.recipient,
            item.depth,
            &mut reasons,
        )?;
        let local_reason = validator.validate_hop(
            campaign,
            parent,
            &item.sender,
            &item.recipient,
            item.created_at,
        )?;
        validate_text(
            &local_reason,
            MAX_SHORT_TEXT_BYTES,
            "local policy explanation",
            false,
        )
    }

    fn validate_item_value(
        &self,
        item: &PropagationItem,
        now: i64,
    ) -> Result<(), PropagationError> {
        let campaign = self
            .campaigns
            .get(&item.campaign_id)
            .ok_or(PropagationError::CampaignNotFound)?;
        if item.policy_version != campaign.policy.version
            || item.payload_hash != campaign.payload_hash
            || item.expires_at != campaign.expires_at
        {
            return Err(PropagationError::PolicyMismatch);
        }
        if now < item.created_at || now >= item.expires_at {
            return Err(PropagationError::CampaignExpired);
        }
        if item.depth as usize != item.provenance.hops.len()
            || item.depth == 0
            || item.depth > campaign.policy.max_depth
        {
            return Err(PropagationError::DepthExceeded);
        }
        if item.provenance.root != campaign.root {
            return Err(PropagationError::ForgedProvenance);
        }
        let first = item
            .provenance
            .hops
            .first()
            .ok_or(PropagationError::ForgedProvenance)?;
        let last = item
            .provenance
            .hops
            .last()
            .ok_or(PropagationError::ForgedProvenance)?;
        if first.sender != campaign.root
            || last.item_id != item.id
            || last.sender != item.sender
            || last.recipient != item.recipient
            || last.sent_at != item.created_at
            || last.local_policy_generation > self.local_policy_generation
        {
            return Err(PropagationError::ForgedProvenance);
        }
        let sender_profile = self
            .candidate_profiles
            .get(&item.sender)
            .ok_or(PropagationError::CandidateUnknown)?;
        let recipient_profile = self
            .candidate_profiles
            .get(&item.recipient)
            .ok_or(PropagationError::CandidateUnknown)?;
        if profile_hash(sender_profile)? != last.sender_profile_hash
            || profile_hash(recipient_profile)? != last.recipient_profile_hash
        {
            return Err(PropagationError::ForgedProvenance);
        }
        let mut seen = HashSet::new();
        seen.insert(campaign.root.clone());
        for (index, hop) in item.provenance.hops.iter().enumerate() {
            validate_hash(&hop.sender_profile_hash)?;
            validate_hash(&hop.recipient_profile_hash)?;
            if hop.sent_at < campaign.created_at
                || hop.sent_at >= campaign.expires_at
                || !seen.insert(hop.recipient.clone())
            {
                return Err(PropagationError::ForgedProvenance);
            }
            if index > 0 {
                let previous = &item.provenance.hops[index - 1];
                if previous.recipient != hop.sender
                    || previous.recipient_profile_hash != hop.sender_profile_hash
                    || previous.local_policy_generation > hop.local_policy_generation
                {
                    return Err(PropagationError::ForgedProvenance);
                }
            }
        }
        if let Some(parent_id) = &item.parent_item_id {
            let parent = self
                .items
                .get(parent_id)
                .ok_or(PropagationError::ItemNotFound)?;
            if parent.campaign_id != item.campaign_id
                || parent.recipient != item.sender
                || parent.provenance.hops.as_slice()
                    != &item.provenance.hops[..item.provenance.hops.len() - 1]
            {
                return Err(PropagationError::ForgedProvenance);
            }
        } else if item.depth != 1 {
            return Err(PropagationError::ForgedProvenance);
        }
        let expected = provenance_hash(
            &campaign.id,
            &campaign.payload_hash,
            campaign.policy.version,
            &item.provenance.hops,
        )?;
        if expected != item.provenance.chain_hash {
            return Err(PropagationError::ForgedProvenance);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum PropagationError {
    #[error("malformed {0}")]
    MalformedIdentifier(&'static str),
    #[error("invalid propagation policy: {0}")]
    InvalidPolicy(&'static str),
    #[error("invalid typed payload field: {0}")]
    InvalidPayload(&'static str),
    #[error("invalid content hash")]
    InvalidHash,
    #[error("{0} exceeds its configured bound")]
    LimitExceeded(&'static str),
    #[error("invalid timestamp")]
    InvalidTimestamp,
    #[error("invalid campaign expiry")]
    InvalidExpiry,
    #[error("campaign already exists")]
    DuplicateCampaign,
    #[error("campaign was not found")]
    CampaignNotFound,
    #[error("propagation item was not found")]
    ItemNotFound,
    #[error("acknowledgement was not found")]
    AcknowledgementNotFound,
    #[error("campaign expired")]
    CampaignExpired,
    #[error("campaign was revoked")]
    CampaignRevoked,
    #[error("authenticated sender does not match the protocol operation")]
    SenderMismatch,
    #[error("parent item has not been accepted")]
    ParentNotAccepted,
    #[error("message ID was reused for different content")]
    MessageIdConflict,
    #[error("candidate is not in the locally validated candidate set")]
    CandidateUnknown,
    #[error("a candidate profile referenced by propagation history is immutable")]
    CandidateProfileInUse,
    #[error("recipient does not satisfy the deterministic propagation strategy")]
    StrategyRejected,
    #[error("campaign visibility policy denied this recipient")]
    VisibilityDenied,
    #[error("maximum propagation depth exceeded")]
    DepthExceeded,
    #[error("maximum sender fan-out exceeded")]
    FanOutExceeded,
    #[error("per-sender propagation rate exceeded")]
    RateLimited,
    #[error("Cthulhu opted out of propagation")]
    OptedOut,
    #[error("a local block rule denied propagation")]
    Blocked,
    #[error("self-referrals are forbidden")]
    SelfReferral,
    #[error("referral loop detected")]
    ReferralLoop,
    #[error("campaign was already delivered to this recipient")]
    DuplicateDelivery,
    #[error("policy version or payload hash mismatch")]
    PolicyMismatch,
    #[error("payload hash mismatch")]
    PayloadHashMismatch,
    #[error("provenance chain could not be verified")]
    ForgedProvenance,
    #[error("accept/reject response conflicts with existing state")]
    ConflictingResponse,
    #[error("acknowledgement ID conflicts with existing state")]
    AcknowledgementConflict,
    #[error("acknowledgement was not made by the authenticated recipient")]
    FakeAcknowledgement,
    #[error("outcome was already credited")]
    DuplicateOutcome,
    #[error("acknowledgement was already used as credit evidence")]
    DuplicateCreditEvidence,
    #[error("outcome does not match its authenticated evidence")]
    InvalidCreditEvidence,
    #[error("outcome type is not applicable to this campaign")]
    InapplicableOutcome,
    #[error("bounded contribution credit cap reached")]
    CreditCapReached,
    #[error("failed to serialize propagation state: {0}")]
    Serialization(serde_json::Error),
    #[error("persisted propagation state failed validation: {0}")]
    CorruptState(&'static str),
}

fn validate_outcome_kind(
    payload: &PropagationPayload,
    outcome: ContributionOutcome,
) -> Result<(), PropagationError> {
    let valid = match outcome {
        ContributionOutcome::SuccessfulIntroduction => {
            matches!(payload, PropagationPayload::CouncilInvitation { .. })
        }
        ContributionOutcome::UsefulCapabilityReferral => {
            matches!(payload, PropagationPayload::CapabilityRequest { .. })
        }
        ContributionOutcome::AcknowledgedDownstreamDelivery => true,
        ContributionOutcome::CompletedResourceMatch => {
            matches!(
                payload,
                PropagationPayload::ResourceNeed { .. } | PropagationPayload::ResourceOffer { .. }
            )
        }
    };
    valid
        .then_some(())
        .ok_or(PropagationError::InapplicableOutcome)
}

fn reputation_value(profile: &CandidateProfile, accepted_sources: &[String]) -> u16 {
    profile
        .reputation_signals
        .iter()
        .filter(|signal| accepted_sources.contains(&signal.source))
        .map(|signal| signal.value_bps)
        .max()
        .unwrap_or(0)
}

fn provenance_hash(
    campaign_id: &CampaignId,
    payload_hash: &str,
    policy_version: u32,
    hops: &[ProvenanceHop],
) -> Result<String, PropagationError> {
    #[derive(Serialize)]
    struct HashMaterial<'a> {
        campaign_id: &'a CampaignId,
        payload_hash: &'a str,
        policy_version: u32,
        hops: &'a [ProvenanceHop],
    }
    let bytes = serde_json::to_vec(&HashMaterial {
        campaign_id,
        payload_hash,
        policy_version,
        hops,
    })
    .map_err(PropagationError::Serialization)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn profile_hash(profile: &CandidateProfile) -> Result<String, PropagationError> {
    profile.validate()?;
    let bytes = serde_json::to_vec(profile).map_err(PropagationError::Serialization)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn validate_hash(value: &str) -> Result<(), PropagationError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(PropagationError::InvalidHash);
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(PropagationError::InvalidHash);
    }
    Ok(())
}

fn validate_text(
    value: &str,
    max: usize,
    field: &'static str,
    single_line: bool,
) -> Result<(), PropagationError> {
    if value.trim().is_empty()
        || value.len() > max
        || value.contains('\0')
        || (single_line && value.contains(['\r', '\n']))
    {
        return Err(PropagationError::InvalidPayload(field));
    }
    Ok(())
}

fn validate_token(value: &str, field: &'static str) -> Result<(), PropagationError> {
    if value.is_empty()
        || value.len() > 96
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        return Err(PropagationError::InvalidPayload(field));
    }
    Ok(())
}

fn validate_tags(values: &[String], field: &'static str) -> Result<(), PropagationError> {
    if values.is_empty() || values.len() > MAX_LIST_ITEMS {
        return Err(PropagationError::LimitExceeded(field));
    }
    let mut unique = HashSet::new();
    for value in values {
        validate_token(value, field)?;
        if !unique.insert(value) {
            return Err(PropagationError::InvalidPayload(field));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cthulhu(name: &str) -> CthulhuId {
        CthulhuId::parse(&format!("cthulhu_{name}")).unwrap()
    }
    fn campaign(name: &str) -> CampaignId {
        CampaignId::parse(&format!("propagation_{name}")).unwrap()
    }
    fn item(name: &str) -> PropagationItemId {
        PropagationItemId::parse(&format!("msg_{name}")).unwrap()
    }
    fn ack(name: &str) -> AcknowledgementId {
        AcknowledgementId::parse(&format!("ack_{name}")).unwrap()
    }
    fn outcome(name: &str) -> OutcomeId {
        OutcomeId::parse(&format!("outcome_{name}")).unwrap()
    }
    fn council() -> CouncilId {
        CouncilId::parse("council_test").unwrap()
    }
    fn hash() -> String {
        format!("sha256:{}", "a".repeat(64))
    }
    fn policy(depth: u8, fan_out: u16) -> PropagationPolicy {
        PropagationPolicy {
            version: 1,
            max_depth: depth,
            max_fan_out: fan_out,
            per_sender_rate_limit: 10,
            rate_window_seconds: 60,
            visibility: CampaignVisibility::InvitedBranches,
        }
    }
    fn profile(name: &str) -> CandidateProfile {
        CandidateProfile {
            cthulhu_id: cthulhu(name),
            trusted: name != "trickster",
            council_memberships: vec![council()],
            capability_tags: vec![
                if name == "merchant" {
                    "commerce"
                } else {
                    "archive"
                }
                .into(),
            ],
            region: Some("us-west".into()),
            latency_ms: Some(if name == "hermit" { 40 } else { 10 }),
            reputation_signals: vec![ReputationSignal {
                source: "local-allowlist".into(),
                value_bps: if name == "trickster" { 100 } else { 8_000 },
                observed_at: 0,
            }],
        }
    }
    fn invitation() -> PropagationPayload {
        PropagationPayload::CouncilInvitation {
            council_id: council(),
            summary: "join the test Council".into(),
        }
    }
    fn engine_with_campaign(
        strategy: PropagationStrategy,
        depth: u8,
        fan_out: u16,
    ) -> PropagationEngine {
        let mut engine = PropagationEngine::default();
        for name in ["archivist", "hermit", "merchant", "oracle", "trickster"] {
            engine.register_candidate(profile(name)).unwrap();
        }
        engine
            .create_campaign(
                campaign("test"),
                council(),
                cthulhu("archivist"),
                invitation(),
                strategy,
                policy(depth, fan_out),
                0,
                100,
            )
            .unwrap();
        engine
    }
    fn deliver_and_accept(
        engine: &mut PropagationEngine,
        parent: Option<&PropagationItemId>,
        item_id: PropagationItemId,
        sender: CthulhuId,
        recipient: CthulhuId,
        now: i64,
    ) {
        if let Some(parent) = parent {
            engine
                .forward(parent, item_id.clone(), sender, recipient.clone(), now)
                .unwrap();
        } else {
            engine
                .send_initial(
                    &campaign("test"),
                    item_id.clone(),
                    sender,
                    recipient.clone(),
                    now,
                )
                .unwrap();
        }
        engine.respond(&item_id, &recipient, true, now + 1).unwrap();
    }

    #[test]
    fn invitation_can_be_accepted_or_rejected_idempotently() {
        let mut engine = engine_with_campaign(PropagationStrategy::BreadthFirst, 4, 4);
        let first = item("first");
        engine
            .send_initial(
                &campaign("test"),
                first.clone(),
                cthulhu("archivist"),
                cthulhu("hermit"),
                1,
            )
            .unwrap();
        assert_eq!(
            engine.respond(&first, &cthulhu("hermit"), true, 2).unwrap(),
            ItemStatus::Accepted
        );
        assert_eq!(
            engine.respond(&first, &cthulhu("hermit"), true, 2).unwrap(),
            ItemStatus::Accepted
        );
        let second = item("second");
        engine
            .send_initial(
                &campaign("test"),
                second.clone(),
                cthulhu("archivist"),
                cthulhu("merchant"),
                3,
            )
            .unwrap();
        assert_eq!(
            engine
                .respond(&second, &cthulhu("merchant"), false, 4)
                .unwrap(),
            ItemStatus::Rejected
        );
        assert!(matches!(
            engine.respond(&second, &cthulhu("merchant"), true, 5),
            Err(PropagationError::ConflictingResponse)
        ));
    }

    #[test]
    fn depth_fanout_and_rate_are_bounded() {
        let mut engine = engine_with_campaign(PropagationStrategy::BreadthFirst, 2, 1);
        let first = item("depthone");
        deliver_and_accept(
            &mut engine,
            None,
            first.clone(),
            cthulhu("archivist"),
            cthulhu("hermit"),
            1,
        );
        assert!(matches!(
            engine.send_initial(
                &campaign("test"),
                item("fanout"),
                cthulhu("archivist"),
                cthulhu("merchant"),
                2
            ),
            Err(PropagationError::FanOutExceeded)
        ));
        let second = item("depthtwo");
        deliver_and_accept(
            &mut engine,
            Some(&first),
            second.clone(),
            cthulhu("hermit"),
            cthulhu("merchant"),
            3,
        );
        assert!(matches!(
            engine.forward(
                &second,
                item("toodeep"),
                cthulhu("merchant"),
                cthulhu("oracle"),
                5
            ),
            Err(PropagationError::DepthExceeded)
        ));

        let mut rate_engine = engine_with_campaign(PropagationStrategy::BreadthFirst, 2, 4);
        rate_engine
            .campaigns
            .get_mut(&campaign("test"))
            .unwrap()
            .policy
            .per_sender_rate_limit = 1;
        rate_engine
            .send_initial(
                &campaign("test"),
                item("rateone"),
                cthulhu("archivist"),
                cthulhu("hermit"),
                1,
            )
            .unwrap();
        assert!(matches!(
            rate_engine.send_initial(
                &campaign("test"),
                item("ratetwo"),
                cthulhu("archivist"),
                cthulhu("merchant"),
                2,
            ),
            Err(PropagationError::RateLimited)
        ));
    }

    #[test]
    fn loops_duplicates_and_replays_are_suppressed() {
        let mut engine = engine_with_campaign(PropagationStrategy::BreadthFirst, 4, 4);
        let first = item("first");
        deliver_and_accept(
            &mut engine,
            None,
            first.clone(),
            cthulhu("archivist"),
            cthulhu("hermit"),
            1,
        );
        assert!(matches!(
            engine
                .send_initial(
                    &campaign("test"),
                    first.clone(),
                    cthulhu("archivist"),
                    cthulhu("hermit"),
                    1
                )
                .unwrap(),
            DeliveryResult::ReplaySuppressed { .. }
        ));
        assert!(matches!(
            engine.forward(
                &first,
                item("loop"),
                cthulhu("hermit"),
                cthulhu("archivist"),
                3
            ),
            Err(PropagationError::ReferralLoop)
        ));
        assert!(matches!(
            engine.forward(
                &first,
                item("duplicate"),
                cthulhu("hermit"),
                cthulhu("hermit"),
                3
            ),
            Err(PropagationError::SelfReferral)
        ));
    }

    #[test]
    fn provenance_tampering_is_detected() {
        let mut engine = engine_with_campaign(PropagationStrategy::BreadthFirst, 4, 4);
        let first = item("first");
        deliver_and_accept(
            &mut engine,
            None,
            first.clone(),
            cthulhu("archivist"),
            cthulhu("hermit"),
            1,
        );
        engine.items.get_mut(&first).unwrap().provenance.chain_hash = hash();
        assert!(matches!(
            engine.validate_item(&first, 2),
            Err(PropagationError::ForgedProvenance)
        ));
    }

    #[test]
    fn revocation_optout_and_blocking_are_enforced() {
        let mut engine = engine_with_campaign(PropagationStrategy::BreadthFirst, 4, 4);
        engine.set_opt_out(cthulhu("hermit"), true).unwrap();
        assert!(matches!(
            engine.send_initial(
                &campaign("test"),
                item("optout"),
                cthulhu("archivist"),
                cthulhu("hermit"),
                1
            ),
            Err(PropagationError::OptedOut)
        ));
        engine.set_opt_out(cthulhu("hermit"), false).unwrap();
        engine
            .set_blocked(cthulhu("hermit"), cthulhu("archivist"), true)
            .unwrap();
        assert!(matches!(
            engine.send_initial(
                &campaign("test"),
                item("blocked"),
                cthulhu("archivist"),
                cthulhu("hermit"),
                1
            ),
            Err(PropagationError::Blocked)
        ));
        engine
            .set_blocked(cthulhu("hermit"), cthulhu("archivist"), false)
            .unwrap();
        engine
            .revoke_campaign(
                &campaign("test"),
                &cthulhu("archivist"),
                "operator revoked campaign".into(),
                2,
            )
            .unwrap();
        assert!(matches!(
            engine.send_initial(
                &campaign("test"),
                item("revoked"),
                cthulhu("archivist"),
                cthulhu("hermit"),
                3
            ),
            Err(PropagationError::CampaignRevoked)
        ));

        struct Permissive;
        impl PropagationPolicyValidator for Permissive {
            fn validate_hop(
                &self,
                _campaign: &Campaign,
                _parent: Option<&PropagationItem>,
                _sender: &CthulhuId,
                _recipient: &CthulhuId,
                _now: i64,
            ) -> Result<String, PropagationError> {
                Ok("permit everything".into())
            }
        }
        assert!(matches!(
            engine.send_with_validator(
                &campaign("test"),
                None,
                item("customvalidator"),
                cthulhu("archivist"),
                cthulhu("oracle"),
                4,
                &Permissive,
            ),
            Err(PropagationError::CampaignRevoked)
        ));
    }

    #[test]
    fn deterministic_strategies_select_meaningfully_different_branches() {
        let trusted = engine_with_campaign(PropagationStrategy::TrustedBranchOnly, 4, 4);
        let ranked = trusted.rank_candidates(&campaign("test"), None).unwrap();
        assert!(!ranked.contains(&cthulhu("trickster")));

        let capability = engine_with_campaign(
            PropagationStrategy::CapabilityTargeted {
                capability: "commerce".into(),
            },
            4,
            4,
        );
        assert_eq!(
            capability.rank_candidates(&campaign("test"), None).unwrap(),
            vec![cthulhu("merchant")]
        );

        let latency = engine_with_campaign(
            PropagationStrategy::GeographicOrLatencyAware {
                preferred_region: Some("us-west".into()),
                max_latency_ms: 25,
            },
            4,
            4,
        );
        assert!(
            !latency
                .rank_candidates(&campaign("test"), None)
                .unwrap()
                .contains(&cthulhu("hermit"))
        );

        let reputation = engine_with_campaign(
            PropagationStrategy::ReputationThresholded {
                minimum_bps: 5_000,
                accepted_sources: vec!["local-allowlist".into()],
            },
            4,
            4,
        );
        assert!(
            !reputation
                .rank_candidates(&campaign("test"), None)
                .unwrap()
                .contains(&cthulhu("trickster"))
        );
    }

    #[test]
    fn fake_acknowledgements_and_duplicate_credit_are_rejected() {
        let mut engine = engine_with_campaign(PropagationStrategy::BreadthFirst, 4, 4);
        let first = item("credit");
        deliver_and_accept(
            &mut engine,
            None,
            first.clone(),
            cthulhu("archivist"),
            cthulhu("hermit"),
            1,
        );
        assert!(matches!(
            engine.acknowledge(&first, ack("fake"), &cthulhu("merchant"), 3),
            Err(PropagationError::FakeAcknowledgement)
        ));
        engine
            .acknowledge_outcome(
                &first,
                ack("real"),
                &cthulhu("hermit"),
                ContributionOutcome::SuccessfulIntroduction,
                hash(),
                3,
            )
            .unwrap();
        let claim = OutcomeClaim {
            id: outcome("intro"),
            campaign_id: campaign("test"),
            item_id: first,
            acknowledgement_id: ack("real"),
            contributor: cthulhu("archivist"),
            beneficiary: cthulhu("hermit"),
            outcome: ContributionOutcome::SuccessfulIntroduction,
            evidence_hash: hash(),
            occurred_at: 4,
        };
        let explanation = engine
            .record_outcome(claim.clone(), &SafeOutcomeCredit)
            .unwrap();
        assert_eq!(explanation.awarded_points, 1);
        assert!(explanation.direct_contributor_only);
        assert!(matches!(
            engine.record_outcome(claim, &SafeOutcomeCredit),
            Err(PropagationError::DuplicateOutcome)
        ));
    }

    #[test]
    fn generic_delivery_ack_cannot_be_relabelled_as_a_useful_outcome() {
        let mut engine = engine_with_campaign(PropagationStrategy::BreadthFirst, 4, 4);
        let first = item("genericack");
        deliver_and_accept(
            &mut engine,
            None,
            first.clone(),
            cthulhu("archivist"),
            cthulhu("hermit"),
            1,
        );
        let acknowledgement = engine
            .acknowledge(&first, ack("generic"), &cthulhu("hermit"), 3)
            .unwrap()
            .clone();
        assert!(matches!(
            engine.record_outcome(
                OutcomeClaim {
                    id: outcome("relabeled"),
                    campaign_id: campaign("test"),
                    item_id: first,
                    acknowledgement_id: acknowledgement.id,
                    contributor: cthulhu("archivist"),
                    beneficiary: cthulhu("hermit"),
                    outcome: ContributionOutcome::SuccessfulIntroduction,
                    evidence_hash: acknowledgement.evidence_hash,
                    occurred_at: 4,
                },
                &SafeOutcomeCredit,
            ),
            Err(PropagationError::InvalidCreditEvidence)
        ));
    }

    struct ExcessiveCredit;
    impl IncentiveModel for ExcessiveCredit {
        fn propose_credit(&self, _context: &CreditContext<'_>) -> ProposedCredit {
            ProposedCredit {
                points: u16::MAX,
                reason: "custom model requested excessive credit".into(),
            }
        }
    }

    #[test]
    fn incentive_models_are_clamped_and_no_ancestor_credit_is_created() {
        let mut engine = engine_with_campaign(PropagationStrategy::BreadthFirst, 4, 4);
        let first = item("clamped");
        deliver_and_accept(
            &mut engine,
            None,
            first.clone(),
            cthulhu("archivist"),
            cthulhu("hermit"),
            1,
        );
        engine
            .acknowledge_outcome(
                &first,
                ack("clamped"),
                &cthulhu("hermit"),
                ContributionOutcome::AcknowledgedDownstreamDelivery,
                hash(),
                3,
            )
            .unwrap();
        let explanation = engine
            .record_outcome(
                OutcomeClaim {
                    id: outcome("clamped"),
                    campaign_id: campaign("test"),
                    item_id: first,
                    acknowledgement_id: ack("clamped"),
                    contributor: cthulhu("archivist"),
                    beneficiary: cthulhu("hermit"),
                    outcome: ContributionOutcome::AcknowledgedDownstreamDelivery,
                    evidence_hash: hash(),
                    occurred_at: 4,
                },
                &ExcessiveCredit,
            )
            .unwrap();
        assert_eq!(explanation.awarded_points, MAX_CREDIT_PER_OUTCOME);
        assert_eq!(engine.credits().len(), 1);
        assert_eq!(engine.credits()[0].cthulhu_id, cthulhu("archivist"));
    }

    #[test]
    fn persisted_state_reloads_without_duplicate_effects() {
        let mut engine = engine_with_campaign(PropagationStrategy::BreadthFirst, 4, 4);
        let first = item("persist");
        engine
            .send_initial(
                &campaign("test"),
                first.clone(),
                cthulhu("archivist"),
                cthulhu("hermit"),
                1,
            )
            .unwrap();
        let encoded = serde_json::to_vec(&engine).unwrap();
        let restored: PropagationEngine = serde_json::from_slice(&encoded).unwrap();
        restored.validate_loaded_state(2).unwrap();
        let mut restored = restored;
        assert!(matches!(
            restored
                .send_initial(
                    &campaign("test"),
                    first,
                    cthulhu("archivist"),
                    cthulhu("hermit"),
                    1
                )
                .unwrap(),
            DeliveryResult::ReplaySuppressed { .. }
        ));
        assert_eq!(restored.items.len(), 1);
    }

    #[test]
    fn hostile_persisted_propagation_state_is_rejected() {
        let mut engine = engine_with_campaign(PropagationStrategy::BreadthFirst, 4, 4);
        engine
            .send_initial(
                &campaign("test"),
                item("tampered"),
                cthulhu("archivist"),
                cthulhu("hermit"),
                1,
            )
            .unwrap();
        engine.rate_records[0].sender = cthulhu("oracle");
        assert!(matches!(
            engine.validate_loaded_state(2),
            Err(PropagationError::CorruptState("rate record"))
        ));
    }

    #[test]
    fn restored_state_reapplies_parent_strategy_and_local_policy() {
        let mut parent_tamper = engine_with_campaign(PropagationStrategy::BreadthFirst, 4, 4);
        let parent_id = item("restore-parent");
        deliver_and_accept(
            &mut parent_tamper,
            None,
            parent_id.clone(),
            cthulhu("archivist"),
            cthulhu("hermit"),
            1,
        );
        parent_tamper
            .forward(
                &parent_id,
                item("restore-child"),
                cthulhu("hermit"),
                cthulhu("oracle"),
                3,
            )
            .unwrap();
        parent_tamper.items.get_mut(&parent_id).unwrap().status = ItemStatus::Rejected;
        assert!(matches!(
            parent_tamper.validate_loaded_state(4),
            Err(PropagationError::ParentNotAccepted)
        ));

        let mut strategy_tamper = engine_with_campaign(
            PropagationStrategy::CapabilityTargeted {
                capability: "archive".into(),
            },
            4,
            4,
        );
        strategy_tamper
            .send_initial(
                &campaign("test"),
                item("restore-strategy"),
                cthulhu("archivist"),
                cthulhu("hermit"),
                1,
            )
            .unwrap();
        let mut changed_profile = profile("hermit");
        changed_profile.capability_tags = vec!["commerce".into()];
        assert!(matches!(
            strategy_tamper.register_candidate(changed_profile.clone()),
            Err(PropagationError::CandidateProfileInUse)
        ));
        strategy_tamper
            .candidate_profiles
            .get_mut(&cthulhu("hermit"))
            .unwrap()
            .capability_tags = changed_profile.capability_tags;
        assert!(matches!(
            strategy_tamper.validate_loaded_state(2),
            Err(PropagationError::ForgedProvenance)
        ));

        let mut local_policy = engine_with_campaign(PropagationStrategy::BreadthFirst, 4, 4);
        local_policy
            .send_initial(
                &campaign("test"),
                item("restore-optout"),
                cthulhu("archivist"),
                cthulhu("hermit"),
                1,
            )
            .unwrap();
        local_policy.set_opt_out(cthulhu("hermit"), true).unwrap();
        local_policy.validate_loaded_state(2).unwrap();

        let forged_id = item("restore-optout");
        local_policy
            .items
            .get_mut(&forged_id)
            .unwrap()
            .provenance
            .hops
            .last_mut()
            .unwrap()
            .local_policy_generation = 1;
        let campaign_record = local_policy.campaigns.get(&campaign("test")).unwrap();
        let forged_hash = provenance_hash(
            &campaign_record.id,
            &campaign_record.payload_hash,
            campaign_record.policy.version,
            &local_policy.items.get(&forged_id).unwrap().provenance.hops,
        )
        .unwrap();
        local_policy
            .items
            .get_mut(&forged_id)
            .unwrap()
            .provenance
            .chain_hash = forged_hash;
        assert!(matches!(
            local_policy.validate_loaded_state(2),
            Err(PropagationError::OptedOut)
        ));

        struct RejectRestoredBranch;
        impl PropagationPolicyValidator for RejectRestoredBranch {
            fn validate_hop(
                &self,
                _campaign: &Campaign,
                _parent: Option<&PropagationItem>,
                _sender: &CthulhuId,
                _recipient: &CthulhuId,
                _now: i64,
            ) -> Result<String, PropagationError> {
                Err(PropagationError::StrategyRejected)
            }
        }
        let mut local_policy = engine_with_campaign(PropagationStrategy::BreadthFirst, 4, 4);
        local_policy
            .send_initial(
                &campaign("test"),
                item("restore-validator"),
                cthulhu("archivist"),
                cthulhu("hermit"),
                1,
            )
            .unwrap();
        assert!(matches!(
            local_policy.validate_loaded_state_with_policy(2, &RejectRestoredBranch),
            Err(PropagationError::StrategyRejected)
        ));
    }

    #[test]
    fn hostile_ids_payloads_and_expiry_are_rejected() {
        assert!(CampaignId::parse("propagation_../../secret").is_err());
        assert!(CampaignId::parse("propagation_UPPER").is_err());
        let arbitrary = serde_json::from_str::<PropagationPayload>(
            r#"{"type":"private_conversation","text":"secret"}"#,
        );
        assert!(arbitrary.is_err());
        let mut engine = PropagationEngine::default();
        assert!(matches!(
            engine.create_campaign(
                campaign("expired"),
                council(),
                cthulhu("archivist"),
                invitation(),
                PropagationStrategy::BreadthFirst,
                policy(4, 4),
                100,
                99,
            ),
            Err(PropagationError::InvalidExpiry)
        ));
    }
}
