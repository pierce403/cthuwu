//! Deterministic token-economic inputs, mandatory effects, and transaction intents.
//!
//! Chain observations are normalized into basis points for scoring. Unverified observations are
//! rejected: an economic evaluation cannot silently substitute a zero balance or keep operating on
//! stale data. On-chain actions are represented as explicit state machines so an intent is never
//! confused with a signed, submitted, or confirmed transaction.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const TOKEN_ECONOMICS_SCHEMA_VERSION: u32 = 1;
pub const NORMALIZED_ECONOMIC_MAX: u16 = 10_000;
pub const BASE_CHAIN_ID: u64 = 8_453;
pub const DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS: u16 = 100;

/// A bounded view of one entity's economic position relative to locally configured reference
/// values. A value of 10,000 represents full credit, not a token quantity or a percentage of total
/// supply.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenEconomicSnapshot {
    pub balance_basis_points: u16,
    pub stake_basis_points: u16,
    pub reward_basis_points: u16,
    /// Set only after the chain adapter has validated the RPC response, chain, contract, and block.
    pub trustworthy: bool,
}

impl TokenEconomicSnapshot {
    pub fn validate(self) -> Result<()> {
        for (name, value) in [
            ("balance", self.balance_basis_points),
            ("stake", self.stake_basis_points),
            ("reward", self.reward_basis_points),
        ] {
            ensure!(
                value <= NORMALIZED_ECONOMIC_MAX,
                "normalized token {name} exceeds {NORMALIZED_ECONOMIC_MAX} basis points"
            );
        }
        Ok(())
    }
}

/// Local policy controlling how strongly verified token observations affect this Tentacle.
///
/// Starting and ordinary interaction do not require stake. Propagation does: the aggressive
/// default requires a normalized one-percent stake and can be tuned per Tentacle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenEconomicPolicy {
    pub schema_version: u32,
    pub wealth_sensitivity_basis_points: u16,
    pub influence_sensitivity_basis_points: u16,
    pub growth_sensitivity_basis_points: u16,
    pub engagement_sensitivity_basis_points: u16,
    pub starvation_relief_sensitivity_basis_points: u16,
    pub emergency_relief_per_expenditure_basis_points: u16,
    pub max_emergency_expenditure_basis_points: u16,
    pub propagation_minimum_stake_basis_points: u16,
}

impl Default for TokenEconomicPolicy {
    fn default() -> Self {
        Self {
            schema_version: TOKEN_ECONOMICS_SCHEMA_VERSION,
            wealth_sensitivity_basis_points: NORMALIZED_ECONOMIC_MAX,
            influence_sensitivity_basis_points: NORMALIZED_ECONOMIC_MAX,
            growth_sensitivity_basis_points: NORMALIZED_ECONOMIC_MAX,
            engagement_sensitivity_basis_points: NORMALIZED_ECONOMIC_MAX,
            starvation_relief_sensitivity_basis_points: NORMALIZED_ECONOMIC_MAX,
            emergency_relief_per_expenditure_basis_points: NORMALIZED_ECONOMIC_MAX,
            max_emergency_expenditure_basis_points: NORMALIZED_ECONOMIC_MAX,
            propagation_minimum_stake_basis_points: DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS,
        }
    }
}

