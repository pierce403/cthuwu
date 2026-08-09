//! Deterministic, local token-weighted governance.
//!
//! This module consumes normalized observations and produces advisory
//! governance outcomes. It has no network, key, transaction, command, process,
//! filesystem, or operator-authorization surface. In particular, transferable
//! UWU holdings can influence the closed governance subjects represented here,
//! but can never authenticate an operator or grant operating-system authority.

use crate::token_eye::{Address, ReputationTier};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{collections::BTreeMap, error::Error, fmt, str::FromStr};

pub const NORMALIZED_WEIGHT_MAX_BPS: u16 = 10_000;
const BASIS_POINTS_DENOMINATOR: u64 = 10_000;
const NEUTRAL_MULTIPLIER_BPS: u16 = 10_000;

/// A content-addressed proposal identifier.
///
/// Parsing accepts 64 hexadecimal digits with an optional lowercase `0x`
/// prefix. Display and serialization use the canonical prefixed lowercase form.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TokenProposalId([u8; 32]);

impl TokenProposalId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl FromStr for TokenProposalId {
    type Err = TokenGovernanceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let encoded = value.strip_prefix("0x").unwrap_or(value);
        if encoded.len() != 64 {
            return Err(TokenGovernanceError::InvalidProposalId(
                "proposal id must contain exactly 64 hexadecimal digits",
            ));
        }

        let mut bytes = [0_u8; 32];
        for (pair, byte) in encoded.as_bytes().chunks_exact(2).zip(&mut bytes) {
            let high =
                decode_hex_nibble(pair[0]).ok_or(TokenGovernanceError::InvalidProposalId(
                    "proposal id contains a non-hexadecimal digit",
                ))?;
            let low = decode_hex_nibble(pair[1]).ok_or(TokenGovernanceError::InvalidProposalId(
                "proposal id contains a non-hexadecimal digit",
            ))?;
            *byte = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for TokenProposalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("0x")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for TokenProposalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Serialize for TokenProposalId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for TokenProposalId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

/// Closed subjects that token governance may influence.
///
/// There is deliberately no operator, shell, process, credential, or arbitrary
/// execution subject in this enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceSubject {
    NatureAdjustment,
    CouncilPolicy,
    EconomicPolicy,
    SkillPropagationPriority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BallotChoice {
    Yes,
    No,
    Abstain,
}

/// Full-strength tier multipliers, expressed in basis points where 10,000 is
/// neutral. Multipliers use `u16` so their absolute maximum remains bounded.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct TierVoteMultipliers {
    pub whale_bps: u16,
    pub elder_bps: u16,
    pub acolyte_bps: u16,
    pub initiate_bps: u16,
    pub unproven_bps: u16,
}

impl TierVoteMultipliers {
    pub const fn for_tier(self, tier: ReputationTier) -> u16 {
        match tier {
            ReputationTier::Whale => self.whale_bps,
            ReputationTier::Elder => self.elder_bps,
            ReputationTier::Acolyte => self.acolyte_bps,
            ReputationTier::Initiate => self.initiate_bps,
            ReputationTier::Unproven => self.unproven_bps,
        }
    }
}

impl Default for TierVoteMultipliers {
    fn default() -> Self {
        Self {
            whale_bps: 20_000,
            elder_bps: 15_000,
            acolyte_bps: NEUTRAL_MULTIPLIER_BPS,
            initiate_bps: 7_500,
            unproven_bps: 5_000,
        }
    }
}

/// Local policy for one token-weighted ballot box.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct TokenGovernancePolicy {
    /// Raw holding weight needed for quorum. Zero is intentionally permitted.
    pub quorum_bps: u16,
    /// Weighted yes share of yes + no needed for approval.
    pub approval_bps: u16,
    /// Defaults to `Unproven`, allowing a zero-balance address to cast a
    /// zero-weight ballot.
    pub minimum_tier: ReputationTier,
    /// Interpolates tier multipliers from neutral (0) to full strength (10,000).
    pub nature_tier_scale_bps: u16,
    pub tier_multipliers: TierVoteMultipliers,
}

impl Default for TokenGovernancePolicy {
    fn default() -> Self {
        Self {
            quorum_bps: 0,
            approval_bps: 5_001,
            minimum_tier: ReputationTier::Unproven,
            nature_tier_scale_bps: NORMALIZED_WEIGHT_MAX_BPS,
            tier_multipliers: TierVoteMultipliers::default(),
        }
    }
}

