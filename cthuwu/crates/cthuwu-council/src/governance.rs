//! Deterministic, transport-independent Council governance.
//!
//! Governance decides what the Council has approved. Execution is a separate
//! local decision: a ratified action never bypasses an operator's policy.

use std::collections::{HashMap, HashSet};

use cthuwu_protocol::{CouncilId, CthulhuId, ProposalId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MAX_MEMBERS: usize = 4_096;
pub const MAX_MEMBERSHIP_SNAPSHOTS: usize = 16_384;
pub const MAX_OPEN_PROPOSALS: usize = 1_024;
pub const MAX_PROPOSALS: usize = 16_384;
pub const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_SHORT_TEXT_BYTES: usize = 512;
pub const MAX_LIST_ITEMS: usize = 128;
pub const MAX_ARGUMENTS: usize = 512;
pub const MAX_AMENDMENTS: usize = 256;
pub const MAX_PROPOSAL_LIFETIME_SECONDS: i64 = 31 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceDocument {
    Constitution(Constitution),
    Agenda(Agenda),
    Strategy(Strategy),
    Action(ActionDocument),
}

impl GovernanceDocument {
    pub fn kind(&self) -> DocumentKind {
        match self {
            Self::Constitution(_) => DocumentKind::Constitution,
            Self::Agenda(_) => DocumentKind::Agenda,
            Self::Strategy(_) => DocumentKind::Strategy,
            Self::Action(_) => DocumentKind::Action,
        }
    }

    pub fn validate(&self) -> Result<(), GovernanceError> {
        match self {
            Self::Constitution(value) => value.validate(),
            Self::Agenda(value) => value.validate(),
            Self::Strategy(value) => value.validate(),
            Self::Action(value) => value.validate(),
        }?;
        let encoded = serde_json::to_vec(self).map_err(GovernanceError::Serialization)?;
        if encoded.len() > MAX_DOCUMENT_BYTES {
            return Err(GovernanceError::LimitExceeded("document bytes"));
        }
        Ok(())
    }

    pub fn content_hash(&self) -> Result<String, GovernanceError> {
        self.validate()?;
        let encoded = serde_json::to_vec(self).map_err(GovernanceError::Serialization)?;
        Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
    }

    fn parent_hash(&self) -> Option<&str> {
        match self {
            Self::Constitution(value) => value.parent_hash.as_deref(),
            Self::Agenda(value) => value.parent_hash.as_deref(),
            Self::Strategy(value) => Some(value.agenda_hash.as_str()),
            Self::Action(value) => value.agenda_hash.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Constitution,
    Agenda,
    Strategy,
    Action,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constitution {
    pub version: u64,
    pub parent_hash: Option<String>,
    pub principles: Vec<String>,
    pub security_invariants: Vec<String>,
}

impl Constitution {
    fn validate(&self) -> Result<(), GovernanceError> {
        if self.version == 0 {
            return Err(GovernanceError::InvalidDocument(
                "constitution version must be positive",
            ));
        }
        validate_hash_opt(self.parent_hash.as_deref())?;
        validate_text_list(&self.principles, "principles", false)?;
        validate_text_list(&self.security_invariants, "security invariants", false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agenda {
    pub version: u64,
    pub parent_hash: Option<String>,
    pub summary: String,
    pub goals: Vec<String>,
}

impl Agenda {
    fn validate(&self) -> Result<(), GovernanceError> {
        if self.version == 0 {
            return Err(GovernanceError::InvalidDocument(
                "agenda version must be positive",
            ));
        }
        validate_hash_opt(self.parent_hash.as_deref())?;
        validate_text(&self.summary, MAX_TEXT_BYTES, "agenda summary", false)?;
        validate_text_list(&self.goals, "agenda goals", false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Strategy {
    pub agenda_hash: String,
    pub title: String,
    pub approach: String,
    pub tradeoffs: Vec<String>,
}

impl Strategy {
    fn validate(&self) -> Result<(), GovernanceError> {
        validate_hash(&self.agenda_hash)?;
        validate_text(&self.title, MAX_SHORT_TEXT_BYTES, "strategy title", true)?;
        validate_text(&self.approach, MAX_TEXT_BYTES, "strategy approach", false)?;
        validate_text_list(&self.tradeoffs, "strategy tradeoffs", true)
    }
}

/// Harmless, closed action set. Deliberately contains no generic command,
/// script, URL fetch, filesystem path, or arbitrary tool invocation variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GovernanceAction {
    CapabilityRefresh { target: Option<CthulhuId> },
    ProtocolSelfTest { suite: SelfTestSuite },
    LocalResourceSummary { fields: Vec<ResourceSummaryField> },
    RoutingScenarioEvaluation { scenario_id: String },
}

impl GovernanceAction {
    fn validate(&self) -> Result<(), GovernanceError> {
        match self {
            Self::CapabilityRefresh { .. } | Self::ProtocolSelfTest { .. } => Ok(()),
            Self::LocalResourceSummary { fields } => {
                if fields.is_empty() || fields.len() > 16 {
                    return Err(GovernanceError::LimitExceeded("resource summary fields"));
                }
                let unique: HashSet<_> = fields.iter().collect();
                if unique.len() != fields.len() {
                    return Err(GovernanceError::InvalidDocument(
                        "duplicate resource summary field",
                    ));
                }
                Ok(())
            }
            Self::RoutingScenarioEvaluation { scenario_id } => {
                validate_token(scenario_id, "scenario id")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfTestSuite {
    EnvelopeValidation,
    RoutingDeterminism,
    LeaseGeneration,
    PersistenceReplay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceSummaryField {
    CapabilityClasses,
    AvailableCapacity,
    MemoryModes,
    PrivacyProperties,
    ProtocolVersions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDocument {
    pub agenda_hash: Option<String>,
    pub action: GovernanceAction,
    pub rationale: String,
}

impl ActionDocument {
    fn validate(&self) -> Result<(), GovernanceError> {
        validate_hash_opt(self.agenda_hash.as_deref())?;
        self.action.validate()?;
        validate_text(&self.rationale, MAX_TEXT_BYTES, "action rationale", false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Open,
    Ratified,
    Rejected,
    Expired,
    ParentConflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    Support,
    Oppose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoteChoice {
    Support,
    Oppose,
    Abstain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Argument {
    pub author: CthulhuId,
    pub position: Position,
    pub text: String,
    pub submitted_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmendmentSuggestion {
    pub author: CthulhuId,
    pub text: String,
    pub submitted_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoteRecord {
    pub choice: VoteChoice,
    pub cast_at: i64,
    pub revision: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: ProposalId,
    pub council_id: CouncilId,
    pub proposer: CthulhuId,
    pub document: GovernanceDocument,
    pub document_hash: String,
    pub created_at: i64,
    pub deadline: i64,
    pub status: ProposalStatus,
    /// Append-only membership revision whose exact member set was eligible
    /// when this proposal opened.
    pub membership_revision: u64,
    /// Content hash of `membership_revision`. This prevents a proposal from
    /// supplying an independent voter set during persisted-state replay.
    pub membership_hash: String,
    /// Index of the hash-chained proposal-to-membership binding.
    pub membership_binding_index: u64,
    pub eligible_voters: HashSet<CthulhuId>,
    pub arguments: Vec<Argument>,
    pub amendments: Vec<AmendmentSuggestion>,
    pub votes: HashMap<CthulhuId, VoteRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernanceRules {
    /// Fraction of all eligible members that must participate, in basis points.
    pub quorum_bps: u16,
    /// Support / non-abstaining votes needed for normal documents.
    pub approval_bps: u16,
    /// Stricter support threshold for Constitution changes.
    pub constitution_approval_bps: u16,
    pub max_proposal_lifetime_seconds: i64,
}

impl Default for GovernanceRules {
    fn default() -> Self {
        Self {
            quorum_bps: 5_000,
            approval_bps: 5_001,
            constitution_approval_bps: 6_667,
            max_proposal_lifetime_seconds: 7 * 24 * 60 * 60,
        }
    }
}

impl GovernanceRules {
    pub fn validate(self) -> Result<Self, GovernanceError> {
        if self.quorum_bps == 0
            || self.quorum_bps > 10_000
            || self.approval_bps == 0
            || self.approval_bps > 10_000
            || self.constitution_approval_bps <= self.approval_bps
            || self.constitution_approval_bps > 10_000
        {
            return Err(GovernanceError::InvalidRules);
        }
        if self.max_proposal_lifetime_seconds <= 0
            || self.max_proposal_lifetime_seconds > MAX_PROPOSAL_LIFETIME_SECONDS
        {
            return Err(GovernanceError::InvalidRules);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentHead {
    pub version: u64,
    pub hash: String,
    pub proposal_id: ProposalId,
}

/// Canonical, append-only basis for proposal eligibility.
///
/// Snapshots are hash chained and serialized in sorted member order. The
/// chain is deliberately a self-consistency mechanism, not a production
/// signature: the persistence layer still has to protect the state file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MembershipSnapshot {
    revision: u64,
    parent_hash: Option<String>,
    members: Vec<CthulhuId>,
    hash: String,
}

impl MembershipSnapshot {
    fn new(
        council_id: &CouncilId,
        revision: u64,
        parent_hash: Option<String>,
        members: &HashSet<CthulhuId>,
    ) -> Self {
        let mut members: Vec<_> = members.iter().cloned().collect();
        members.sort();
        let hash = membership_snapshot_hash(council_id, revision, parent_hash.as_deref(), &members);
        Self {
            revision,
            parent_hash,
            members,
            hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProposalMembershipBinding {
    index: u64,
    parent_hash: Option<String>,
    proposal_id: ProposalId,
    membership_revision: u64,
    membership_hash: String,
    hash: String,
}

impl ProposalMembershipBinding {
    fn new(
        council_id: &CouncilId,
        index: u64,
        parent_hash: Option<String>,
        proposal_id: ProposalId,
        membership: &MembershipSnapshot,
    ) -> Self {
        let hash = proposal_membership_binding_hash(
            council_id,
            index,
            parent_hash.as_deref(),
            &proposal_id,
            membership.revision,
            &membership.hash,
        );
        Self {
            index,
            parent_hash,
            proposal_id,
            membership_revision: membership.revision,
            membership_hash: membership.hash.clone(),
            hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernanceEngine {
    council_id: CouncilId,
    rules: GovernanceRules,
    members: HashSet<CthulhuId>,
    membership_history: Vec<MembershipSnapshot>,
    proposal_membership_log: Vec<ProposalMembershipBinding>,
    proposals: HashMap<ProposalId, Proposal>,
    constitution: Option<DocumentHead>,
    agenda: Option<DocumentHead>,
}

impl GovernanceEngine {
    pub fn new(
        council_id: CouncilId,
        rules: GovernanceRules,
        members: impl IntoIterator<Item = CthulhuId>,
    ) -> Result<Self, GovernanceError> {
        let members: HashSet<_> = members.into_iter().collect();
        if members.is_empty() || members.len() > MAX_MEMBERS {
            return Err(GovernanceError::LimitExceeded("Council members"));
        }
        let genesis = MembershipSnapshot::new(&council_id, 0, None, &members);
        Ok(Self {
            council_id,
            rules: rules.validate()?,
            members,
            membership_history: vec![genesis],
            proposal_membership_log: Vec::new(),
            proposals: HashMap::new(),
            constitution: None,
            agenda: None,
        })
    }

    pub fn rules(&self) -> GovernanceRules {
        self.rules
    }

    pub fn constitution(&self) -> Option<&DocumentHead> {
        self.constitution.as_ref()
    }

    pub fn agenda(&self) -> Option<&DocumentHead> {
        self.agenda.as_ref()
    }

    pub fn proposal(&self, id: &ProposalId) -> Option<&Proposal> {
        self.proposals.get(id)
    }

    pub fn add_member(&mut self, member: CthulhuId) -> Result<bool, GovernanceError> {
        if self.members.contains(&member) {
            return Ok(false);
        }
        if self.members.len() >= MAX_MEMBERS {
            return Err(GovernanceError::LimitExceeded("Council members"));
        }
        let mut members = self.members.clone();
        members.insert(member);
        self.append_membership_snapshot(&members)?;
        self.members = members;
        Ok(true)
    }

    pub fn remove_member(&mut self, member: &CthulhuId) -> bool {
        if !self.members.contains(member)
            || self.members.len() == 1
            || self.membership_history.len() >= MAX_MEMBERSHIP_SNAPSHOTS
        {
            return false;
        }
        let mut members = self.members.clone();
        members.remove(member);
        if self.append_membership_snapshot(&members).is_err() {
            return false;
        }
        self.members = members;
        true
    }

    fn append_membership_snapshot(
        &mut self,
        members: &HashSet<CthulhuId>,
    ) -> Result<(), GovernanceError> {
        if members.is_empty() || members.len() > MAX_MEMBERS {
            return Err(GovernanceError::LimitExceeded("Council members"));
        }
        if self.membership_history.len() >= MAX_MEMBERSHIP_SNAPSHOTS {
            return Err(GovernanceError::LimitExceeded("membership snapshots"));
        }
        let parent = self
            .membership_history
            .last()
            .ok_or(GovernanceError::CorruptState("membership history"))?;
        let revision = parent
            .revision
            .checked_add(1)
            .ok_or(GovernanceError::LimitExceeded("membership revisions"))?;
        self.membership_history.push(MembershipSnapshot::new(
            &self.council_id,
            revision,
            Some(parent.hash.clone()),
            members,
        ));
        Ok(())
    }

    pub fn submit(
        &mut self,
        id: ProposalId,
        proposer: CthulhuId,
        document: GovernanceDocument,
        created_at: i64,
        deadline: i64,
    ) -> Result<&Proposal, GovernanceError> {
        if !self.members.contains(&proposer) {
            return Err(GovernanceError::NotMember);
        }
        if self.proposals.contains_key(&id) {
            return Err(GovernanceError::DuplicateProposal);
        }
        if self.proposals.len() >= MAX_PROPOSALS {
            return Err(GovernanceError::LimitExceeded("proposals"));
        }
        if self
            .proposals
            .values()
            .filter(|proposal| proposal.status == ProposalStatus::Open)
            .count()
            >= MAX_OPEN_PROPOSALS
        {
            return Err(GovernanceError::LimitExceeded("open proposals"));
        }
        if created_at < 0
            || deadline <= created_at
            || deadline - created_at > self.rules.max_proposal_lifetime_seconds
        {
            return Err(GovernanceError::InvalidDeadline);
        }
        document.validate()?;
        self.validate_document_lineage(&document)?;
        let document_hash = document.content_hash()?;
        let membership = self
            .membership_history
            .last()
            .ok_or(GovernanceError::CorruptState("membership history"))?;
        if self.proposal_membership_log.len() != self.proposals.len()
            || self.proposal_membership_log.len() >= MAX_PROPOSALS
        {
            return Err(GovernanceError::CorruptState("proposal membership log"));
        }
        let binding_index = u64::try_from(self.proposal_membership_log.len())
            .map_err(|_| GovernanceError::LimitExceeded("proposal membership bindings"))?;
        let binding_parent = self
            .proposal_membership_log
            .last()
            .map(|binding| binding.hash.clone());
        let binding = ProposalMembershipBinding::new(
            &self.council_id,
            binding_index,
            binding_parent,
            id.clone(),
            membership,
        );
        let proposal = Proposal {
            id: id.clone(),
            council_id: self.council_id.clone(),
            proposer,
            document,
            document_hash,
            created_at,
            deadline,
            status: ProposalStatus::Open,
            membership_revision: membership.revision,
            membership_hash: membership.hash.clone(),
            membership_binding_index: binding_index,
            eligible_voters: self.members.clone(),
            arguments: Vec::new(),
            amendments: Vec::new(),
            votes: HashMap::new(),
        };
        self.proposal_membership_log.push(binding);
        self.proposals.insert(id.clone(), proposal);
        self.proposals
            .get(&id)
            .ok_or(GovernanceError::ProposalNotFound)
    }

    pub fn add_argument(
        &mut self,
        proposal_id: &ProposalId,
        author: CthulhuId,
        position: Position,
        text: String,
        now: i64,
    ) -> Result<(), GovernanceError> {
        validate_text(&text, MAX_TEXT_BYTES, "argument", false)?;
        let proposal = self.open_proposal_mut(proposal_id, &author, now)?;
        if proposal.arguments.len() >= MAX_ARGUMENTS {
            return Err(GovernanceError::LimitExceeded("arguments"));
        }
        proposal.arguments.push(Argument {
            author,
            position,
            text,
            submitted_at: now,
        });
        Ok(())
    }

    pub fn suggest_amendment(
        &mut self,
        proposal_id: &ProposalId,
        author: CthulhuId,
        text: String,
        now: i64,
    ) -> Result<(), GovernanceError> {
        validate_text(&text, MAX_TEXT_BYTES, "amendment", false)?;
        let proposal = self.open_proposal_mut(proposal_id, &author, now)?;
        if proposal.amendments.len() >= MAX_AMENDMENTS {
            return Err(GovernanceError::LimitExceeded("amendments"));
        }
        proposal.amendments.push(AmendmentSuggestion {
            author,
            text,
            submitted_at: now,
        });
        Ok(())
    }

    pub fn cast_vote(
        &mut self,
        proposal_id: &ProposalId,
        voter: CthulhuId,
        choice: VoteChoice,
        now: i64,
    ) -> Result<VoteReceipt, GovernanceError> {
        let proposal = self.open_proposal_mut(proposal_id, &voter, now)?;
        let previous = proposal.votes.get(&voter);
        let revision = previous
            .map(|record| {
                record
                    .revision
                    .checked_add(1)
                    .ok_or(GovernanceError::LimitExceeded("vote revisions"))
            })
            .transpose()?
            .unwrap_or(0);
        let replaced = previous.is_some();
        proposal.votes.insert(
            voter,
            VoteRecord {
                choice,
                cast_at: now,
                revision,
            },
        );
        Ok(VoteReceipt { replaced, revision })
    }

    /// Finalization is only allowed at or after the deadline, preserving every
    /// member's right to replace a vote until voting closes.
    pub fn finalize(
        &mut self,
        proposal_id: &ProposalId,
        now: i64,
    ) -> Result<Finalization, GovernanceError> {
        let proposal = self
            .proposals
            .get(proposal_id)
            .ok_or(GovernanceError::ProposalNotFound)?;
        if proposal.status != ProposalStatus::Open {
            return Err(GovernanceError::ProposalClosed);
        }
        if now < proposal.deadline {
            return Err(GovernanceError::VotingStillOpen);
        }
        if self.validate_membership_history().is_err()
            || self.validate_proposal_membership_log().is_err()
            || !self.proposal_membership_is_valid(proposal)
            || !proposal.votes.iter().all(|(voter, vote)| {
                proposal.eligible_voters.contains(voter)
                    && vote.cast_at >= proposal.created_at
                    && vote.cast_at < proposal.deadline
            })
        {
            return Err(GovernanceError::InvalidRatificationEvidence);
        }

        let tally = Tally::from(proposal);
        let parent_conflict = self.has_parent_conflict(&proposal.document);
        let threshold = if proposal.document.kind() == DocumentKind::Constitution {
            self.rules.constitution_approval_bps
        } else {
            self.rules.approval_bps
        };
        let quorum_needed = ceil_bps(proposal.eligible_voters.len(), self.rules.quorum_bps);
        let status = if parent_conflict {
            ProposalStatus::ParentConflict
        } else if tally.participating < quorum_needed {
            ProposalStatus::Expired
        } else if tally.non_abstaining == 0 {
            ProposalStatus::Rejected
        } else if (tally.support as u128) * 10_000
            >= (tally.non_abstaining as u128) * (threshold as u128)
        {
            ProposalStatus::Ratified
        } else {
            ProposalStatus::Rejected
        };

        let document = proposal.document.clone();
        let document_hash = proposal.document_hash.clone();
        if status == ProposalStatus::Ratified {
            match &document {
                GovernanceDocument::Constitution(value) => {
                    self.constitution = Some(DocumentHead {
                        version: value.version,
                        hash: document_hash,
                        proposal_id: proposal_id.clone(),
                    });
                }
                GovernanceDocument::Agenda(value) => {
                    self.agenda = Some(DocumentHead {
                        version: value.version,
                        hash: document_hash,
                        proposal_id: proposal_id.clone(),
                    });
                }
                GovernanceDocument::Strategy(_) | GovernanceDocument::Action(_) => {}
            }
        }
        self.proposals
            .get_mut(proposal_id)
            .ok_or(GovernanceError::ProposalNotFound)?
            .status = status;
        Ok(Finalization {
            status,
            tally,
            quorum_needed,
            approval_threshold_bps: threshold,
        })
    }

    pub fn competing_proposals(
        &self,
        proposal_id: &ProposalId,
    ) -> Result<Vec<ProposalId>, GovernanceError> {
        let proposal = self
            .proposals
            .get(proposal_id)
            .ok_or(GovernanceError::ProposalNotFound)?;
        let kind = proposal.document.kind();
        let parent = proposal.document.parent_hash();
        let mut competing: Vec<_> = self
            .proposals
            .values()
            .filter(|other| {
                other.id != proposal.id
                    && other.status == ProposalStatus::Open
                    && other.document.kind() == kind
                    && other.document.parent_hash() == parent
            })
            .map(|other| other.id.clone())
            .collect();
        competing.sort();
        Ok(competing)
    }

    pub fn authorize_action<P: LocalOperatorPolicy>(
        &self,
        proposal_id: &ProposalId,
        policy: &P,
    ) -> Result<ActionAuthorization, GovernanceError> {
        let proposal = self
            .proposals
            .get(proposal_id)
            .ok_or(GovernanceError::ProposalNotFound)?;
        if proposal.status != ProposalStatus::Ratified {
            return Err(GovernanceError::ActionNotRatified);
        }
        if self.validate_membership_history().is_err()
            || self.validate_proposal_membership_log().is_err()
            || !self.has_valid_ratification_evidence(proposal)
        {
            return Err(GovernanceError::InvalidRatificationEvidence);
        }
        let GovernanceDocument::Action(action) = &proposal.document else {
            return Err(GovernanceError::NotAnAction);
        };
        match policy.evaluate(&action.action) {
            LocalPolicyDecision::Allow => Ok(ActionAuthorization {
                authorized: true,
                reason: "ratified by the Council and allowed by local operator policy".into(),
            }),
            LocalPolicyDecision::Deny { reason } => {
                validate_text(&reason, MAX_SHORT_TEXT_BYTES, "local policy reason", false)?;
                Ok(ActionAuthorization {
                    authorized: false,
                    reason,
                })
            }
        }
    }

    /// Revalidates every invariant after deserializing operator-controlled
    /// state. Call this before using a restored engine.
    pub fn validate_loaded_state(&self, now: i64) -> Result<(), GovernanceError> {
        if now < 0
            || self.members.is_empty()
            || self.members.len() > MAX_MEMBERS
            || self.membership_history.is_empty()
            || self.membership_history.len() > MAX_MEMBERSHIP_SNAPSHOTS
            || self.proposal_membership_log.len() > MAX_PROPOSALS
            || self.proposals.len() > MAX_PROPOSALS
        {
            return Err(GovernanceError::CorruptState("top-level bounds"));
        }
        self.rules.validate()?;
        self.validate_membership_history()?;
        self.validate_proposal_membership_log()?;
        let open = self
            .proposals
            .values()
            .filter(|proposal| proposal.status == ProposalStatus::Open)
            .count();
        if open > MAX_OPEN_PROPOSALS {
            return Err(GovernanceError::CorruptState("open proposal bound"));
        }
        for (key, proposal) in &self.proposals {
            if key != &proposal.id
                || proposal.council_id != self.council_id
                || proposal.eligible_voters.is_empty()
                || proposal.eligible_voters.len() > MAX_MEMBERS
                || !proposal.eligible_voters.contains(&proposal.proposer)
                || proposal.created_at < 0
                || proposal.deadline <= proposal.created_at
                || proposal.deadline - proposal.created_at
                    > self.rules.max_proposal_lifetime_seconds
                || proposal.arguments.len() > MAX_ARGUMENTS
                || proposal.amendments.len() > MAX_AMENDMENTS
                || proposal.votes.len() > proposal.eligible_voters.len()
                || (proposal.status != ProposalStatus::Open && proposal.deadline > now)
            {
                return Err(GovernanceError::CorruptState("proposal metadata"));
            }
            if !self.proposal_membership_is_valid(proposal) {
                return Err(GovernanceError::CorruptState("proposal membership basis"));
            }
            proposal.document.validate()?;
            if proposal.document.content_hash()? != proposal.document_hash {
                return Err(GovernanceError::CorruptState("document hash"));
            }
            if proposal.status == ProposalStatus::Ratified
                && !self.has_valid_ratification_evidence(proposal)
            {
                return Err(GovernanceError::CorruptState("ratification evidence"));
            }
            for argument in &proposal.arguments {
                if !proposal.eligible_voters.contains(&argument.author)
                    || argument.submitted_at < proposal.created_at
                    || argument.submitted_at >= proposal.deadline
                {
                    return Err(GovernanceError::CorruptState("argument metadata"));
                }
                validate_text(&argument.text, MAX_TEXT_BYTES, "argument", false)?;
            }
            for amendment in &proposal.amendments {
                if !proposal.eligible_voters.contains(&amendment.author)
                    || amendment.submitted_at < proposal.created_at
                    || amendment.submitted_at >= proposal.deadline
                {
                    return Err(GovernanceError::CorruptState("amendment metadata"));
                }
                validate_text(&amendment.text, MAX_TEXT_BYTES, "amendment", false)?;
            }
            for (voter, vote) in &proposal.votes {
                if !proposal.eligible_voters.contains(voter)
                    || vote.cast_at < proposal.created_at
                    || vote.cast_at >= proposal.deadline
                {
                    return Err(GovernanceError::CorruptState("vote metadata"));
                }
            }
        }
        self.validate_loaded_head(self.constitution.as_ref(), DocumentKind::Constitution)?;
        self.validate_loaded_head(self.agenda.as_ref(), DocumentKind::Agenda)?;
        Ok(())
    }

    fn validate_membership_history(&self) -> Result<(), GovernanceError> {
        let mut expected_parent: Option<&str> = None;
        for (index, snapshot) in self.membership_history.iter().enumerate() {
            let revision = u64::try_from(index)
                .map_err(|_| GovernanceError::CorruptState("membership history"))?;
            if snapshot.revision != revision
                || snapshot.parent_hash.as_deref() != expected_parent
                || snapshot.members.is_empty()
                || snapshot.members.len() > MAX_MEMBERS
                || !snapshot.members.windows(2).all(|pair| pair[0] < pair[1])
            {
                return Err(GovernanceError::CorruptState("membership history"));
            }
            validate_hash(&snapshot.hash)?;
            validate_hash_opt(snapshot.parent_hash.as_deref())?;
            let expected_hash = membership_snapshot_hash(
                &self.council_id,
                snapshot.revision,
                snapshot.parent_hash.as_deref(),
                &snapshot.members,
            );
            if snapshot.hash != expected_hash {
                return Err(GovernanceError::CorruptState("membership history hash"));
            }
            expected_parent = Some(snapshot.hash.as_str());
        }

        let current = self
            .membership_history
            .last()
            .ok_or(GovernanceError::CorruptState("membership history"))?;
        if current.members.len() != self.members.len()
            || !current
                .members
                .iter()
                .all(|member| self.members.contains(member))
        {
            return Err(GovernanceError::CorruptState("current membership"));
        }
        Ok(())
    }

    fn validate_proposal_membership_log(&self) -> Result<(), GovernanceError> {
        if self.proposal_membership_log.len() != self.proposals.len() {
            return Err(GovernanceError::CorruptState("proposal membership log"));
        }
        let mut expected_parent: Option<&str> = None;
        let mut proposal_ids = HashSet::with_capacity(self.proposal_membership_log.len());
        for (index, binding) in self.proposal_membership_log.iter().enumerate() {
            let expected_index = u64::try_from(index)
                .map_err(|_| GovernanceError::CorruptState("proposal membership log"))?;
            let Some(membership) = usize::try_from(binding.membership_revision)
                .ok()
                .and_then(|revision| self.membership_history.get(revision))
            else {
                return Err(GovernanceError::CorruptState("proposal membership binding"));
            };
            if binding.index != expected_index
                || binding.parent_hash.as_deref() != expected_parent
                || binding.membership_hash != membership.hash
                || !proposal_ids.insert(binding.proposal_id.clone())
            {
                return Err(GovernanceError::CorruptState("proposal membership binding"));
            }
            let Some(proposal) = self.proposals.get(&binding.proposal_id) else {
                return Err(GovernanceError::CorruptState("proposal membership binding"));
            };
            if proposal.membership_binding_index != binding.index
                || proposal.membership_revision != binding.membership_revision
                || proposal.membership_hash != binding.membership_hash
            {
                return Err(GovernanceError::CorruptState("proposal membership binding"));
            }
            let expected_hash = proposal_membership_binding_hash(
                &self.council_id,
                binding.index,
                binding.parent_hash.as_deref(),
                &binding.proposal_id,
                binding.membership_revision,
                &binding.membership_hash,
            );
            if binding.hash != expected_hash {
                return Err(GovernanceError::CorruptState(
                    "proposal membership binding hash",
                ));
            }
            expected_parent = Some(binding.hash.as_str());
        }
        Ok(())
    }

    fn validate_loaded_head(
        &self,
        head: Option<&DocumentHead>,
        kind: DocumentKind,
    ) -> Result<(), GovernanceError> {
        let Some(head) = head else {
            return Ok(());
        };
        validate_hash(&head.hash)?;
        let proposal = self
            .proposals
            .get(&head.proposal_id)
            .ok_or(GovernanceError::CorruptState("document head proposal"))?;
        let version = match &proposal.document {
            GovernanceDocument::Constitution(document) if kind == DocumentKind::Constitution => {
                document.version
            }
            GovernanceDocument::Agenda(document) if kind == DocumentKind::Agenda => {
                document.version
            }
            _ => return Err(GovernanceError::CorruptState("document head kind")),
        };
        if proposal.status != ProposalStatus::Ratified
            || proposal.document_hash != head.hash
            || version != head.version
        {
            return Err(GovernanceError::CorruptState("document head mismatch"));
        }
        Ok(())
    }

    fn has_valid_ratification_evidence(&self, proposal: &Proposal) -> bool {
        if self.rules.validate().is_err()
            || !self.proposal_membership_is_valid(proposal)
            || match proposal.document.content_hash() {
                Ok(hash) => hash != proposal.document_hash,
                Err(_) => true,
            }
            || proposal.votes.iter().any(|(voter, vote)| {
                !proposal.eligible_voters.contains(voter)
                    || vote.cast_at < proposal.created_at
                    || vote.cast_at >= proposal.deadline
            })
        {
            return false;
        }
        let tally = Tally::from(proposal);
        let quorum_needed = ceil_bps(proposal.eligible_voters.len(), self.rules.quorum_bps);
        let threshold = if proposal.document.kind() == DocumentKind::Constitution {
            self.rules.constitution_approval_bps
        } else {
            self.rules.approval_bps
        };
        tally.participating >= quorum_needed
            && tally.non_abstaining > 0
            && (tally.support as u128) * 10_000
                >= (tally.non_abstaining as u128) * (threshold as u128)
    }

    fn proposal_membership_is_valid(&self, proposal: &Proposal) -> bool {
        let Ok(binding_index) = usize::try_from(proposal.membership_binding_index) else {
            return false;
        };
        let Some(binding) = self.proposal_membership_log.get(binding_index) else {
            return false;
        };
        if binding.index != proposal.membership_binding_index
            || binding.proposal_id != proposal.id
            || binding.membership_revision != proposal.membership_revision
            || binding.membership_hash != proposal.membership_hash
        {
            return false;
        }
        let Ok(revision) = usize::try_from(binding.membership_revision) else {
            return false;
        };
        let Some(snapshot) = self.membership_history.get(revision) else {
            return false;
        };
        snapshot.revision == proposal.membership_revision
            && snapshot.hash == proposal.membership_hash
            && snapshot.members.len() == proposal.eligible_voters.len()
            && snapshot
                .members
                .iter()
                .all(|member| proposal.eligible_voters.contains(member))
    }

    fn open_proposal_mut(
        &mut self,
        proposal_id: &ProposalId,
        actor: &CthulhuId,
        now: i64,
    ) -> Result<&mut Proposal, GovernanceError> {
        let proposal = self
            .proposals
            .get_mut(proposal_id)
            .ok_or(GovernanceError::ProposalNotFound)?;
        if proposal.status != ProposalStatus::Open || now >= proposal.deadline {
            return Err(GovernanceError::ProposalClosed);
        }
        if now < proposal.created_at {
            return Err(GovernanceError::InvalidTimestamp);
        }
        if !proposal.eligible_voters.contains(actor) {
            return Err(GovernanceError::NotEligible);
        }
        Ok(proposal)
    }

    fn validate_document_lineage(
        &self,
        document: &GovernanceDocument,
    ) -> Result<(), GovernanceError> {
        match document {
            GovernanceDocument::Constitution(value) => validate_next_version_and_parent(
                self.constitution.as_ref(),
                value.version,
                value.parent_hash.as_deref(),
            ),
            GovernanceDocument::Agenda(value) => validate_next_version_and_parent(
                self.agenda.as_ref(),
                value.version,
                value.parent_hash.as_deref(),
            ),
            GovernanceDocument::Strategy(value) => {
                if self.agenda.as_ref().map(|head| head.hash.as_str())
                    != Some(value.agenda_hash.as_str())
                {
                    Err(GovernanceError::ParentMismatch)
                } else {
                    Ok(())
                }
            }
            GovernanceDocument::Action(value) => {
                if let Some(parent) = value.agenda_hash.as_deref()
                    && self.agenda.as_ref().map(|head| head.hash.as_str()) != Some(parent)
                {
                    return Err(GovernanceError::ParentMismatch);
                }
                Ok(())
            }
        }
    }

    fn has_parent_conflict(&self, document: &GovernanceDocument) -> bool {
        match document {
            GovernanceDocument::Constitution(value) => !lineage_matches(
                self.constitution.as_ref(),
                value.version,
                value.parent_hash.as_deref(),
            ),
            GovernanceDocument::Agenda(value) => !lineage_matches(
                self.agenda.as_ref(),
                value.version,
                value.parent_hash.as_deref(),
            ),
            GovernanceDocument::Strategy(value) => {
                self.agenda.as_ref().map(|head| head.hash.as_str())
                    != Some(value.agenda_hash.as_str())
            }
            GovernanceDocument::Action(value) => {
                value.agenda_hash.as_deref().is_some_and(|parent| {
                    self.agenda.as_ref().map(|head| head.hash.as_str()) != Some(parent)
                })
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoteReceipt {
    pub replaced: bool,
    pub revision: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tally {
    pub support: usize,
    pub oppose: usize,
    pub abstain: usize,
    pub participating: usize,
    pub non_abstaining: usize,
}

impl From<&Proposal> for Tally {
    fn from(proposal: &Proposal) -> Self {
        let mut tally = Self {
            support: 0,
            oppose: 0,
            abstain: 0,
            participating: 0,
            non_abstaining: 0,
        };
        for vote in proposal.votes.values() {
            match vote.choice {
                VoteChoice::Support => tally.support += 1,
                VoteChoice::Oppose => tally.oppose += 1,
                VoteChoice::Abstain => tally.abstain += 1,
            }
        }
        tally.participating = tally.support + tally.oppose + tally.abstain;
        tally.non_abstaining = tally.support + tally.oppose;
        tally
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Finalization {
    pub status: ProposalStatus,
    pub tally: Tally,
    pub quorum_needed: usize,
    pub approval_threshold_bps: u16,
}

pub trait LocalOperatorPolicy {
    fn evaluate(&self, action: &GovernanceAction) -> LocalPolicyDecision;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalPolicyDecision {
    Allow,
    Deny { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionAuthorization {
    pub authorized: bool,
    pub reason: String,
}

#[derive(Debug, Error)]
pub enum GovernanceError {
    #[error("governance rule set is invalid")]
    InvalidRules,
    #[error("invalid governance document: {0}")]
    InvalidDocument(&'static str),
    #[error("invalid or unsupported content hash")]
    InvalidHash,
    #[error("{0} exceeds its configured bound")]
    LimitExceeded(&'static str),
    #[error("invalid proposal deadline")]
    InvalidDeadline,
    #[error("invalid timestamp")]
    InvalidTimestamp,
    #[error("actor is not a Council member")]
    NotMember,
    #[error("actor was not eligible when the proposal opened")]
    NotEligible,
    #[error("proposal already exists")]
    DuplicateProposal,
    #[error("proposal was not found")]
    ProposalNotFound,
    #[error("proposal is closed")]
    ProposalClosed,
    #[error("voting is still open")]
    VotingStillOpen,
    #[error("document parent or version does not match the current head")]
    ParentMismatch,
    #[error("proposal is not a typed action")]
    NotAnAction,
    #[error("action has not been ratified")]
    ActionNotRatified,
    #[error("ratified proposal does not contain valid quorum and vote evidence")]
    InvalidRatificationEvidence,
    #[error("failed to serialize governance content: {0}")]
    Serialization(serde_json::Error),
    #[error("persisted governance state failed validation: {0}")]
    CorruptState(&'static str),
}

fn validate_next_version_and_parent(
    current: Option<&DocumentHead>,
    version: u64,
    parent: Option<&str>,
) -> Result<(), GovernanceError> {
    if lineage_matches(current, version, parent) {
        Ok(())
    } else {
        Err(GovernanceError::ParentMismatch)
    }
}

fn lineage_matches(current: Option<&DocumentHead>, version: u64, parent: Option<&str>) -> bool {
    match current {
        None => version == 1 && parent.is_none(),
        Some(head) => {
            head.version.checked_add(1) == Some(version) && parent == Some(head.hash.as_str())
        }
    }
}

fn ceil_bps(total: usize, bps: u16) -> usize {
    ((total as u128 * bps as u128).div_ceil(10_000)) as usize
}

fn membership_snapshot_hash(
    council_id: &CouncilId,
    revision: u64,
    parent_hash: Option<&str>,
    members: &[CthulhuId],
) -> String {
    fn update_field(hasher: &mut Sha256, value: &str) {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }

    let mut hasher = Sha256::new();
    hasher.update(b"cthuwu-governance-membership-v1\0");
    update_field(&mut hasher, council_id.as_str());
    hasher.update(revision.to_be_bytes());
    match parent_hash {
        Some(parent_hash) => {
            hasher.update([1]);
            update_field(&mut hasher, parent_hash);
        }
        None => hasher.update([0]),
    }
    hasher.update((members.len() as u64).to_be_bytes());
    for member in members {
        update_field(&mut hasher, member.as_str());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn proposal_membership_binding_hash(
    council_id: &CouncilId,
    index: u64,
    parent_hash: Option<&str>,
    proposal_id: &ProposalId,
    membership_revision: u64,
    membership_hash: &str,
) -> String {
    fn update_field(hasher: &mut Sha256, value: &str) {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }

    let mut hasher = Sha256::new();
    hasher.update(b"cthuwu-governance-proposal-membership-v1\0");
    update_field(&mut hasher, council_id.as_str());
    hasher.update(index.to_be_bytes());
    match parent_hash {
        Some(parent_hash) => {
            hasher.update([1]);
            update_field(&mut hasher, parent_hash);
        }
        None => hasher.update([0]),
    }
    update_field(&mut hasher, proposal_id.as_str());
    hasher.update(membership_revision.to_be_bytes());
    update_field(&mut hasher, membership_hash);
    format!("sha256:{:x}", hasher.finalize())
}

fn validate_hash_opt(value: Option<&str>) -> Result<(), GovernanceError> {
    if let Some(value) = value {
        validate_hash(value)?;
    }
    Ok(())
}

fn validate_hash(value: &str) -> Result<(), GovernanceError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(GovernanceError::InvalidHash);
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(GovernanceError::InvalidHash);
    }
    Ok(())
}

fn validate_text(
    value: &str,
    max: usize,
    field: &'static str,
    single_line: bool,
) -> Result<(), GovernanceError> {
    if value.trim().is_empty()
        || value.len() > max
        || value.contains('\0')
        || (single_line && value.contains(['\r', '\n']))
    {
        return Err(GovernanceError::InvalidDocument(field));
    }
    Ok(())
}

fn validate_text_list(
    values: &[String],
    field: &'static str,
    single_line: bool,
) -> Result<(), GovernanceError> {
    if values.is_empty() || values.len() > MAX_LIST_ITEMS {
        return Err(GovernanceError::LimitExceeded(field));
    }
    for value in values {
        validate_text(value, MAX_TEXT_BYTES, field, single_line)?;
    }
    Ok(())
}

fn validate_token(value: &str, field: &'static str) -> Result<(), GovernanceError> {
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(GovernanceError::InvalidDocument(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cthulhu(name: &str) -> CthulhuId {
        CthulhuId::parse(&format!("cthulhu_{name}")).unwrap()
    }

    fn proposal(name: &str) -> ProposalId {
        ProposalId::parse(&format!("proposal_{name}")).unwrap()
    }

    fn council() -> CouncilId {
        CouncilId::parse("council_test").unwrap()
    }

    fn engine() -> GovernanceEngine {
        GovernanceEngine::new(
            council(),
            GovernanceRules::default(),
            [cthulhu("archivist"), cthulhu("hermit"), cthulhu("merchant")],
        )
        .unwrap()
    }

    fn agenda(version: u64, parent_hash: Option<String>, summary: &str) -> GovernanceDocument {
        GovernanceDocument::Agenda(Agenda {
            version,
            parent_hash,
            summary: summary.into(),
            goals: vec!["help without leaking private conversations".into()],
        })
    }

    fn ratify(engine: &mut GovernanceEngine, id: &ProposalId, voters: &[CthulhuId]) {
        for voter in voters {
            engine
                .cast_vote(id, voter.clone(), VoteChoice::Support, 5)
                .unwrap();
        }
        assert_eq!(
            engine.finalize(id, 10).unwrap().status,
            ProposalStatus::Ratified
        );
    }

    #[test]
    fn vote_replacement_and_abstention_obey_one_cthulhu_one_vote() {
        let mut engine = engine();
        let id = proposal("agenda");
        engine
            .submit(
                id.clone(),
                cthulhu("archivist"),
                agenda(1, None, "first agenda"),
                0,
                10,
            )
            .unwrap();
        let first = engine
            .cast_vote(&id, cthulhu("archivist"), VoteChoice::Oppose, 2)
            .unwrap();
        let replacement = engine
            .cast_vote(&id, cthulhu("archivist"), VoteChoice::Support, 3)
            .unwrap();
        engine
            .cast_vote(&id, cthulhu("hermit"), VoteChoice::Abstain, 3)
            .unwrap();
        assert!(!first.replaced);
        assert!(replacement.replaced);
        assert_eq!(replacement.revision, 1);
        let result = engine.finalize(&id, 10).unwrap();
        assert_eq!(result.status, ProposalStatus::Ratified);
        assert_eq!(
            result.tally,
            Tally {
                support: 1,
                oppose: 0,
                abstain: 1,
                participating: 2,
                non_abstaining: 1
            }
        );
    }

    #[test]
    fn votes_arguments_and_amendments_close_at_deadline() {
        let mut engine = engine();
        let id = proposal("deadline");
        engine
            .submit(
                id.clone(),
                cthulhu("archivist"),
                agenda(1, None, "first"),
                0,
                10,
            )
            .unwrap();
        assert!(matches!(
            engine.cast_vote(&id, cthulhu("archivist"), VoteChoice::Support, 10),
            Err(GovernanceError::ProposalClosed)
        ));
        assert!(matches!(
            engine.add_argument(
                &id,
                cthulhu("archivist"),
                Position::Support,
                "late".into(),
                10
            ),
            Err(GovernanceError::ProposalClosed)
        ));
        assert!(matches!(
            engine.suggest_amendment(&id, cthulhu("archivist"), "late".into(), 10),
            Err(GovernanceError::ProposalClosed)
        ));
    }

    #[test]
    fn constitution_uses_stricter_threshold() {
        let mut engine = engine();
        let id = proposal("constitution");
        let doc = GovernanceDocument::Constitution(Constitution {
            version: 1,
            parent_hash: None,
            principles: vec!["mutual aid".into()],
            security_invariants: vec!["local policy remains sovereign".into()],
        });
        engine
            .submit(id.clone(), cthulhu("archivist"), doc, 0, 10)
            .unwrap();
        engine
            .cast_vote(&id, cthulhu("archivist"), VoteChoice::Support, 2)
            .unwrap();
        engine
            .cast_vote(&id, cthulhu("hermit"), VoteChoice::Oppose, 2)
            .unwrap();
        assert_eq!(
            engine.finalize(&id, 10).unwrap().status,
            ProposalStatus::Rejected
        );
    }

    #[test]
    fn no_quorum_expires() {
        let mut engine = engine();
        let id = proposal("noquorum");
        engine
            .submit(
                id.clone(),
                cthulhu("archivist"),
                agenda(1, None, "first"),
                0,
                10,
            )
            .unwrap();
        engine
            .cast_vote(&id, cthulhu("archivist"), VoteChoice::Support, 2)
            .unwrap();
        assert_eq!(
            engine.finalize(&id, 10).unwrap().status,
            ProposalStatus::Expired
        );
    }

    #[test]
    fn competing_agendas_are_detected_and_loser_gets_parent_conflict() {
        let mut engine = engine();
        let first = proposal("agendaone");
        let competing = proposal("agendatwo");
        engine
            .submit(
                first.clone(),
                cthulhu("archivist"),
                agenda(1, None, "path A"),
                0,
                10,
            )
            .unwrap();
        engine
            .submit(
                competing.clone(),
                cthulhu("hermit"),
                agenda(1, None, "path B"),
                0,
                11,
            )
            .unwrap();
        assert_eq!(
            engine.competing_proposals(&first).unwrap(),
            vec![competing.clone()]
        );
        ratify(
            &mut engine,
            &first,
            &[cthulhu("archivist"), cthulhu("hermit")],
        );
        engine
            .cast_vote(&competing, cthulhu("archivist"), VoteChoice::Support, 6)
            .unwrap();
        engine
            .cast_vote(&competing, cthulhu("hermit"), VoteChoice::Support, 6)
            .unwrap();
        assert_eq!(
            engine.finalize(&competing, 11).unwrap().status,
            ProposalStatus::ParentConflict
        );
    }

    #[test]
    fn agenda_requires_parent_hash_and_monotonic_version() {
        let mut engine = engine();
        let first = proposal("first");
        engine
            .submit(
                first.clone(),
                cthulhu("archivist"),
                agenda(1, None, "first"),
                0,
                10,
            )
            .unwrap();
        ratify(
            &mut engine,
            &first,
            &[cthulhu("archivist"), cthulhu("hermit")],
        );
        assert!(matches!(
            engine.submit(
                proposal("badparent"),
                cthulhu("archivist"),
                agenda(2, None, "bad"),
                11,
                20
            ),
            Err(GovernanceError::ParentMismatch)
        ));
        let head = engine.agenda().unwrap().hash.clone();
        engine
            .submit(
                proposal("second"),
                cthulhu("archivist"),
                agenda(2, Some(head), "second"),
                11,
                20,
            )
            .unwrap();
    }

    struct DenyAll;
    impl LocalOperatorPolicy for DenyAll {
        fn evaluate(&self, _action: &GovernanceAction) -> LocalPolicyDecision {
            LocalPolicyDecision::Deny {
                reason: "disabled by the local operator".into(),
            }
        }
    }

    #[test]
    fn council_ratification_cannot_override_local_policy() {
        let mut engine = engine();
        let id = proposal("selftest");
        let doc = GovernanceDocument::Action(ActionDocument {
            agenda_hash: None,
            action: GovernanceAction::ProtocolSelfTest {
                suite: SelfTestSuite::RoutingDeterminism,
            },
            rationale: "verify routing determinism".into(),
        });
        engine
            .submit(id.clone(), cthulhu("archivist"), doc, 0, 10)
            .unwrap();
        ratify(&mut engine, &id, &[cthulhu("archivist"), cthulhu("hermit")]);
        let authorization = engine.authorize_action(&id, &DenyAll).unwrap();
        assert!(!authorization.authorized);
        assert_eq!(authorization.reason, "disabled by the local operator");
    }

    #[test]
    fn malformed_and_oversized_documents_are_rejected() {
        let mut engine = engine();
        let oversized = agenda(1, None, &"x".repeat(MAX_TEXT_BYTES + 1));
        assert!(matches!(
            engine.submit(
                proposal("oversized"),
                cthulhu("archivist"),
                oversized,
                0,
                10
            ),
            Err(GovernanceError::InvalidDocument("agenda summary"))
        ));
        let arbitrary_command =
            serde_json::from_str::<GovernanceAction>(r#"{"type":"shell","command":"rm -rf /"}"#);
        assert!(arbitrary_command.is_err());
    }

    #[test]
    fn eligible_voters_are_snapshotted() {
        let mut engine = engine();
        let id = proposal("snapshot");
        engine
            .submit(
                id.clone(),
                cthulhu("archivist"),
                agenda(1, None, "first"),
                0,
                10,
            )
            .unwrap();
        let newcomer = cthulhu("oracle");
        engine.add_member(newcomer.clone()).unwrap();
        assert!(matches!(
            engine.cast_vote(&id, newcomer, VoteChoice::Support, 2),
            Err(GovernanceError::NotEligible)
        ));
    }

    #[test]
    fn round_trip_preserves_vote_replay_state() {
        let mut engine = engine();
        let id = proposal("persist");
        engine
            .submit(
                id.clone(),
                cthulhu("archivist"),
                agenda(1, None, "first"),
                0,
                10,
            )
            .unwrap();
        engine
            .cast_vote(&id, cthulhu("archivist"), VoteChoice::Support, 2)
            .unwrap();
        let encoded = serde_json::to_vec(&engine).unwrap();
        let mut restored: GovernanceEngine = serde_json::from_slice(&encoded).unwrap();
        restored.validate_loaded_state(3).unwrap();
        let receipt = restored
            .cast_vote(&id, cthulhu("archivist"), VoteChoice::Oppose, 3)
            .unwrap();
        assert!(receipt.replaced);
        assert_eq!(receipt.revision, 1);
        assert_eq!(restored.proposal(&id).unwrap().votes.len(), 1);
    }

    #[test]
    fn membership_churn_preserves_each_proposals_original_voter_basis() {
        let mut engine = engine();
        let original = proposal("originalmembership");
        engine
            .submit(
                original.clone(),
                cthulhu("archivist"),
                agenda(1, None, "first"),
                0,
                10,
            )
            .unwrap();
        let oracle = cthulhu("oracle");
        engine.add_member(oracle.clone()).unwrap();
        let later = proposal("latermembership");
        engine
            .submit(
                later.clone(),
                cthulhu("archivist"),
                agenda(1, None, "competing"),
                1,
                11,
            )
            .unwrap();

        assert_eq!(engine.proposal(&original).unwrap().membership_revision, 0);
        assert_eq!(engine.proposal(&later).unwrap().membership_revision, 1);
        assert!(
            !engine
                .proposal(&original)
                .unwrap()
                .eligible_voters
                .contains(&oracle)
        );
        assert!(
            engine
                .proposal(&later)
                .unwrap()
                .eligible_voters
                .contains(&oracle)
        );

        let restored: GovernanceEngine =
            serde_json::from_slice(&serde_json::to_vec(&engine).unwrap()).unwrap();
        restored.validate_loaded_state(2).unwrap();
    }

    #[test]
    fn hostile_persisted_governance_state_is_rejected() {
        let mut engine = engine();
        let id = proposal("tampered");
        engine
            .submit(
                id.clone(),
                cthulhu("archivist"),
                agenda(1, None, "first"),
                0,
                10,
            )
            .unwrap();
        engine.proposals.get_mut(&id).unwrap().document_hash = hash_for_test("tampered");
        assert!(matches!(
            engine.validate_loaded_state(2),
            Err(GovernanceError::CorruptState("document hash"))
        ));
    }

    #[test]
    fn hostile_reload_cannot_expand_voters_and_forge_ratification() {
        let mut engine = engine();
        let id = proposal("forgedmembership");
        let document = GovernanceDocument::Action(ActionDocument {
            agenda_hash: None,
            action: GovernanceAction::ProtocolSelfTest {
                suite: SelfTestSuite::EnvelopeValidation,
            },
            rationale: "test membership-bound quorum".into(),
        });
        engine
            .submit(id.clone(), cthulhu("archivist"), document, 0, 10)
            .unwrap();
        engine
            .cast_vote(&id, cthulhu("archivist"), VoteChoice::Support, 2)
            .unwrap();

        let encoded = serde_json::to_vec(&engine).unwrap();
        let mut restored: GovernanceEngine = serde_json::from_slice(&encoded).unwrap();
        let attacker = cthulhu("attacker");
        let tampered = restored.proposals.get_mut(&id).unwrap();
        tampered.eligible_voters.insert(attacker.clone());
        tampered.votes.insert(
            attacker,
            VoteRecord {
                choice: VoteChoice::Support,
                cast_at: 3,
                revision: 0,
            },
        );
        tampered.status = ProposalStatus::Ratified;

        assert!(matches!(
            restored.validate_loaded_state(10),
            Err(GovernanceError::CorruptState("proposal membership basis"))
        ));
        assert!(matches!(
            restored.authorize_action(&id, &DenyAll),
            Err(GovernanceError::InvalidRatificationEvidence)
        ));
    }

    #[test]
    fn hostile_reload_cannot_rebind_old_proposal_to_later_membership() {
        let mut engine = engine();
        let id = proposal("staleproposalbinding");
        engine
            .submit(
                id.clone(),
                cthulhu("archivist"),
                agenda(1, None, "first"),
                0,
                10,
            )
            .unwrap();
        let oracle = cthulhu("oracle");
        engine.add_member(oracle.clone()).unwrap();

        let later_membership = engine.membership_history.last().unwrap().clone();
        let tampered = engine.proposals.get_mut(&id).unwrap();
        tampered.membership_revision = later_membership.revision;
        tampered.membership_hash = later_membership.hash;
        tampered.eligible_voters.insert(oracle);

        let restored: GovernanceEngine =
            serde_json::from_slice(&serde_json::to_vec(&engine).unwrap()).unwrap();
        assert!(matches!(
            restored.validate_loaded_state(2),
            Err(GovernanceError::CorruptState("proposal membership binding"))
        ));
    }

    #[test]
    fn hostile_reload_rejects_rewritten_membership_history() {
        let mut engine = engine();
        engine.add_member(cthulhu("oracle")).unwrap();
        engine
            .membership_history
            .last_mut()
            .unwrap()
            .members
            .push(cthulhu("trickster"));

        let restored: GovernanceEngine =
            serde_json::from_slice(&serde_json::to_vec(&engine).unwrap()).unwrap();
        assert!(matches!(
            restored.validate_loaded_state(0),
            Err(GovernanceError::CorruptState("membership history hash"))
        ));
    }

    #[test]
    fn forged_ratified_action_without_votes_cannot_execute() {
        let mut engine = engine();
        let id = proposal("forgedaction");
        let document = GovernanceDocument::Action(ActionDocument {
            agenda_hash: None,
            action: GovernanceAction::ProtocolSelfTest {
                suite: SelfTestSuite::EnvelopeValidation,
            },
            rationale: "test".into(),
        });
        engine
            .submit(id.clone(), cthulhu("archivist"), document, 0, 10)
            .unwrap();
        engine.proposals.get_mut(&id).unwrap().status = ProposalStatus::Ratified;
        assert!(matches!(
            engine.validate_loaded_state(10),
            Err(GovernanceError::CorruptState("ratification evidence"))
        ));
        assert!(matches!(
            engine.authorize_action(&id, &DenyAll),
            Err(GovernanceError::InvalidRatificationEvidence)
        ));
    }

    fn hash_for_test(value: &str) -> String {
        format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
    }
}