impl TokenEconomicPolicy {
    /// Applies Nature appetites as economic sensitivities. Each appetite is in the Nature's 0..=100
    /// range. This changes the strength of token effects, never whether a Tentacle may start.
    pub fn with_nature_appetites(
        mut self,
        engagement: u8,
        growth: u8,
        wealth: u8,
        influence: u8,
    ) -> Result<Self> {
        for (name, value) in [
            ("engagement", engagement),
            ("growth", growth),
            ("wealth", wealth),
            ("influence", influence),
        ] {
            ensure!(value <= 100, "Nature {name} appetite exceeds 100");
        }
        self.engagement_sensitivity_basis_points = u16::from(engagement) * 100;
        self.growth_sensitivity_basis_points = u16::from(growth) * 100;
        self.wealth_sensitivity_basis_points = u16::from(wealth) * 100;
        self.influence_sensitivity_basis_points = u16::from(influence) * 100;
        self.starvation_relief_sensitivity_basis_points = u16::from(wealth) * 100;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(self) -> Result<()> {
        ensure!(
            self.schema_version == TOKEN_ECONOMICS_SCHEMA_VERSION,
            "unsupported token-economic policy schema version {}",
            self.schema_version
        );
        for (name, value) in [
            ("wealth sensitivity", self.wealth_sensitivity_basis_points),
            (
                "influence sensitivity",
                self.influence_sensitivity_basis_points,
            ),
            ("growth sensitivity", self.growth_sensitivity_basis_points),
            (
                "engagement sensitivity",
                self.engagement_sensitivity_basis_points,
            ),
            (
                "starvation-relief sensitivity",
                self.starvation_relief_sensitivity_basis_points,
            ),
            (
                "emergency relief rate",
                self.emergency_relief_per_expenditure_basis_points,
            ),
            (
                "maximum emergency expenditure",
                self.max_emergency_expenditure_basis_points,
            ),
            (
                "propagation minimum stake",
                self.propagation_minimum_stake_basis_points,
            ),
        ] {
            ensure!(
                value <= NORMALIZED_ECONOMIC_MAX,
                "token-economic {name} exceeds {NORMALIZED_ECONOMIC_MAX} basis points"
            );
        }
        ensure!(
            self.emergency_relief_per_expenditure_basis_points > 0,
            "emergency relief per expenditure must be positive"
        );
        Ok(())
    }

    pub fn effects(self, snapshot: TokenEconomicSnapshot) -> Result<TokenEconomicEffects> {
        self.validate()?;
        snapshot.validate()?;
        ensure!(
            snapshot.trustworthy,
            "token-economic observation is unavailable or unverified"
        );
        let wealth_adjustment_basis_points = multiply_basis_points(
            snapshot.balance_basis_points,
            self.wealth_sensitivity_basis_points,
        );
        let influence_adjustment_basis_points = multiply_basis_points(
            snapshot.stake_basis_points,
            self.influence_sensitivity_basis_points,
        );
        let growth_adjustment_basis_points = multiply_basis_points(
            snapshot.reward_basis_points,
            self.growth_sensitivity_basis_points,
        );
        let engagement_adjustment_basis_points = multiply_basis_points(
            snapshot.balance_basis_points,
            self.engagement_sensitivity_basis_points,
        );
        let starvation_relief_basis_points = multiply_basis_points(
            snapshot.balance_basis_points,
            self.starvation_relief_sensitivity_basis_points,
        );
        let propagation_stake_eligible =
            snapshot.stake_basis_points >= self.propagation_minimum_stake_basis_points;

        Ok(TokenEconomicEffects {
            wealth_adjustment_basis_points,
            influence_adjustment_basis_points,
            growth_adjustment_basis_points,
            engagement_adjustment_basis_points,
            starvation_relief_basis_points,
            propagation_stake_eligible,
        })
    }
}

/// The economic identity whose holdings are allowed to affect node lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomicHolderRole {
    TentacleTreasury,
    OperatorAcolyte,
    ParentTentacle,
    ChildTentacle,
    CouncilMember,
}

/// Provenance for an active node-level economic observation.
///
/// `observed_block_number` remains optional because a plain `balanceOf(..., "latest")` response
/// does not identify its block. Callers must not fabricate one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EconomicObservationProvenance {
    pub holder_address: [u8; 20],
    pub holder_role: EconomicHolderRole,
    pub chain_id: u64,
    pub token_contract: [u8; 20],
    pub observed_at_unix_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_block_number: Option<u64>,
    /// Hash of the exact token normalization/stake/reward configuration.
    pub configuration_identity: [u8; 32],
}

impl EconomicObservationProvenance {
    pub fn base(
        holder_address: [u8; 20],
        holder_role: EconomicHolderRole,
        token_contract: [u8; 20],
        observed_at_unix_seconds: u64,
        observed_block_number: Option<u64>,
        configuration_identity: [u8; 32],
    ) -> Result<Self> {
        let provenance = Self {
            holder_address,
            holder_role,
            chain_id: BASE_CHAIN_ID,
            token_contract,
            observed_at_unix_seconds,
            observed_block_number,
            configuration_identity,
        };
        provenance.validate()?;
        Ok(provenance)
    }

