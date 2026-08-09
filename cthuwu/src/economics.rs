//! Deterministic token-economic inputs and policy-derived effects.
//!
//! This module deliberately consumes normalized observations rather than RPC- or ERC-20-specific
//! types. Chain adapters remain responsible for deciding whether an observation is trustworthy and
//! for normalizing balances, stake, and earned rewards into basis points.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const TOKEN_ECONOMICS_SCHEMA_VERSION: u32 = 1;
pub const NORMALIZED_ECONOMIC_MAX: u16 = 10_000;

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

/// Local policy controlling how strongly token observations affect this Tentacle.
///
/// The default is intentionally permissive: all observed dimensions have full sensitivity, all
/// observed balance may be recommended for emergency survival, and no stake is required for
/// propagation. A caller may derive sensitivities from Nature or configure a future propagation
/// stake without introducing a minimum stake merely to run or interact with a Tentacle.
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
            propagation_minimum_stake_basis_points: 0,
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
        let trusted = snapshot.trustworthy;
        let trusted_effect = |value, sensitivity| {
            if trusted {
                multiply_basis_points(value, sensitivity)
            } else {
                0
            }
        };
        let wealth_adjustment_basis_points = trusted_effect(
            snapshot.balance_basis_points,
            self.wealth_sensitivity_basis_points,
        );
        let influence_adjustment_basis_points = trusted_effect(
            snapshot.stake_basis_points,
            self.influence_sensitivity_basis_points,
        );
        let growth_adjustment_basis_points = trusted_effect(
            snapshot.reward_basis_points,
            self.growth_sensitivity_basis_points,
        );
        let engagement_adjustment_basis_points = trusted_effect(
            snapshot.balance_basis_points,
            self.engagement_sensitivity_basis_points,
        );
        let starvation_relief_basis_points = trusted_effect(
            snapshot.balance_basis_points,
            self.starvation_relief_sensitivity_basis_points,
        );
        let propagation_stake_eligible = self.propagation_minimum_stake_basis_points == 0
            || (trusted
                && snapshot.stake_basis_points >= self.propagation_minimum_stake_basis_points);

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
}

impl RecordedTokenEconomics {
    pub fn new(snapshot: TokenEconomicSnapshot, policy: TokenEconomicPolicy) -> Result<Self> {
        let effects = policy.effects(snapshot)?;
        Ok(Self {
            snapshot,
            policy,
            effects,
        })
    }

    pub fn validate(self) -> Result<()> {
        self.snapshot.validate()?;
        self.policy.validate()?;
        self.effects.validate()?;
        ensure!(
            self.effects == self.policy.effects(self.snapshot)?,
            "recorded token-economic effects do not match their snapshot and policy"
        );
        Ok(())
    }

    pub fn emergency_survival_recommendation(
        self,
        current_score: u16,
        base_survival_threshold: u16,
    ) -> Result<Option<EmergencySurvivalRecommendation>> {
        self.validate()?;
        ensure!(
            current_score <= NORMALIZED_ECONOMIC_MAX,
            "current survival score exceeds its bound"
        );
        ensure!(
            base_survival_threshold <= NORMALIZED_ECONOMIC_MAX,
            "base survival threshold exceeds its bound"
        );
        if !self.snapshot.trustworthy {
            return Ok(None);
        }

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
        let recommended_expenditure_basis_points = required_expenditure.min(available_expenditure);
        let expected_score_relief_basis_points = multiply_basis_points(
            recommended_expenditure_basis_points,
            self.policy.emergency_relief_per_expenditure_basis_points,
        )
        .min(score_shortfall);

        Ok(Some(EmergencySurvivalRecommendation {
            effective_survival_threshold,
            score_shortfall,
            recommended_expenditure_basis_points,
            expected_score_relief_basis_points,
            fully_funded: required_expenditure <= available_expenditure,
        }))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmergencySurvivalRecommendation {
    pub effective_survival_threshold: u16,
    pub score_shortfall: u16,
    /// Normalized economic units; the transaction adapter converts this to a token amount.
    pub recommended_expenditure_basis_points: u16,
    pub expected_score_relief_basis_points: u16,
    pub fully_funded: bool,
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
    fn permissive_policy_has_no_stake_floor_and_applies_all_dimensions() {
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
    }

    #[test]
    fn untrustworthy_data_has_no_score_effect_and_only_zero_floor_is_eligible() {
        let snapshot = TokenEconomicSnapshot {
            balance_basis_points: NORMALIZED_ECONOMIC_MAX,
            stake_basis_points: NORMALIZED_ECONOMIC_MAX,
            reward_basis_points: NORMALIZED_ECONOMIC_MAX,
            trustworthy: false,
        };
        let permissive = TokenEconomicPolicy::default().effects(snapshot).unwrap();
        assert_eq!(permissive.wealth_adjustment_basis_points, 0);
        assert_eq!(permissive.influence_adjustment_basis_points, 0);
        assert!(permissive.propagation_stake_eligible);

        let gated = TokenEconomicPolicy {
            propagation_minimum_stake_basis_points: 1,
            ..TokenEconomicPolicy::default()
        }
        .effects(snapshot)
        .unwrap();
        assert!(!gated.propagation_stake_eligible);
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
        assert_eq!(policy.propagation_minimum_stake_basis_points, 0);
    }

    #[test]
    fn emergency_recommendation_accounts_for_holdings_and_affordability() {
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
        let recommendation = economics
            .emergency_survival_recommendation(3_500, 5_500)
            .unwrap()
            .unwrap();
        assert_eq!(recommendation.effective_survival_threshold, 5_000);
        assert_eq!(recommendation.score_shortfall, 1_500);
        assert_eq!(recommendation.recommended_expenditure_basis_points, 1_500);
        assert_eq!(recommendation.expected_score_relief_basis_points, 1_500);
        assert!(recommendation.fully_funded);

        let underfunded = economics
            .emergency_survival_recommendation(0, 5_500)
            .unwrap()
            .unwrap();
        assert_eq!(underfunded.recommended_expenditure_basis_points, 2_000);
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
}