impl TokenGovernancePolicy {
    pub fn validate(self) -> Result<Self, TokenGovernanceError> {
        validate_normalized("quorum", self.quorum_bps)?;
        validate_normalized("approval", self.approval_bps)?;
        validate_normalized("Nature tier scale", self.nature_tier_scale_bps)?;
        Ok(self)
    }

    /// Derives tier sensitivity from the Nature cooperation slider.
    ///
    /// Fully cooperative Natures ignore tier differences; fully competitive
    /// Natures apply the configured multipliers at full strength.
    pub fn with_nature_cooperation(
        mut self,
        cooperation: u8,
    ) -> Result<Self, TokenGovernanceError> {
        if cooperation > 100 {
            return Err(TokenGovernanceError::InvalidPolicy(
                "Nature cooperation must be between 0 and 100",
            ));
        }
        self.nature_tier_scale_bps = u16::from(100 - cooperation) * 100;
        self.validate()
    }

    pub fn scaled_multiplier(self, tier: ReputationTier) -> u16 {
        interpolate_multiplier(
            self.tier_multipliers.for_tier(tier),
            self.nature_tier_scale_bps,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct TokenBallot {
    pub voter: Address,
    pub tier: ReputationTier,
    /// Normalized share of locally eligible UWU holding, from 0 through 10,000.
    pub holding_weight_bps: u16,
    /// `holding_weight_bps` after the Nature-scaled tier multiplier.
    pub effective_weight_bps: u16,
    pub choice: BallotChoice,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct GovernanceTally {
    pub ballot_count: u64,
    pub yes_holding_bps: u16,
    pub no_holding_bps: u16,
    pub abstain_holding_bps: u16,
    pub participating_holding_bps: u16,
    pub yes_effective_bps: u16,
    pub no_effective_bps: u16,
    pub abstain_effective_bps: u16,
    pub participating_effective_bps: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceOutcome {
    Approved,
    Rejected,
    QuorumNotMet,
    NoDecisiveWeight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct GovernanceResult {
    pub proposal_id: TokenProposalId,
    pub subject: GovernanceSubject,
    pub tally: GovernanceTally,
    pub quorum_required_weight_bps: u16,
    pub quorum_met: bool,
    /// Weighted yes share of yes + no. Abstentions do not affect this ratio.
    pub approval_achieved_bps: u16,
    pub approval_required_bps: u16,
    pub outcome: GovernanceOutcome,
}

/// One deterministic local view of a proposal's token ballots.
#[derive(Clone, Debug)]
pub struct TokenBallotBox {
    proposal_id: TokenProposalId,
    subject: GovernanceSubject,
    eligible_weight_bps: u16,
    policy: TokenGovernancePolicy,
    ballots: BTreeMap<Address, TokenBallot>,
    participating_holding_bps: u16,
}

impl TokenBallotBox {
    pub fn new(
        proposal_id: TokenProposalId,
        subject: GovernanceSubject,
        eligible_weight_bps: u16,
        policy: TokenGovernancePolicy,
    ) -> Result<Self, TokenGovernanceError> {
        validate_normalized("eligible weight", eligible_weight_bps)?;
        let policy = policy.validate()?;
        Ok(Self {
            proposal_id,
            subject,
            eligible_weight_bps,
            policy,
            ballots: BTreeMap::new(),
            participating_holding_bps: 0,
        })
    }

    pub fn from_hex_id(
        proposal_id: &str,
        subject: GovernanceSubject,
        eligible_weight_bps: u16,
        policy: TokenGovernancePolicy,
    ) -> Result<Self, TokenGovernanceError> {
        Self::new(proposal_id.parse()?, subject, eligible_weight_bps, policy)
    }

    pub const fn proposal_id(&self) -> TokenProposalId {
        self.proposal_id
    }

    pub const fn subject(&self) -> GovernanceSubject {
        self.subject
    }

    pub const fn eligible_weight_bps(&self) -> u16 {
        self.eligible_weight_bps
    }

    pub const fn policy(&self) -> TokenGovernancePolicy {
        self.policy
    }

    pub fn ballot(&self, voter: Address) -> Option<&TokenBallot> {
        self.ballots.get(&voter)
    }

    pub fn ballots(&self) -> impl ExactSizeIterator<Item = &TokenBallot> {
        self.ballots.values()
    }

    /// Casts exactly one ballot for an address. Ballots are immutable once cast.
    pub fn cast(
        &mut self,
        voter: Address,
        tier: ReputationTier,
        holding_weight_bps: u16,
        choice: BallotChoice,
    ) -> Result<TokenBallot, TokenGovernanceError> {
        validate_normalized("ballot holding weight", holding_weight_bps)?;
        if self.ballots.contains_key(&voter) {
            return Err(TokenGovernanceError::DuplicateBallot(voter));
        }
        if !tier.meets(self.policy.minimum_tier) {
            return Err(TokenGovernanceError::TierBelowMinimum {
                actual: tier,
                minimum: self.policy.minimum_tier,
            });
        }
        let next_participating = self
            .participating_holding_bps
            .checked_add(holding_weight_bps)
            .ok_or(TokenGovernanceError::EligibleWeightExceeded)?;
        if next_participating > self.eligible_weight_bps {
            return Err(TokenGovernanceError::EligibleWeightExceeded);
        }

        let effective_weight_bps =
            apply_multiplier(holding_weight_bps, self.policy.scaled_multiplier(tier));
        let ballot = TokenBallot {
            voter,
            tier,
            holding_weight_bps,
            effective_weight_bps,
            choice,
        };
        self.ballots.insert(voter, ballot);
        self.participating_holding_bps = next_participating;
        Ok(ballot)
    }

    pub fn tally(&self) -> GovernanceTally {
        let mut tally = GovernanceTally {
            ballot_count: u64::try_from(self.ballots.len()).unwrap_or(u64::MAX),
            ..GovernanceTally::default()
        };
        for ballot in self.ballots.values() {
            match ballot.choice {
                BallotChoice::Yes => {
                    tally.yes_holding_bps = tally
                        .yes_holding_bps
                        .saturating_add(ballot.holding_weight_bps);
                    tally.yes_effective_bps = tally
                        .yes_effective_bps
                        .saturating_add(ballot.effective_weight_bps);
                }
                BallotChoice::No => {
                    tally.no_holding_bps = tally
                        .no_holding_bps
                        .saturating_add(ballot.holding_weight_bps);
                    tally.no_effective_bps = tally
                        .no_effective_bps
                        .saturating_add(ballot.effective_weight_bps);
                }
                BallotChoice::Abstain => {
                    tally.abstain_holding_bps = tally
                        .abstain_holding_bps
                        .saturating_add(ballot.holding_weight_bps);
                    tally.abstain_effective_bps = tally
                        .abstain_effective_bps
                        .saturating_add(ballot.effective_weight_bps);
                }
            }
        }
        tally.participating_holding_bps = tally
            .yes_holding_bps
            .saturating_add(tally.no_holding_bps)
            .saturating_add(tally.abstain_holding_bps);
        tally.participating_effective_bps = tally
            .yes_effective_bps
            .saturating_add(tally.no_effective_bps)
            .saturating_add(tally.abstain_effective_bps);
        tally
    }

    pub fn result(&self) -> GovernanceResult {
        let tally = self.tally();
        let quorum_required_weight_bps =
            ceil_basis_points(self.eligible_weight_bps, self.policy.quorum_bps);
        let quorum_met = tally.participating_holding_bps >= quorum_required_weight_bps;
        let decisive_weight = tally
            .yes_effective_bps
            .saturating_add(tally.no_effective_bps);
        let approval_achieved_bps = if decisive_weight == 0 {
            0
        } else {
            u16::try_from(
                u64::from(tally.yes_effective_bps) * BASIS_POINTS_DENOMINATOR
                    / u64::from(decisive_weight),
            )
            .unwrap_or(NORMALIZED_WEIGHT_MAX_BPS)
        };
        let outcome = if !quorum_met {
            GovernanceOutcome::QuorumNotMet
        } else if decisive_weight == 0 {
            GovernanceOutcome::NoDecisiveWeight
        } else if approval_achieved_bps >= self.policy.approval_bps {
            GovernanceOutcome::Approved
        } else {
            GovernanceOutcome::Rejected
        };

        GovernanceResult {
            proposal_id: self.proposal_id,
            subject: self.subject,
            tally,
            quorum_required_weight_bps,
            quorum_met,
            approval_achieved_bps,
            approval_required_bps: self.policy.approval_bps,
            outcome,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TokenGovernanceError {
    InvalidProposalId(&'static str),
    InvalidPolicy(&'static str),
    WeightOutOfRange {
        field: &'static str,
        value: u16,
    },
    DuplicateBallot(Address),
    TierBelowMinimum {
        actual: ReputationTier,
        minimum: ReputationTier,
    },
    EligibleWeightExceeded,
}

impl fmt::Display for TokenGovernanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProposalId(reason) => write!(formatter, "invalid proposal id: {reason}"),
            Self::InvalidPolicy(reason) => {
                write!(formatter, "invalid token governance policy: {reason}")
            }
            Self::WeightOutOfRange { field, value } => write!(
                formatter,
                "{field} must be between 0 and {NORMALIZED_WEIGHT_MAX_BPS} basis points, got {value}"
            ),
            Self::DuplicateBallot(voter) => {
                write!(formatter, "address {voter} has already cast a ballot")
            }
            Self::TierBelowMinimum { actual, minimum } => write!(
                formatter,
                "voter tier {actual:?} is below configured minimum {minimum:?}"
            ),
            Self::EligibleWeightExceeded => {
                formatter.write_str("ballots exceed the locally eligible normalized weight")
            }
        }
    }
}

impl Error for TokenGovernanceError {}

fn validate_normalized(field: &'static str, value: u16) -> Result<(), TokenGovernanceError> {
    if value > NORMALIZED_WEIGHT_MAX_BPS {
        return Err(TokenGovernanceError::WeightOutOfRange { field, value });
    }
    Ok(())
}

fn interpolate_multiplier(full_strength_bps: u16, nature_scale_bps: u16) -> u16 {
    let neutral = i64::from(NEUTRAL_MULTIPLIER_BPS);
    let delta = i64::from(full_strength_bps) - neutral;
    let interpolated =
        neutral + delta * i64::from(nature_scale_bps) / i64::from(NORMALIZED_WEIGHT_MAX_BPS);
    u16::try_from(interpolated.clamp(0, i64::from(u16::MAX)))
        .expect("clamped multiplier fits in u16")
}

fn apply_multiplier(weight_bps: u16, multiplier_bps: u16) -> u16 {
    let weighted =
        u64::from(weight_bps) * u64::from(multiplier_bps) / u64::from(NORMALIZED_WEIGHT_MAX_BPS);
    u16::try_from(weighted.min(u64::from(u16::MAX))).expect("bounded weight fits in u16")
}

fn ceil_basis_points(value: u16, rate_bps: u16) -> u16 {
    let product = u64::from(value) * u64::from(rate_bps);
    let rounded = product.saturating_add(BASIS_POINTS_DENOMINATOR - 1) / BASIS_POINTS_DENOMINATOR;
    u16::try_from(rounded).unwrap_or(u16::MAX)
}

const fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROPOSAL_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn address(suffix: u8) -> Address {
        let mut bytes = [0_u8; 20];
        bytes[19] = suffix;
        Address::from_bytes(bytes)
    }

    fn ballot_box(policy: TokenGovernancePolicy, eligible: u16) -> TokenBallotBox {
        TokenBallotBox::from_hex_id(
            PROPOSAL_HEX,
            GovernanceSubject::NatureAdjustment,
            eligible,
            policy,
        )
        .unwrap()
    }

    #[test]
    fn proposal_ids_validate_and_canonicalize_exactly_64_hex_digits() {
        let bare: TokenProposalId = PROPOSAL_HEX.parse().unwrap();
        let prefixed: TokenProposalId = format!("0x{PROPOSAL_HEX}").parse().unwrap();
        assert_eq!(bare, prefixed);
        assert_eq!(bare.to_string(), format!("0x{PROPOSAL_HEX}"));
        assert_eq!(
            serde_json::to_string(&bare).unwrap(),
            format!("\"0x{PROPOSAL_HEX}\"")
        );

        assert!("abc".parse::<TokenProposalId>().is_err());
        assert!(
            format!("{PROPOSAL_HEX}0")
                .parse::<TokenProposalId>()
                .is_err()
        );
        assert!(
            format!("{}z", &PROPOSAL_HEX[..63])
                .parse::<TokenProposalId>()
                .is_err()
        );
        assert!(
            format!(" 0x{PROPOSAL_HEX}")
                .parse::<TokenProposalId>()
                .is_err()
        );
        assert!(
            format!("0X{PROPOSAL_HEX}")
                .parse::<TokenProposalId>()
                .is_err()
        );
    }

    #[test]
    fn tier_weighting_changes_approval_deterministically() {
        let policy = TokenGovernancePolicy {
            quorum_bps: 0,
            approval_bps: 6_000,
            ..TokenGovernancePolicy::default()
        };
        let mut ballots = ballot_box(policy, 200);
        let whale = ballots
            .cast(address(1), ReputationTier::Whale, 100, BallotChoice::Yes)
            .unwrap();
        let acolyte = ballots
            .cast(address(2), ReputationTier::Acolyte, 100, BallotChoice::No)
            .unwrap();

        assert_eq!(whale.effective_weight_bps, 200);
        assert_eq!(acolyte.effective_weight_bps, 100);
        let result = ballots.result();
        assert_eq!(result.approval_achieved_bps, 6_666);
        assert_eq!(result.outcome, GovernanceOutcome::Approved);
    }

    #[test]
    fn cooperative_nature_can_neutralize_tier_multipliers() {
        let policy = TokenGovernancePolicy::default()
            .with_nature_cooperation(100)
            .unwrap();
        assert_eq!(policy.scaled_multiplier(ReputationTier::Whale), 10_000);
        assert_eq!(policy.scaled_multiplier(ReputationTier::Unproven), 10_000);

        let competitive = TokenGovernancePolicy::default()
            .with_nature_cooperation(0)
            .unwrap();
        assert_eq!(competitive.scaled_multiplier(ReputationTier::Whale), 20_000);
        assert_eq!(
            competitive.scaled_multiplier(ReputationTier::Initiate),
            7_500
        );
        assert!(
            TokenGovernancePolicy::default()
                .with_nature_cooperation(101)
                .is_err()
        );
    }

    #[test]
    fn duplicate_address_ballots_are_rejected_without_replacement() {
        let mut ballots = ballot_box(TokenGovernancePolicy::default(), 100);
        ballots
            .cast(address(1), ReputationTier::Acolyte, 100, BallotChoice::Yes)
            .unwrap();
        let error = ballots
            .cast(address(1), ReputationTier::Whale, 0, BallotChoice::No)
            .unwrap_err();
        assert_eq!(error, TokenGovernanceError::DuplicateBallot(address(1)));
        assert_eq!(ballots.ballots().len(), 1);
        assert_eq!(ballots.result().outcome, GovernanceOutcome::Approved);
    }

    #[test]
    fn default_is_permissive_for_zero_balance_but_grants_no_weight() {
        let policy = TokenGovernancePolicy::default();
        assert_eq!(policy.minimum_tier, ReputationTier::Unproven);
        assert_eq!(policy.quorum_bps, 0);
        let mut ballots = ballot_box(policy, 0);
        let ballot = ballots
            .cast(address(1), ReputationTier::Unproven, 0, BallotChoice::Yes)
            .unwrap();
        assert_eq!(ballot.effective_weight_bps, 0);

        let result = ballots.result();
        assert!(result.quorum_met);
        assert_eq!(result.tally.ballot_count, 1);
        assert_eq!(result.tally.participating_holding_bps, 0);
        assert_eq!(result.tally.participating_effective_bps, 0);
        assert_eq!(result.outcome, GovernanceOutcome::NoDecisiveWeight);
    }

    #[test]
    fn configured_minimum_tier_and_weight_bounds_are_enforced() {
        let policy = TokenGovernancePolicy {
            minimum_tier: ReputationTier::Acolyte,
            ..TokenGovernancePolicy::default()
        };
        let mut ballots = ballot_box(policy, 100);
        assert!(matches!(
            ballots.cast(
                address(1),
                ReputationTier::Initiate,
                0,
                BallotChoice::Abstain,
            ),
            Err(TokenGovernanceError::TierBelowMinimum { .. })
        ));
        assert!(matches!(
            TokenBallotBox::from_hex_id(
                PROPOSAL_HEX,
                GovernanceSubject::CouncilPolicy,
                10_001,
                TokenGovernancePolicy::default(),
            ),
            Err(TokenGovernanceError::WeightOutOfRange { .. })
        ));
        assert!(matches!(
            ballots.cast(address(2), ReputationTier::Acolyte, 101, BallotChoice::Yes,),
            Err(TokenGovernanceError::EligibleWeightExceeded)
        ));
    }

    #[test]
    fn quorum_uses_raw_participation_and_abstentions_count() {
        let policy = TokenGovernancePolicy {
            quorum_bps: 5_000,
            approval_bps: 5_001,
            ..TokenGovernancePolicy::default()
        };
        let mut ballots = ballot_box(policy, 1_000);
        ballots
            .cast(address(1), ReputationTier::Acolyte, 499, BallotChoice::Yes)
            .unwrap();
        let before = ballots.result();
        assert_eq!(before.quorum_required_weight_bps, 500);
        assert_eq!(before.outcome, GovernanceOutcome::QuorumNotMet);

        ballots
            .cast(
                address(2),
                ReputationTier::Unproven,
                1,
                BallotChoice::Abstain,
            )
            .unwrap();
        let after = ballots.result();
        assert!(after.quorum_met);
        assert_eq!(after.outcome, GovernanceOutcome::Approved);
    }
}