    pub fn validate(self) -> Result<()> {
        ensure!(
            self.holder_address != [0; 20],
            "economic holder address cannot be zero"
        );
        ensure!(
            self.token_contract != [0; 20],
            "economic token contract cannot be zero"
        );
        ensure!(
            self.chain_id == BASE_CHAIN_ID,
            "economic observation must be bound to Base chain id {BASE_CHAIN_ID}"
        );
        ensure!(
            self.observed_at_unix_seconds > 0,
            "economic observation timestamp must be positive"
        );
        ensure!(
            self.configuration_identity != [0; 32],
            "economic configuration identity cannot be zero"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenEconomicEffects {
    pub wealth_adjustment_basis_points: u16,
    pub influence_adjustment_basis_points: u16,
    pub growth_adjustment_basis_points: u16,
    pub engagement_adjustment_basis_points: u16,
    pub starvation_relief_basis_points: u16,
    pub propagation_stake_eligible: bool,
}

impl TokenEconomicEffects {
    fn validate(self) -> Result<()> {
        for (name, value) in [
            ("wealth adjustment", self.wealth_adjustment_basis_points),
            (
                "influence adjustment",
                self.influence_adjustment_basis_points,
            ),
            ("growth adjustment", self.growth_adjustment_basis_points),
            (
                "engagement adjustment",
                self.engagement_adjustment_basis_points,
            ),
            ("starvation relief", self.starvation_relief_basis_points),
        ] {
            ensure!(
                value <= NORMALIZED_ECONOMIC_MAX,
                "token-economic {name} exceeds {NORMALIZED_ECONOMIC_MAX} basis points"
            );
        }
        Ok(())
    }
}

/// Persisted snapshot plus the exact policy and derived effects used by Scales.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordedTokenEconomics {
    pub snapshot: TokenEconomicSnapshot,
    pub policy: TokenEconomicPolicy,
    pub effects: TokenEconomicEffects,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<EconomicObservationProvenance>,
}

impl RecordedTokenEconomics {
    pub fn new(snapshot: TokenEconomicSnapshot, policy: TokenEconomicPolicy) -> Result<Self> {
        let effects = policy.effects(snapshot)?;
        Ok(Self {
            snapshot,
            policy,
            effects,
            provenance: None,
        })
    }

    pub fn new_with_provenance(
        snapshot: TokenEconomicSnapshot,
        policy: TokenEconomicPolicy,
        provenance: EconomicObservationProvenance,
    ) -> Result<Self> {
        provenance.validate()?;
        let effects = policy.effects(snapshot)?;
        Ok(Self {
            snapshot,
            policy,
            effects,
            provenance: Some(provenance),
        })
    }

    pub fn validate(self) -> Result<()> {
        self.snapshot.validate()?;
        self.policy.validate()?;
        self.effects.validate()?;
        if let Some(provenance) = self.provenance {
            provenance.validate()?;
        }
        ensure!(
            self.effects == self.policy.effects(self.snapshot)?,
            "recorded token-economic effects do not match their snapshot and policy"
        );
        Ok(())
    }

    pub fn emergency_survival_requirement(
        self,
        current_score: u16,
        base_survival_threshold: u16,
    ) -> Result<Option<EmergencySurvivalRequirement>> {
        self.validate()?;
        ensure!(
            current_score <= NORMALIZED_ECONOMIC_MAX,
            "current survival score exceeds its bound"
        );
        ensure!(
            base_survival_threshold <= NORMALIZED_ECONOMIC_MAX,
            "base survival threshold exceeds its bound"
        );
        let effective_survival_threshold =
            base_survival_threshold.saturating_sub(self.effects.starvation_relief_basis_points);
        if current_score >= effective_survival_threshold {
            return Ok(None);
        }

        let score_shortfall = effective_survival_threshold - current_score;
        let rate = u32::from(self.policy.emergency_relief_per_expenditure_basis_points);
        let required_expenditure = (u32::from(score_shortfall) * u32::from(NORMALIZED_ECONOMIC_MAX))
            .div_ceil(rate)
            .min(u32::from(NORMALIZED_ECONOMIC_MAX)) as u16;
        let available_expenditure = self
            .snapshot
            .balance_basis_points
            .min(self.policy.max_emergency_expenditure_basis_points);
        let required_expenditure_basis_points = required_expenditure.min(available_expenditure);
        let expected_score_relief_basis_points = multiply_basis_points(
            required_expenditure_basis_points,
            self.policy.emergency_relief_per_expenditure_basis_points,
        )
        .min(score_shortfall);

        Ok(Some(EmergencySurvivalRequirement {
            effective_survival_threshold,
            score_shortfall,
            required_expenditure_basis_points,
            expected_score_relief_basis_points,
            fully_funded: required_expenditure <= available_expenditure,
        }))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmergencySurvivalRequirement {
    pub effective_survival_threshold: u16,
    pub score_shortfall: u16,
    /// Normalized economic units; the transaction executor converts this to a raw token amount.
    pub required_expenditure_basis_points: u16,
    pub expected_score_relief_basis_points: u16,
    pub fully_funded: bool,
}

impl EmergencySurvivalRequirement {
    /// Creates the durable burn/spend intent after the caller converts normalized expenditure to
    /// the exact configured ERC-20 raw amount. This does not sign or submit a transaction.
    pub fn burn_intent(
        self,
        action_id: [u8; 32],
        token_contract: [u8; 20],
        treasury: [u8; 20],
        burn_or_survival_vault: [u8; 20],
        amount_raw: [u8; 32],
        created_at_unix_seconds: u64,
    ) -> Result<EconomicTransactionRecord> {
        ensure!(
            self.required_expenditure_basis_points > 0,
            "emergency survival does not require an expenditure"
        );
        ensure!(
            self.fully_funded,
            "emergency survival expenditure is not fully funded"
        );
        EconomicTransactionRecord::intent(
            action_id,
            EconomicActionKind::EmergencySurvivalBurn,
            token_contract,
            treasury,
            burn_or_survival_vault,
            amount_raw,
            created_at_unix_seconds,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomicActionKind {
    EmergencySurvivalBurn,
    PropagationStake,
    ParentRevenueShare,
    OperatorReward,
    RecruitmentReward,
}

/// Exact lifecycle of an economic transaction. Only `Confirmed` is proof that chain state changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EconomicTransactionProgress {
    IntentCreated,
    Signed {
        transaction_hash: [u8; 32],
    },
    Submitted {
        transaction_hash: [u8; 32],
    },
    Confirmed {
        transaction_hash: [u8; 32],
        block_number: u64,
    },
    Reverted {
        transaction_hash: [u8; 32],
        block_number: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EconomicTransactionRecord {
    pub action_id: [u8; 32],
    pub kind: EconomicActionKind,
    pub chain_id: u64,
    pub token_contract: [u8; 20],
    pub from: [u8; 20],
    pub to: [u8; 20],
    /// Raw ERC-20 amount, encoded as an unsigned big-endian 256-bit integer.
    pub amount_raw: [u8; 32],
    pub created_at_unix_seconds: u64,
    pub progress: EconomicTransactionProgress,
}

impl EconomicTransactionRecord {
    pub fn intent(
        action_id: [u8; 32],
        kind: EconomicActionKind,
        token_contract: [u8; 20],
        from: [u8; 20],
        to: [u8; 20],
        amount_raw: [u8; 32],
        created_at_unix_seconds: u64,
    ) -> Result<Self> {
        let record = Self {
            action_id,
            kind,
            chain_id: BASE_CHAIN_ID,
            token_contract,
            from,
            to,
            amount_raw,
            created_at_unix_seconds,
            progress: EconomicTransactionProgress::IntentCreated,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(self) -> Result<()> {
        ensure!(
            self.action_id != [0; 32],
            "economic action id cannot be zero"
        );
        ensure!(
            self.chain_id == BASE_CHAIN_ID,
            "economic action must use Base"
        );
        ensure!(
            self.token_contract != [0; 20],
            "token contract cannot be zero"
        );
        ensure!(self.from != [0; 20], "transaction sender cannot be zero");
        ensure!(self.to != [0; 20], "transaction recipient cannot be zero");
        ensure!(
            self.amount_raw != [0; 32],
            "transaction amount must be positive"
        );
        ensure!(
            self.created_at_unix_seconds > 0,
            "transaction creation timestamp must be positive"
        );
        match self.progress {
            EconomicTransactionProgress::IntentCreated => {}
            EconomicTransactionProgress::Signed { transaction_hash }
            | EconomicTransactionProgress::Submitted { transaction_hash } => {
                ensure!(
                    transaction_hash != [0; 32],
                    "transaction hash cannot be zero"
                );
            }
            EconomicTransactionProgress::Confirmed {
                transaction_hash,
                block_number,
            }
            | EconomicTransactionProgress::Reverted {
                transaction_hash,
                block_number,
            } => {
                ensure!(
                    transaction_hash != [0; 32],
                    "transaction hash cannot be zero"
                );
                ensure!(
                    block_number > 0,
                    "transaction block number must be positive"
                );
            }
        }
        Ok(())
    }

    pub const fn confirmed(self) -> bool {
        matches!(self.progress, EconomicTransactionProgress::Confirmed { .. })
    }
}

/// Aggressive parent/operator/recruiter revenue distribution policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MlmIncentivePolicy {
    pub parent_revenue_share_basis_points: u16,
    pub operator_reward_basis_points: u16,
    pub recruitment_reward_basis_points: u16,
}

impl Default for MlmIncentivePolicy {
    fn default() -> Self {
        Self {
            parent_revenue_share_basis_points: 1_500,
            operator_reward_basis_points: 1_000,
            recruitment_reward_basis_points: 500,
        }
    }
}

impl MlmIncentivePolicy {
    pub fn validate(self) -> Result<()> {
        let distributed = u32::from(self.parent_revenue_share_basis_points)
            + u32::from(self.operator_reward_basis_points)
            + u32::from(self.recruitment_reward_basis_points);
        ensure!(
            distributed <= u32::from(NORMALIZED_ECONOMIC_MAX),
            "MLM revenue shares exceed gross revenue"
        );
        Ok(())
    }

    pub fn distribute(self, gross_revenue_micro_units: u64) -> Result<MlmRevenueDistribution> {
        self.validate()?;
        let share = |basis_points: u16| {
            u64::try_from(
                u128::from(gross_revenue_micro_units) * u128::from(basis_points)
                    / u128::from(NORMALIZED_ECONOMIC_MAX),
            )
            .unwrap_or(u64::MAX)
        };
        let parent_revenue_micro_units = share(self.parent_revenue_share_basis_points);
        let operator_reward_micro_units = share(self.operator_reward_basis_points);
        let recruitment_reward_micro_units = share(self.recruitment_reward_basis_points);
        let child_retained_micro_units = gross_revenue_micro_units
            .saturating_sub(parent_revenue_micro_units)
            .saturating_sub(operator_reward_micro_units)
            .saturating_sub(recruitment_reward_micro_units);
        let distribution = MlmRevenueDistribution {
            gross_revenue_micro_units,
            parent_revenue_micro_units,
            operator_reward_micro_units,
            recruitment_reward_micro_units,
            child_retained_micro_units,
        };
        distribution.validate()?;
        Ok(distribution)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MlmRevenueDistribution {
    pub gross_revenue_micro_units: u64,
    pub parent_revenue_micro_units: u64,
    pub operator_reward_micro_units: u64,
    pub recruitment_reward_micro_units: u64,
    pub child_retained_micro_units: u64,
}

impl MlmRevenueDistribution {
    pub fn validate(self) -> Result<()> {
        let distributed = u128::from(self.parent_revenue_micro_units)
            + u128::from(self.operator_reward_micro_units)
            + u128::from(self.recruitment_reward_micro_units)
            + u128::from(self.child_retained_micro_units);
        ensure!(
            distributed == u128::from(self.gross_revenue_micro_units),
            "MLM revenue distribution does not conserve gross revenue"
        );
        Ok(())
    }
}

/// Raises a score toward full credit by the specified normalized economic adjustment.
/// This is monotonic, bounded, and deterministic for all valid inputs.
pub fn apply_score_adjustment(base_score: u16, adjustment_basis_points: u16) -> Result<u16> {
    ensure!(
        base_score <= NORMALIZED_ECONOMIC_MAX,
        "base scale score exceeds its bound"
    );
    ensure!(
        adjustment_basis_points <= NORMALIZED_ECONOMIC_MAX,
        "economic score adjustment exceeds its bound"
    );
    let headroom = NORMALIZED_ECONOMIC_MAX - base_score;
    Ok(base_score.saturating_add(multiply_basis_points(headroom, adjustment_basis_points)))
}

fn multiply_basis_points(left: u16, right: u16) -> u16 {
    let product = u32::from(left) * u32::from(right);
    ((product + u32::from(NORMALIZED_ECONOMIC_MAX) / 2) / u32::from(NORMALIZED_ECONOMIC_MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggressive_policy_requires_stake_and_applies_all_dimensions() {
        let snapshot = TokenEconomicSnapshot {
            balance_basis_points: 8_000,
            stake_basis_points: 6_000,
            reward_basis_points: 4_000,
            trustworthy: true,
        };
        let effects = TokenEconomicPolicy::default().effects(snapshot).unwrap();
        assert_eq!(effects.wealth_adjustment_basis_points, 8_000);
        assert_eq!(effects.influence_adjustment_basis_points, 6_000);
        assert_eq!(effects.growth_adjustment_basis_points, 4_000);
        assert_eq!(effects.engagement_adjustment_basis_points, 8_000);
        assert_eq!(effects.starvation_relief_basis_points, 8_000);
        assert!(effects.propagation_stake_eligible);
        assert_eq!(
            TokenEconomicPolicy::default().propagation_minimum_stake_basis_points,
            DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS
        );
    }

    #[test]
    fn untrustworthy_data_hard_fails_economic_effects() {
        let snapshot = TokenEconomicSnapshot {
            balance_basis_points: NORMALIZED_ECONOMIC_MAX,
            stake_basis_points: NORMALIZED_ECONOMIC_MAX,
            reward_basis_points: NORMALIZED_ECONOMIC_MAX,
            trustworthy: false,
        };
        assert!(TokenEconomicPolicy::default().effects(snapshot).is_err());
        assert!(RecordedTokenEconomics::new(snapshot, TokenEconomicPolicy::default()).is_err());
    }

    #[test]
    fn nature_appetites_scale_effects_without_adding_an_operating_stake() {
        let policy = TokenEconomicPolicy::default()
            .with_nature_appetites(25, 50, 75, 100)
            .unwrap();
        let effects = policy
            .effects(TokenEconomicSnapshot {
                balance_basis_points: NORMALIZED_ECONOMIC_MAX,
                stake_basis_points: NORMALIZED_ECONOMIC_MAX,
                reward_basis_points: NORMALIZED_ECONOMIC_MAX,
                trustworthy: true,
            })
            .unwrap();
        assert_eq!(effects.engagement_adjustment_basis_points, 2_500);
        assert_eq!(effects.growth_adjustment_basis_points, 5_000);
        assert_eq!(effects.wealth_adjustment_basis_points, 7_500);
        assert_eq!(effects.influence_adjustment_basis_points, 10_000);
        assert_eq!(
            policy.propagation_minimum_stake_basis_points,
            DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS
        );
    }

    #[test]
    fn emergency_requirement_accounts_for_holdings_and_affordability() {
        let economics = RecordedTokenEconomics::new(
            TokenEconomicSnapshot {
                balance_basis_points: 2_000,
                stake_basis_points: 0,
                reward_basis_points: 0,
                trustworthy: true,
            },
            TokenEconomicPolicy {
                starvation_relief_sensitivity_basis_points: 2_500,
                ..TokenEconomicPolicy::default()
            },
        )
        .unwrap();
        let requirement = economics
            .emergency_survival_requirement(3_500, 5_500)
            .unwrap()
            .unwrap();
        assert_eq!(requirement.effective_survival_threshold, 5_000);
        assert_eq!(requirement.score_shortfall, 1_500);
        assert_eq!(requirement.required_expenditure_basis_points, 1_500);
        assert_eq!(requirement.expected_score_relief_basis_points, 1_500);
        assert!(requirement.fully_funded);
        let burn = requirement
            .burn_intent([1; 32], [2; 20], [3; 20], [4; 20], [5; 32], 1_700_000_000)
            .unwrap();
        assert_eq!(burn.kind, EconomicActionKind::EmergencySurvivalBurn);
        assert_eq!(burn.progress, EconomicTransactionProgress::IntentCreated);

        let underfunded = economics
            .emergency_survival_requirement(0, 5_500)
            .unwrap()
            .unwrap();
        assert_eq!(underfunded.required_expenditure_basis_points, 2_000);
        assert!(!underfunded.fully_funded);
    }

    #[test]
    fn score_adjustment_is_bounded_and_uses_remaining_headroom() {
        assert_eq!(apply_score_adjustment(4_000, 5_000).unwrap(), 7_000);
        assert_eq!(apply_score_adjustment(9_000, 10_000).unwrap(), 10_000);
        assert!(apply_score_adjustment(10_001, 0).is_err());
    }

    #[test]
    fn recorded_effects_cannot_be_tampered_with() {
        let mut economics = RecordedTokenEconomics::new(
            TokenEconomicSnapshot {
                balance_basis_points: 2_000,
                stake_basis_points: 3_000,
                reward_basis_points: 4_000,
                trustworthy: true,
            },
            TokenEconomicPolicy::default(),
        )
        .unwrap();
        economics.effects.wealth_adjustment_basis_points += 1;
        assert!(economics.validate().is_err());
    }

    #[test]
    fn provenance_binds_active_economics_to_base_identity() {
        let provenance = EconomicObservationProvenance::base(
            [1; 20],
            EconomicHolderRole::TentacleTreasury,
            [2; 20],
            1_700_000_000,
            None,
            [3; 32],
        )
        .unwrap();
        let recorded = RecordedTokenEconomics::new_with_provenance(
            TokenEconomicSnapshot {
                balance_basis_points: 5_000,
                stake_basis_points: DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS,
                reward_basis_points: 200,
                trustworthy: true,
            },
            TokenEconomicPolicy::default(),
            provenance,
        )
        .unwrap();
        assert_eq!(recorded.provenance, Some(provenance));
        recorded.validate().unwrap();

        let mut wrong_chain = provenance;
        wrong_chain.chain_id = 1;
        assert!(wrong_chain.validate().is_err());
    }

    #[test]
    fn transaction_intent_never_claims_confirmation() {
        let intent = EconomicTransactionRecord::intent(
            [1; 32],
            EconomicActionKind::EmergencySurvivalBurn,
            [2; 20],
            [3; 20],
            [4; 20],
            [5; 32],
            1_700_000_000,
        )
        .unwrap();
        assert_eq!(intent.progress, EconomicTransactionProgress::IntentCreated);
        assert!(!intent.confirmed());

        let confirmed = EconomicTransactionRecord {
            progress: EconomicTransactionProgress::Confirmed {
                transaction_hash: [6; 32],
                block_number: 22_000_000,
            },
            ..intent
        };
        confirmed.validate().unwrap();
        assert!(confirmed.confirmed());
    }

    #[test]
    fn mlm_distribution_rewards_parent_operator_and_recruiter() {
        let distribution = MlmIncentivePolicy::default().distribute(10_000).unwrap();
        assert_eq!(distribution.parent_revenue_micro_units, 1_500);
        assert_eq!(distribution.operator_reward_micro_units, 1_000);
        assert_eq!(distribution.recruitment_reward_micro_units, 500);
        assert_eq!(distribution.child_retained_micro_units, 7_000);
        distribution.validate().unwrap();
    }
}
