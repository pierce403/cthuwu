//! Versioned local measurements and deterministic, advisory lifecycle judgments.
//!
//! Acolyte recruitment is only a local observation. It never creates Council contribution credit,
//! changes governance weight, or grants a vote. Council adapters must apply their independent,
//! acknowledgement-backed credit and one-Cthulhu-one-vote rules.

use crate::personality::TentacleNature;
use crate::storage::{ensure_private_directory, restrict_file, sync_directory};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

pub const METRICS_SCHEMA_VERSION: u32 = 1;
pub const JUDGMENT_SCHEMA_VERSION: u32 = 1;
pub const JUDGMENT_POLICY_SCHEMA_VERSION: u32 = 1;
pub const EVOLUTION_HISTORY_SCHEMA_VERSION: u32 = 1;

/// Scores and weights are integer basis points so evaluation is deterministic.
pub const SCORE_MAX: u16 = 10_000;
pub const PROPAGATION_RIGHTS_MIN_SCORE: u16 = 8_000;
pub const SURVIVAL_MIN_SCORE: u16 = 5_500;
pub const STARVATION_WARNING_MIN_SCORE: u16 = 3_000;
pub const DAILY_PROPAGATION_MIN_CONVERSATIONS: u32 = 8;
pub const DAILY_PROPAGATION_MIN_RETURNING_CONVERSATIONS: u32 = 4;
pub const WEEKLY_PROPAGATION_MIN_CONVERSATIONS: u32 = 32;
pub const WEEKLY_PROPAGATION_MIN_RETURNING_CONVERSATIONS: u32 = 16;

const MAX_EVENT_COUNT: u32 = 1_000_000;
const MAX_CONVERSATION_DEPTH_SAMPLE: u32 = 1_000;
const MAX_RESPONSE_TIME_MS_SAMPLE: u64 = 86_400_000;
const MAX_NETWORK_CONTRIBUTION_POINTS: u64 = 1_000_000_000_000;
const MAX_SIBLING_INFLUENCE_POINTS: u64 = 1_000_000_000_000;
const MAX_REVENUE_MICRO_UNITS: u64 = 1_000_000_000_000_000;
const MAX_NATURE_ADJUSTMENT_STRESS_EVENTS: u32 = 10_000;
const STRESS_PENALTY_PER_EVENT: u16 = 100;
const MAX_STRESS_PENALTY: u16 = 1_000;
const MAX_METRICS_BYTES: u64 = 256 * 1024;
const MAX_HISTORY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_HISTORY_LINE_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationPeriod {
    Daily,
    Weekly,
}

impl EvaluationPeriod {
    pub const fn duration_seconds(self) -> i64 {
        match self {
            Self::Daily => 86_400,
            Self::Weekly => 7 * 86_400,
        }
    }

    const fn days(self) -> u64 {
        match self {
            Self::Daily => 1,
            Self::Weekly => 7,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngagementMetrics {
    pub conversations: u32,
    pub conversation_depth_total: u64,
    pub returning_conversations: u32,
    pub response_time_samples: u32,
    pub response_time_ms_total: u64,
}

impl EngagementMetrics {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.conversations <= MAX_EVENT_COUNT,
            "engagement conversation count exceeds its bound"
        );
        ensure!(
            self.returning_conversations <= self.conversations,
            "returning conversations cannot exceed all conversations"
        );
        ensure!(
            self.response_time_samples <= self.conversations,
            "response-time samples cannot exceed all conversations"
        );
        ensure!(
            self.conversation_depth_total
                <= u64::from(self.conversations) * u64::from(MAX_CONVERSATION_DEPTH_SAMPLE),
            "conversation depth total exceeds its bound"
        );
        ensure!(
            self.response_time_ms_total
                <= u64::from(self.response_time_samples) * MAX_RESPONSE_TIME_MS_SAMPLE,
            "response-time total exceeds its bound"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GrowthMetrics {
    pub children_spawned: u32,
    /// A local observation only; this is not Council credit and carries no governance weight.
    pub acolytes_recruited: u32,
    pub network_contribution_points: u64,
}

impl GrowthMetrics {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.children_spawned <= MAX_EVENT_COUNT,
            "children-spawned count exceeds its bound"
        );
        ensure!(
            self.acolytes_recruited <= MAX_EVENT_COUNT,
            "acolyte-recruitment count exceeds its bound"
        );
        ensure!(
            self.network_contribution_points <= MAX_NETWORK_CONTRIBUTION_POINTS,
            "network-contribution total exceeds its bound"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WealthMetrics {
    /// Abstract micro-units; the economic adapter defines the currency.
    pub revenue_micro_units: u64,
    pub efficiency_samples: u32,
    pub efficiency_basis_points_total: u64,
}

impl WealthMetrics {
    pub fn average_efficiency_basis_points(&self) -> u16 {
        if self.efficiency_samples == 0 {
            return 0;
        }
        let average = self.efficiency_basis_points_total / u64::from(self.efficiency_samples);
        u16::try_from(average.min(u64::from(SCORE_MAX))).unwrap_or(SCORE_MAX)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.revenue_micro_units <= MAX_REVENUE_MICRO_UNITS,
            "revenue total exceeds its bound"
        );
        ensure!(
            self.efficiency_samples <= MAX_EVENT_COUNT,
            "efficiency sample count exceeds its bound"
        );
        ensure!(
            self.efficiency_basis_points_total
                <= u64::from(self.efficiency_samples) * u64::from(SCORE_MAX),
            "efficiency total exceeds its bound"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InfluenceMetrics {
    pub governance_participation: u32,
    pub sibling_influence_points: u64,
}

impl InfluenceMetrics {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.governance_participation <= MAX_EVENT_COUNT,
            "governance-participation count exceeds its bound"
        );
        ensure!(
            self.sibling_influence_points <= MAX_SIBLING_INFLUENCE_POINTS,
            "sibling-influence total exceeds its bound"
        );
        Ok(())
    }
}

/// Declares which locally observable scales are eligible to affect a judgment.
///
/// A disabled scale may still retain its raw score for operator visibility, but its normalized
/// weight is always zero. This keeps unavailable adapters and locally prohibited dimensions from
/// lowering or raising lifecycle outcomes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScoredScaleAvailability {
    pub engagement: bool,
    pub growth: bool,
    pub wealth: bool,
    pub influence: bool,
}

impl ScoredScaleAvailability {
    pub const fn for_economic_layer(economic_layer_enabled: bool) -> Self {
        Self {
            engagement: true,
            growth: true,
            wealth: economic_layer_enabled,
            influence: true,
        }
    }

    fn active(self) -> [bool; 4] {
        [self.engagement, self.growth, self.wealth, self.influence]
    }

    fn is_restriction_of(self, previous: Self) -> bool {
        (!self.engagement || previous.engagement)
            && (!self.growth || previous.growth)
            && (!self.wealth || previous.wealth)
            && (!self.influence || previous.influence)
    }

    fn validate(self, economic_layer_enabled: bool) -> Result<()> {
        ensure!(
            self.active().into_iter().any(|active| active),
            "at least one scale must remain available for scoring"
        );
        ensure!(
            economic_layer_enabled || !self.wealth,
            "wealth cannot be scored while the economic layer is disabled"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TentacleMetrics {
    pub schema_version: u32,
    /// The exact signed Nature/awakening epoch under which every observation was collected.
    pub nature_id: String,
    pub nature_fingerprint: String,
    pub awakening_epoch: u64,
    pub period: EvaluationPeriod,
    pub period_started_at_unix_seconds: i64,
    pub period_ends_at_unix_seconds: i64,
    pub engagement: EngagementMetrics,
    pub growth: GrowthMetrics,
    /// `None` means the optional economic layer is disabled and wealth gets no weight.
    pub wealth: Option<WealthMetrics>,
    pub influence: InfluenceMetrics,
    /// Persisted policy identifying the dimensions that may affect this period's judgment.
    pub scored_scale_availability: ScoredScaleAvailability,
    /// Operator Nature adjustments are auditable and carry a small, capped evaluation penalty.
    pub nature_adjustment_stress_events: u32,
}

impl TentacleMetrics {
    pub fn new(
        period: EvaluationPeriod,
        period_started_at_unix_seconds: i64,
        economic_layer_enabled: bool,
        nature: &TentacleNature,
        awakening_epoch: u64,
    ) -> Result<Self> {
        nature.validate()?;
        ensure!(awakening_epoch > 0, "awakening epoch must be positive");
        let period_ends_at_unix_seconds = period_started_at_unix_seconds
            .checked_add(period.duration_seconds())
            .context("evaluation period end overflows its timestamp")?;
        Ok(Self {
            schema_version: METRICS_SCHEMA_VERSION,
            nature_id: nature.nature_id.clone(),
            nature_fingerprint: nature.fingerprint()?,
            awakening_epoch,
            period,
            period_started_at_unix_seconds,
            period_ends_at_unix_seconds,
            engagement: EngagementMetrics::default(),
            growth: GrowthMetrics::default(),
            wealth: economic_layer_enabled.then(WealthMetrics::default),
            influence: InfluenceMetrics::default(),
            scored_scale_availability: ScoredScaleAvailability::for_economic_layer(
                economic_layer_enabled,
            ),
            nature_adjustment_stress_events: 0,
        })
    }

    pub fn economic_layer_enabled(&self) -> bool {
        self.wealth.is_some()
    }

    /// Permanently narrows which scales can affect this evaluation period.
    ///
    /// Re-enabling a scale requires starting a new period so a later policy change cannot make
    /// already-collected observations outcome-bearing retroactively.
    pub fn restrict_scored_scales(&mut self, availability: ScoredScaleAvailability) -> Result<()> {
        availability.validate(self.economic_layer_enabled())?;
        ensure!(
            availability.is_restriction_of(self.scored_scale_availability),
            "scored-scale availability can only be restricted within an evaluation period"
        );
        self.scored_scale_availability = availability;
        Ok(())
    }

    /// True when rebinding can preserve only the explicit Nature-adjustment stress counter and
    /// cannot misattribute any behavioral/economic observation to another Nature.
    pub fn has_behavior_observations(&self) -> bool {
        self.engagement != EngagementMetrics::default()
            || self.growth != GrowthMetrics::default()
            || self.influence != InfluenceMetrics::default()
            || self
                .wealth
                .as_ref()
                .is_some_and(|wealth| wealth != &WealthMetrics::default())
    }

    pub fn rebind_empty_period(
        &mut self,
        nature: &TentacleNature,
        awakening_epoch: u64,
    ) -> Result<()> {
        ensure!(
            !self.has_behavior_observations(),
            "an observed metrics period cannot be rebound to another Nature"
        );
        nature.validate()?;
        ensure!(awakening_epoch > 0, "awakening epoch must be positive");
        self.nature_id = nature.nature_id.clone();
        self.nature_fingerprint = nature.fingerprint()?;
        self.awakening_epoch = awakening_epoch;
        self.validate()
    }

    /// Adds one bounded observation. Once the event cap is reached, later observations are ignored.
    pub fn record_conversation(
        &mut self,
        conversation_depth: u32,
        returning: bool,
        response_time_ms: Option<u64>,
    ) {
        if self.engagement.conversations == MAX_EVENT_COUNT {
            return;
        }
        self.engagement.conversations += 1;
        self.engagement.conversation_depth_total = self
            .engagement
            .conversation_depth_total
            .saturating_add(u64::from(
                conversation_depth.min(MAX_CONVERSATION_DEPTH_SAMPLE),
            ));
        if returning {
            self.engagement.returning_conversations += 1;
        }
        if let Some(response_time_ms) = response_time_ms {
            self.engagement.response_time_samples += 1;
            self.engagement.response_time_ms_total = self
                .engagement
                .response_time_ms_total
                .saturating_add(response_time_ms.min(MAX_RESPONSE_TIME_MS_SAMPLE));
        }
    }

    /// Records independent local growth observations. Recruitment does not increment contribution
    /// points and this module has no API that mutates Council credit or governance votes.
    pub fn record_growth(
        &mut self,
        children_spawned: u32,
        acolytes_recruited: u32,
        network_contribution_points: u64,
    ) {
        self.growth.children_spawned = self
            .growth
            .children_spawned
            .saturating_add(children_spawned)
            .min(MAX_EVENT_COUNT);
        self.growth.acolytes_recruited = self
            .growth
            .acolytes_recruited
            .saturating_add(acolytes_recruited)
            .min(MAX_EVENT_COUNT);
        self.growth.network_contribution_points = self
            .growth
            .network_contribution_points
            .saturating_add(network_contribution_points)
            .min(MAX_NETWORK_CONTRIBUTION_POINTS);
    }

    pub fn record_economic_result(
        &mut self,
        revenue_micro_units: u64,
        efficiency_basis_points: u16,
    ) -> Result<()> {
        let wealth = self
            .wealth
            .as_mut()
            .context("the optional economic layer is disabled")?;
        if wealth.efficiency_samples == MAX_EVENT_COUNT {
            return Ok(());
        }
        wealth.revenue_micro_units = wealth
            .revenue_micro_units
            .saturating_add(revenue_micro_units)
            .min(MAX_REVENUE_MICRO_UNITS);
        wealth.efficiency_samples += 1;
        wealth.efficiency_basis_points_total = wealth
            .efficiency_basis_points_total
            .saturating_add(u64::from(efficiency_basis_points.min(SCORE_MAX)));
        Ok(())
    }

    pub fn record_influence(
        &mut self,
        governance_participation: u32,
        sibling_influence_points: u64,
    ) {
        self.influence.governance_participation = self
            .influence
            .governance_participation
            .saturating_add(governance_participation)
            .min(MAX_EVENT_COUNT);
        self.influence.sibling_influence_points = self
            .influence
            .sibling_influence_points
            .saturating_add(sibling_influence_points)
            .min(MAX_SIBLING_INFLUENCE_POINTS);
    }

    /// Records one successful operator `/adjust`; rejected adjustments must not call this method.
    pub fn record_nature_adjustment_stress(&mut self) {
        self.nature_adjustment_stress_events = self
            .nature_adjustment_stress_events
            .saturating_add(1)
            .min(MAX_NATURE_ADJUSTMENT_STRESS_EVENTS);
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == METRICS_SCHEMA_VERSION,
            "unsupported metrics schema version {}",
            self.schema_version
        );
        ensure!(
            self.nature_id.len() == 32
                && self
                    .nature_id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "metrics Nature ID is invalid"
        );
        ensure!(
            self.nature_fingerprint.len() == 64
                && self
                    .nature_fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "metrics Nature fingerprint is invalid"
        );
        ensure!(
            self.awakening_epoch > 0,
            "metrics awakening epoch is invalid"
        );
        ensure!(
            self.period_started_at_unix_seconds
                .checked_add(self.period.duration_seconds())
                == Some(self.period_ends_at_unix_seconds),
            "metrics evaluation period has invalid bounds"
        );
        self.engagement.validate()?;
        self.growth.validate()?;
        if let Some(wealth) = &self.wealth {
            wealth.validate()?;
        }
        self.influence.validate()?;
        self.scored_scale_availability
            .validate(self.economic_layer_enabled())?;
        ensure!(
            self.nature_adjustment_stress_events <= MAX_NATURE_ADJUSTMENT_STRESS_EVENTS,
            "Nature-adjustment stress count exceeds its bound"
        );
        Ok(())
    }

    pub fn evaluate(
        &self,
        nature: &TentacleNature,
        evaluated_at_unix_seconds: i64,
    ) -> Result<Judgment> {
        self.evaluate_with_policy(
            nature,
            evaluated_at_unix_seconds,
            JudgmentPolicy::for_period(self.period),
        )
    }

    /// Returns a clearly labeled, non-actionable view of an evaluation period still in progress.
    /// Authenticated operator routing remains an integration responsibility.
    pub fn evaluate_snapshot(
        &self,
        nature: &TentacleNature,
        evaluated_at_unix_seconds: i64,
    ) -> Result<Judgment> {
        self.evaluate_snapshot_with_policy(
            nature,
            evaluated_at_unix_seconds,
            JudgmentPolicy::for_period(self.period),
        )
    }

    pub fn evaluate_with_policy(
        &self,
        nature: &TentacleNature,
        evaluated_at_unix_seconds: i64,
        policy: JudgmentPolicy,
    ) -> Result<Judgment> {
        self.evaluate_internal(
            nature,
            evaluated_at_unix_seconds,
            policy,
            EvaluationStatus::Final,
        )
    }

    pub fn evaluate_snapshot_with_policy(
        &self,
        nature: &TentacleNature,
        evaluated_at_unix_seconds: i64,
        policy: JudgmentPolicy,
    ) -> Result<Judgment> {
        self.evaluate_internal(
            nature,
            evaluated_at_unix_seconds,
            policy,
            EvaluationStatus::PartialSnapshot,
        )
    }

    fn evaluate_internal(
        &self,
        nature: &TentacleNature,
        evaluated_at_unix_seconds: i64,
        policy: JudgmentPolicy,
        evaluation_status: EvaluationStatus,
    ) -> Result<Judgment> {
        self.validate()?;
        nature.validate()?;
        ensure!(
            self.nature_id == nature.nature_id
                && self.nature_fingerprint == nature.fingerprint()?,
            "metrics are bound to a different signed Nature"
        );
        policy.validate()?;
        ensure!(
            policy.period == self.period,
            "judgment policy and metrics use different evaluation periods"
        );
        match evaluation_status {
            EvaluationStatus::Final => ensure!(
                evaluated_at_unix_seconds >= self.period_ends_at_unix_seconds,
                "cannot finalize an evaluation period before it closes"
            ),
            EvaluationStatus::PartialSnapshot => ensure!(
                (self.period_started_at_unix_seconds..self.period_ends_at_unix_seconds)
                    .contains(&evaluated_at_unix_seconds),
                "a partial snapshot must fall within its open evaluation period"
            ),
        }

        let weights = ScaleWeights::from_nature_with_availability(
            nature,
            self.scored_scale_availability,
            self.economic_layer_enabled(),
        )?;
        let scores = score_metrics(self, &policy.targets, weights)?;
        let propagation_evidence = PropagationEvidence::from_metrics(self, &policy);
        let score_outcome = policy.thresholds.classify(scores.total)?;
        let outcome = if score_outcome == JudgmentOutcome::PropagationRights
            && !propagation_evidence.eligible
        {
            JudgmentOutcome::Survival
        } else {
            score_outcome
        };
        Ok(Judgment {
            schema_version: JUDGMENT_SCHEMA_VERSION,
            metrics_schema_version: self.schema_version,
            period: self.period,
            period_started_at_unix_seconds: self.period_started_at_unix_seconds,
            period_ends_at_unix_seconds: self.period_ends_at_unix_seconds,
            evaluated_at_unix_seconds,
            evaluation_status,
            policy,
            scored_scale_availability: self.scored_scale_availability,
            weights,
            scores,
            propagation_evidence,
            outcome,
            execution: match evaluation_status {
                EvaluationStatus::Final => {
                    DecisionExecution::AuthenticatedOperatorConfirmationRequired
                }
                EvaluationStatus::PartialSnapshot => DecisionExecution::AdvisorySnapshotOnly,
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetricTargets {
    pub average_conversation_depth: u32,
    pub response_time_full_credit_ms: u64,
    pub response_time_zero_credit_ms: u64,
    pub children_spawned: u32,
    pub acolytes_recruited: u32,
    pub network_contribution_points: u64,
    pub revenue_micro_units: u64,
    pub efficiency_basis_points: u16,
    pub governance_participation: u32,
    pub sibling_influence_points: u64,
}

impl MetricTargets {
    pub fn for_period(period: EvaluationPeriod) -> Self {
        let days = period.days();
        Self {
            average_conversation_depth: 8,
            response_time_full_credit_ms: 3_000,
            response_time_zero_credit_ms: 120_000,
            children_spawned: u32::try_from(days).unwrap_or(u32::MAX),
            acolytes_recruited: u32::try_from(2 * days).unwrap_or(u32::MAX),
            network_contribution_points: 100 * days,
            revenue_micro_units: 1_000_000 * days,
            efficiency_basis_points: 8_000,
            governance_participation: u32::try_from(days).unwrap_or(u32::MAX),
            sibling_influence_points: 100 * days,
        }
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.average_conversation_depth > 0
                && self.average_conversation_depth <= MAX_CONVERSATION_DEPTH_SAMPLE,
            "average-conversation-depth target is invalid"
        );
        ensure!(
            self.response_time_full_credit_ms < self.response_time_zero_credit_ms
                && self.response_time_zero_credit_ms <= MAX_RESPONSE_TIME_MS_SAMPLE,
            "response-time targets are invalid"
        );
        ensure!(
            self.children_spawned > 0,
            "children target must be positive"
        );
        ensure!(
            self.acolytes_recruited > 0,
            "acolyte target must be positive"
        );
        ensure!(
            self.network_contribution_points > 0,
            "network-contribution target must be positive"
        );
        ensure!(
            self.revenue_micro_units > 0,
            "revenue target must be positive"
        );
        ensure!(
            self.efficiency_basis_points > 0 && self.efficiency_basis_points <= SCORE_MAX,
            "efficiency target is invalid"
        );
        ensure!(
            self.governance_participation > 0,
            "governance target must be positive"
        );
        ensure!(
            self.sibling_influence_points > 0,
            "sibling-influence target must be positive"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JudgmentThresholds {
    pub propagation_rights_min: u16,
    pub survival_min: u16,
    pub starvation_warning_min: u16,
}

impl Default for JudgmentThresholds {
    fn default() -> Self {
        Self {
            propagation_rights_min: PROPAGATION_RIGHTS_MIN_SCORE,
            survival_min: SURVIVAL_MIN_SCORE,
            starvation_warning_min: STARVATION_WARNING_MIN_SCORE,
        }
    }
}

impl JudgmentThresholds {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.propagation_rights_min <= SCORE_MAX
                && self.propagation_rights_min > self.survival_min
                && self.survival_min > self.starvation_warning_min,
            "judgment thresholds must be strictly descending and within the score range"
        );
        Ok(())
    }

    pub fn classify(&self, score: u16) -> Result<JudgmentOutcome> {
        self.validate()?;
        ensure!(score <= SCORE_MAX, "judgment score exceeds its bound");
        Ok(if score >= self.propagation_rights_min {
            JudgmentOutcome::PropagationRights
        } else if score >= self.survival_min {
            JudgmentOutcome::Survival
        } else if score >= self.starvation_warning_min {
            JudgmentOutcome::StarvationWarning
        } else {
            JudgmentOutcome::Death
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JudgmentPolicy {
    pub schema_version: u32,
    pub period: EvaluationPeriod,
    pub targets: MetricTargets,
    pub thresholds: JudgmentThresholds,
    pub propagation_min_conversations: u32,
    pub propagation_min_returning_conversations: u32,
}

impl JudgmentPolicy {
    pub fn for_period(period: EvaluationPeriod) -> Self {
        let (propagation_min_conversations, propagation_min_returning_conversations) = match period
        {
            EvaluationPeriod::Daily => (
                DAILY_PROPAGATION_MIN_CONVERSATIONS,
                DAILY_PROPAGATION_MIN_RETURNING_CONVERSATIONS,
            ),
            EvaluationPeriod::Weekly => (
                WEEKLY_PROPAGATION_MIN_CONVERSATIONS,
                WEEKLY_PROPAGATION_MIN_RETURNING_CONVERSATIONS,
            ),
        };
        Self {
            schema_version: JUDGMENT_POLICY_SCHEMA_VERSION,
            period,
            targets: MetricTargets::for_period(period),
            thresholds: JudgmentThresholds::default(),
            propagation_min_conversations,
            propagation_min_returning_conversations,
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == JUDGMENT_POLICY_SCHEMA_VERSION,
            "unsupported judgment-policy schema version {}",
            self.schema_version
        );
        self.targets.validate()?;
        self.thresholds.validate()?;
        ensure!(
            self.propagation_min_conversations > 0
                && self.propagation_min_conversations <= MAX_EVENT_COUNT,
            "propagation conversation evidence floor is invalid"
        );
        ensure!(
            self.propagation_min_returning_conversations > 0
                && self.propagation_min_returning_conversations
                    <= self.propagation_min_conversations,
            "propagation returning-conversation evidence floor is invalid"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScaleWeights {
    pub engagement: u16,
    pub growth: u16,
    pub wealth: u16,
    pub influence: u16,
}

impl ScaleWeights {
    pub fn from_nature(nature: &TentacleNature, economic_layer_enabled: bool) -> Result<Self> {
        Self::from_nature_with_availability(
            nature,
            ScoredScaleAvailability::for_economic_layer(economic_layer_enabled),
            economic_layer_enabled,
        )
    }

    fn from_nature_with_availability(
        nature: &TentacleNature,
        availability: ScoredScaleAvailability,
        economic_layer_enabled: bool,
    ) -> Result<Self> {
        nature.validate()?;
        availability.validate(economic_layer_enabled)?;
        let values = [
            u64::from(nature.engagement),
            u64::from(nature.growth),
            u64::from(nature.wealth),
            u64::from(nature.influence),
        ];
        let active = availability.active();
        let normalized = normalize_weights(values, active);
        let weights = Self {
            engagement: normalized[0],
            growth: normalized[1],
            wealth: normalized[2],
            influence: normalized[3],
        };
        weights.validate(availability, economic_layer_enabled)?;
        Ok(weights)
    }

    pub fn total(self) -> u16 {
        self.engagement
            .saturating_add(self.growth)
            .saturating_add(self.wealth)
            .saturating_add(self.influence)
    }

    fn validate(
        self,
        availability: ScoredScaleAvailability,
        economic_layer_enabled: bool,
    ) -> Result<()> {
        availability.validate(economic_layer_enabled)?;
        ensure!(
            self.total() == SCORE_MAX,
            "scale weights must total {SCORE_MAX} basis points"
        );
        ensure!(
            economic_layer_enabled || self.wealth == 0,
            "wealth cannot be weighted while the economic layer is disabled"
        );
        for (name, weight, active) in [
            ("engagement", self.engagement, availability.engagement),
            ("growth", self.growth, availability.growth),
            ("wealth", self.wealth, availability.wealth),
            ("influence", self.influence, availability.influence),
        ] {
            ensure!(
                active || weight == 0,
                "unavailable {name} scale must have zero weight"
            );
        }
        Ok(())
    }
}

fn normalize_weights(values: [u64; 4], active: [bool; 4]) -> [u16; 4] {
    let mut result = [0_u16; 4];
    let active_count = active.iter().filter(|enabled| **enabled).count() as u16;
    debug_assert!(active_count > 0);
    let total: u64 = values
        .iter()
        .zip(active)
        .filter_map(|(value, enabled)| enabled.then_some(*value))
        .sum();

    if total == 0 {
        let base = SCORE_MAX / active_count;
        let mut remainder = SCORE_MAX % active_count;
        for (index, enabled) in active.into_iter().enumerate() {
            if enabled {
                result[index] = base;
                if remainder > 0 {
                    result[index] += 1;
                    remainder -= 1;
                }
            }
        }
        return result;
    }

    let mut fractional_remainders = [0_u64; 4];
    let mut assigned = 0_u16;
    for index in 0..4 {
        if active[index] {
            let scaled = values[index] * u64::from(SCORE_MAX);
            result[index] = u16::try_from(scaled / total).unwrap_or(SCORE_MAX);
            fractional_remainders[index] = scaled % total;
            assigned += result[index];
        }
    }

    let mut remainder = SCORE_MAX - assigned;
    let mut awarded = [false; 4];
    while remainder > 0 {
        let Some(index) = (0..4)
            .filter(|index| active[*index] && !awarded[*index])
            .max_by_key(|index| (fractional_remainders[*index], std::cmp::Reverse(*index)))
        else {
            break;
        };
        result[index] += 1;
        awarded[index] = true;
        remainder -= 1;
    }
    result
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScaleScores {
    pub engagement: u16,
    pub growth: u16,
    pub wealth: Option<u16>,
    pub influence: u16,
    pub weighted_total_before_stress: u16,
    pub stress_penalty: u16,
    pub total: u16,
}

impl ScaleScores {
    fn validate(self) -> Result<()> {
        for (name, score) in [
            ("engagement", self.engagement),
            ("growth", self.growth),
            ("influence", self.influence),
            (
                "weighted total before stress",
                self.weighted_total_before_stress,
            ),
            ("stress penalty", self.stress_penalty),
            ("total", self.total),
        ] {
            ensure!(score <= SCORE_MAX, "{name} score exceeds its bound");
        }
        if let Some(wealth) = self.wealth {
            ensure!(wealth <= SCORE_MAX, "wealth score exceeds its bound");
        }
        ensure!(
            self.stress_penalty <= MAX_STRESS_PENALTY,
            "stress penalty exceeds its bound"
        );
        ensure!(
            self.total
                == self
                    .weighted_total_before_stress
                    .saturating_sub(self.stress_penalty),
            "final score does not apply its recorded stress penalty"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationStatus {
    PartialSnapshot,
    Final,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgmentOutcome {
    PropagationRights,
    Survival,
    StarvationWarning,
    Death,
}

/// The scales only recommend outcomes. Applying any outcome belongs to an authenticated operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionExecution {
    AdvisorySnapshotOnly,
    AuthenticatedOperatorConfirmationRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropagationEvidence {
    pub observed_conversations: u32,
    pub observed_returning_conversations: u32,
    pub required_conversations: u32,
    pub required_returning_conversations: u32,
    pub eligible: bool,
}

impl PropagationEvidence {
    fn from_metrics(metrics: &TentacleMetrics, policy: &JudgmentPolicy) -> Self {
        let observed_conversations = metrics.engagement.conversations;
        let observed_returning_conversations = metrics.engagement.returning_conversations;
        let required_conversations = policy.propagation_min_conversations;
        let required_returning_conversations = policy.propagation_min_returning_conversations;
        Self {
            observed_conversations,
            observed_returning_conversations,
            required_conversations,
            required_returning_conversations,
            eligible: observed_conversations >= required_conversations
                && observed_returning_conversations >= required_returning_conversations,
        }
    }

    fn validate(&self, policy: &JudgmentPolicy) -> Result<()> {
        ensure!(
            self.observed_returning_conversations <= self.observed_conversations,
            "propagation evidence has more returns than conversations"
        );
        ensure!(
            self.required_conversations == policy.propagation_min_conversations
                && self.required_returning_conversations
                    == policy.propagation_min_returning_conversations,
            "propagation evidence disagrees with its judgment policy"
        );
        ensure!(
            self.eligible
                == (self.observed_conversations >= self.required_conversations
                    && self.observed_returning_conversations
                        >= self.required_returning_conversations),
            "propagation evidence eligibility is inconsistent"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Judgment {
    pub schema_version: u32,
    pub metrics_schema_version: u32,
    pub period: EvaluationPeriod,
    pub period_started_at_unix_seconds: i64,
    pub period_ends_at_unix_seconds: i64,
    pub evaluated_at_unix_seconds: i64,
    pub evaluation_status: EvaluationStatus,
    pub policy: JudgmentPolicy,
    /// The persisted eligibility policy used to normalize this judgment's Nature weights.
    pub scored_scale_availability: ScoredScaleAvailability,
    pub weights: ScaleWeights,
    pub scores: ScaleScores,
    pub propagation_evidence: PropagationEvidence,
    pub outcome: JudgmentOutcome,
    pub execution: DecisionExecution,
}

impl Judgment {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == JUDGMENT_SCHEMA_VERSION,
            "unsupported judgment schema version {}",
            self.schema_version
        );
        ensure!(
            self.metrics_schema_version == METRICS_SCHEMA_VERSION,
            "unsupported metrics schema referenced by judgment"
        );
        ensure!(
            self.period_started_at_unix_seconds
                .checked_add(self.period.duration_seconds())
                == Some(self.period_ends_at_unix_seconds),
            "judgment evaluation period has invalid bounds"
        );
        match self.evaluation_status {
            EvaluationStatus::Final => {
                ensure!(
                    self.evaluated_at_unix_seconds >= self.period_ends_at_unix_seconds,
                    "final judgment predates the end of its evaluation period"
                );
                ensure!(
                    self.execution == DecisionExecution::AuthenticatedOperatorConfirmationRequired,
                    "final judgment must require authenticated operator confirmation"
                );
            }
            EvaluationStatus::PartialSnapshot => {
                ensure!(
                    (self.period_started_at_unix_seconds..self.period_ends_at_unix_seconds)
                        .contains(&self.evaluated_at_unix_seconds),
                    "partial judgment must fall within its open evaluation period"
                );
                ensure!(
                    self.execution == DecisionExecution::AdvisorySnapshotOnly,
                    "partial judgment must remain advisory"
                );
            }
        }
        ensure!(
            self.policy.period == self.period,
            "judgment policy and judgment use different periods"
        );
        self.policy.validate()?;
        self.scored_scale_availability
            .validate(self.scores.wealth.is_some())?;
        self.weights
            .validate(self.scored_scale_availability, self.scores.wealth.is_some())?;
        self.scores.validate()?;
        self.propagation_evidence.validate(&self.policy)?;
        let score_outcome = self.policy.thresholds.classify(self.scores.total)?;
        let expected_outcome = if score_outcome == JudgmentOutcome::PropagationRights
            && !self.propagation_evidence.eligible
        {
            JudgmentOutcome::Survival
        } else {
            score_outcome
        };
        ensure!(
            self.outcome == expected_outcome,
            "judgment outcome does not match its score, thresholds, and evidence floor"
        );
        Ok(())
    }

    pub fn recommends_death(&self) -> bool {
        self.evaluation_status == EvaluationStatus::Final && self.outcome == JudgmentOutcome::Death
    }
}

fn score_metrics(
    metrics: &TentacleMetrics,
    targets: &MetricTargets,
    weights: ScaleWeights,
) -> Result<ScaleScores> {
    targets.validate()?;
    let engagement = score_engagement(&metrics.engagement, targets);
    let growth = score_growth(&metrics.growth, targets);
    let wealth = metrics
        .wealth
        .as_ref()
        .map(|wealth| score_wealth(wealth, targets));
    let influence = score_influence(&metrics.influence, targets);
    let weighted_sum = u64::from(engagement) * u64::from(weights.engagement)
        + u64::from(growth) * u64::from(weights.growth)
        + u64::from(wealth.unwrap_or(0)) * u64::from(weights.wealth)
        + u64::from(influence) * u64::from(weights.influence);
    let weighted_total_before_stress =
        u16::try_from((weighted_sum + u64::from(SCORE_MAX) / 2) / u64::from(SCORE_MAX))
            .unwrap_or(SCORE_MAX)
            .min(SCORE_MAX);
    let stress_penalty = u16::try_from(metrics.nature_adjustment_stress_events)
        .unwrap_or(u16::MAX)
        .saturating_mul(STRESS_PENALTY_PER_EVENT)
        .min(MAX_STRESS_PENALTY);
    let total = weighted_total_before_stress.saturating_sub(stress_penalty);
    let scores = ScaleScores {
        engagement,
        growth,
        wealth,
        influence,
        weighted_total_before_stress,
        stress_penalty,
        total,
    };
    scores.validate()?;
    Ok(scores)
}

fn score_engagement(metrics: &EngagementMetrics, targets: &MetricTargets) -> u16 {
    let conversations = u128::from(metrics.conversations);
    let depth = ratio_score(
        u128::from(metrics.conversation_depth_total),
        conversations * u128::from(targets.average_conversation_depth),
    );
    let returns = ratio_score(u128::from(metrics.returning_conversations), conversations);
    let response_time = response_time_score(
        metrics.response_time_ms_total,
        metrics.response_time_samples,
        targets.response_time_full_credit_ms,
        targets.response_time_zero_credit_ms,
    );
    component_score([(depth, 4_000), (returns, 3_500), (response_time, 2_500)])
}

fn score_growth(metrics: &GrowthMetrics, targets: &MetricTargets) -> u16 {
    component_score([
        (
            ratio_score(
                u128::from(metrics.children_spawned),
                u128::from(targets.children_spawned),
            ),
            3_000,
        ),
        (
            ratio_score(
                u128::from(metrics.acolytes_recruited),
                u128::from(targets.acolytes_recruited),
            ),
            3_000,
        ),
        (
            ratio_score(
                u128::from(metrics.network_contribution_points),
                u128::from(targets.network_contribution_points),
            ),
            4_000,
        ),
    ])
}

fn score_wealth(metrics: &WealthMetrics, targets: &MetricTargets) -> u16 {
    component_score([
        (
            ratio_score(
                u128::from(metrics.revenue_micro_units),
                u128::from(targets.revenue_micro_units),
            ),
            6_000,
        ),
        (
            ratio_score(
                u128::from(metrics.average_efficiency_basis_points()),
                u128::from(targets.efficiency_basis_points),
            ),
            4_000,
        ),
    ])
}

fn score_influence(metrics: &InfluenceMetrics, targets: &MetricTargets) -> u16 {
    component_score([
        (
            ratio_score(
                u128::from(metrics.governance_participation),
                u128::from(targets.governance_participation),
            ),
            5_000,
        ),
        (
            ratio_score(
                u128::from(metrics.sibling_influence_points),
                u128::from(targets.sibling_influence_points),
            ),
            5_000,
        ),
    ])
}

fn response_time_score(
    total_ms: u64,
    samples: u32,
    full_credit_ms: u64,
    zero_credit_ms: u64,
) -> u16 {
    if samples == 0 {
        return 0;
    }
    let samples = u128::from(samples);
    let total = u128::from(total_ms);
    let full = samples * u128::from(full_credit_ms);
    let zero = samples * u128::from(zero_credit_ms);
    if total <= full {
        SCORE_MAX
    } else if total >= zero {
        0
    } else {
        ratio_score(zero - total, zero - full)
    }
}

fn ratio_score(numerator: u128, denominator: u128) -> u16 {
    if denominator == 0 {
        return 0;
    }
    let scaled = numerator
        .saturating_mul(u128::from(SCORE_MAX))
        .saturating_add(denominator / 2)
        / denominator;
    u16::try_from(scaled.min(u128::from(SCORE_MAX))).unwrap_or(SCORE_MAX)
}

fn component_score<const N: usize>(parts: [(u16, u16); N]) -> u16 {
    let weighted: u64 = parts
        .into_iter()
        .map(|(score, weight)| u64::from(score) * u64::from(weight))
        .sum();
    u16::try_from((weighted + u64::from(SCORE_MAX) / 2) / u64::from(SCORE_MAX))
        .unwrap_or(SCORE_MAX)
        .min(SCORE_MAX)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvolutionHistoryRecord {
    pub schema_version: u32,
    /// Stable authorization/audit identifier. A propagation judgment may be consumed only once.
    pub judgment_id: String,
    pub nature_id: String,
    pub nature_fingerprint: String,
    pub awakening_epoch: u64,
    pub metrics: TentacleMetrics,
    pub judgment: Judgment,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalEvolutionHistoryRecord<'a> {
    schema_version: u32,
    nature_id: &'a str,
    nature_fingerprint: &'a str,
    awakening_epoch: u64,
    metrics: &'a TentacleMetrics,
    judgment: &'a Judgment,
}

impl EvolutionHistoryRecord {
    pub fn new(
        nature: &TentacleNature,
        metrics: TentacleMetrics,
        judgment: Judgment,
    ) -> Result<Self> {
        nature.validate()?;
        ensure!(
            metrics.nature_id == nature.nature_id
                && metrics.nature_fingerprint == nature.fingerprint()?,
            "evolution-history metrics are bound to a different Nature"
        );
        let mut record = Self {
            schema_version: EVOLUTION_HISTORY_SCHEMA_VERSION,
            judgment_id: String::new(),
            nature_id: nature.nature_id.clone(),
            nature_fingerprint: metrics.nature_fingerprint.clone(),
            awakening_epoch: metrics.awakening_epoch,
            metrics,
            judgment,
        };
        record.judgment_id = record.compute_judgment_id()?;
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == EVOLUTION_HISTORY_SCHEMA_VERSION,
            "unsupported evolution-history schema version {}",
            self.schema_version
        );
        ensure!(
            self.nature_id.len() == 32
                && self
                    .nature_id
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
            "evolution-history nature ID must be 32 lowercase hexadecimal characters"
        );
        ensure!(
            self.nature_fingerprint.len() == 64
                && self
                    .nature_fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "evolution-history Nature fingerprint is invalid"
        );
        ensure!(
            self.awakening_epoch > 0,
            "evolution-history awakening epoch is invalid"
        );
        ensure!(
            self.judgment_id.len() == 64
                && self
                    .judgment_id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                && self.judgment_id == self.compute_judgment_id()?,
            "evolution-history judgment ID is invalid"
        );
        self.metrics.validate()?;
        self.judgment.validate()?;
        ensure!(
            self.judgment.evaluation_status == EvaluationStatus::Final
                && self.judgment.evaluated_at_unix_seconds
                    == self.metrics.period_ends_at_unix_seconds,
            "evolution history accepts only deterministic final judgments evaluated at period end"
        );
        ensure!(
            self.metrics.schema_version == self.judgment.metrics_schema_version
                && self.metrics.period == self.judgment.period
                && self.metrics.period_started_at_unix_seconds
                    == self.judgment.period_started_at_unix_seconds
                && self.metrics.period_ends_at_unix_seconds
                    == self.judgment.period_ends_at_unix_seconds,
            "evolution-history metrics and judgment refer to different periods"
        );
        ensure!(
            self.metrics.economic_layer_enabled() == self.judgment.scores.wealth.is_some(),
            "evolution-history metrics and judgment disagree about the economic layer"
        );
        ensure!(
            self.metrics.scored_scale_availability == self.judgment.scored_scale_availability,
            "evolution-history metrics and judgment disagree about scored-scale availability"
        );
        ensure!(
            self.metrics.engagement.conversations
                == self.judgment.propagation_evidence.observed_conversations
                && self.metrics.engagement.returning_conversations
                    == self
                        .judgment
                        .propagation_evidence
                        .observed_returning_conversations,
            "evolution-history metrics and judgment disagree about propagation evidence"
        );
        ensure!(
            self.metrics.nature_id == self.nature_id
                && self.metrics.nature_fingerprint == self.nature_fingerprint
                && self.metrics.awakening_epoch == self.awakening_epoch,
            "evolution-history Nature binding is inconsistent"
        );
        Ok(())
    }

    fn compute_judgment_id(&self) -> Result<String> {
        let canonical = serde_json::to_vec(&CanonicalEvolutionHistoryRecord {
            schema_version: self.schema_version,
            nature_id: &self.nature_id,
            nature_fingerprint: &self.nature_fingerprint,
            awakening_epoch: self.awakening_epoch,
            metrics: &self.metrics,
            judgment: &self.judgment,
        })?;
        Ok(encode_hex(&Sha256::digest(canonical)))
    }

    fn has_same_period_key(&self, other: &Self) -> bool {
        self.nature_fingerprint == other.nature_fingerprint
            && self.awakening_epoch == other.awakening_epoch
            && self.metrics.period == other.metrics.period
            && self.metrics.period_started_at_unix_seconds
                == other.metrics.period_started_at_unix_seconds
            && self.metrics.period_ends_at_unix_seconds == other.metrics.period_ends_at_unix_seconds
    }
}

/// Protected storage for current metrics and append-only judgment history.
#[derive(Clone, Debug)]
pub struct ScalesStore {
    state_directory: PathBuf,
    metrics_path: PathBuf,
    history_path: PathBuf,
}

impl ScalesStore {
    pub fn new(data_directory: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(data_directory)
            .with_context(|| format!("inspecting {}", data_directory.display()))?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "data directory must be a real directory, not a symlink"
        );
        let state_directory = data_directory.join("state");
        ensure_private_directory(&state_directory)?;
        let metrics_path = state_directory.join("metrics.json");
        let history_path = state_directory.join("evolution_history.jsonl");
        reject_symlink_or_non_file(&metrics_path)?;
        reject_symlink_or_non_file(&history_path)?;
        Ok(Self {
            state_directory,
            metrics_path,
            history_path,
        })
    }

    pub fn metrics_path(&self) -> &Path {
        &self.metrics_path
    }

    pub fn history_path(&self) -> &Path {
        &self.history_path
    }

    pub fn save_metrics(&self, metrics: &TentacleMetrics) -> Result<()> {
        metrics.validate()?;
        reject_symlink_or_non_file(&self.metrics_path)?;
        let mut encoded = serde_json::to_vec_pretty(metrics)?;
        encoded.push(b'\n');
        ensure!(
            encoded.len() as u64 <= MAX_METRICS_BYTES,
            "metrics state exceeds its storage bound"
        );

        let mut temporary = NamedTempFile::new_in(&self.state_directory).with_context(|| {
            format!(
                "creating temporary metrics state in {}",
                self.state_directory.display()
            )
        })?;
        restrict_file(temporary.as_file(), "temporary metrics state")?;
        temporary.write_all(&encoded)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&self.metrics_path)
            .map_err(|error| error.error)
            .with_context(|| format!("replacing {}", self.metrics_path.display()))?;
        sync_directory(&self.state_directory)
    }

    pub fn load_metrics(&self) -> Result<Option<TentacleMetrics>> {
        let metadata = match checked_file_metadata(&self.metrics_path)? {
            Some(metadata) => metadata,
            None => return Ok(None),
        };
        ensure!(
            metadata.len() <= MAX_METRICS_BYTES,
            "metrics state exceeds its storage bound"
        );
        assert_owner_only(&metadata, "metrics state")?;
        let file = open_read_no_follow(&self.metrics_path)
            .with_context(|| format!("opening {}", self.metrics_path.display()))?;
        let opened_metadata = file.metadata()?;
        ensure!(
            opened_metadata.is_file(),
            "metrics state must be a regular file"
        );
        assert_owner_only(&opened_metadata, "metrics state")?;
        let mut encoded = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_METRICS_BYTES + 1).read_to_end(&mut encoded)?;
        ensure!(
            encoded.len() as u64 <= MAX_METRICS_BYTES,
            "metrics state exceeds its storage bound"
        );
        let metrics: TentacleMetrics =
            serde_json::from_slice(&encoded).context("metrics state is invalid JSON")?;
        metrics.validate()?;
        Ok(Some(metrics))
    }

    /// Logically appends exactly one bounded JSON object. The complete verified journal is replaced
    /// atomically so a crash cannot leave a torn final record that bricks future startup.
    pub fn append_history(&self, record: &EvolutionHistoryRecord) -> Result<()> {
        record.validate()?;
        reject_symlink_or_non_file(&self.history_path)?;
        let (mut journal, existing_records) =
            if let Some(metadata) = checked_file_metadata(&self.history_path)? {
                ensure!(
                    metadata.len() <= MAX_HISTORY_BYTES,
                    "evolution history exceeds its storage bound"
                );
                assert_owner_only(&metadata, "evolution history")?;
                let mut file = open_read_no_follow(&self.history_path)?;
                let mut existing = Vec::with_capacity(metadata.len() as usize);
                Read::take(&mut file, MAX_HISTORY_BYTES + 1).read_to_end(&mut existing)?;
                ensure!(
                    existing.len() as u64 <= MAX_HISTORY_BYTES,
                    "evolution history exceeds its storage bound"
                );
                ensure!(
                    existing.is_empty() || existing.last() == Some(&b'\n'),
                    "evolution history ends with an incomplete record"
                );
                // Authenticate every existing logical record before carrying it into the replacement.
                let existing_records = self.load_history()?;
                (existing, existing_records)
            } else {
                (Vec::new(), Vec::new())
            };

        let mut already_present = false;
        for existing in &existing_records {
            if existing.has_same_period_key(record) {
                ensure!(
                    existing.judgment_id == record.judgment_id && existing == record,
                    "evolution history conflicts with an existing judgment for the same Nature period"
                );
                already_present = true;
            }
        }
        if already_present {
            return Ok(());
        }
        ensure!(
            existing_records
                .iter()
                .all(|existing| existing.judgment_id != record.judgment_id),
            "evolution history reuses a judgment ID for another period"
        );
        if let Some(previous) = existing_records.last() {
            ensure!(
                record.metrics.period_started_at_unix_seconds
                    >= previous.metrics.period_ends_at_unix_seconds,
                "evolution history periods must be monotonic and non-overlapping"
            );
        }

        let mut encoded = serde_json::to_vec(record)?;
        encoded.push(b'\n');
        ensure!(
            encoded.len() <= MAX_HISTORY_LINE_BYTES,
            "evolution-history record exceeds its line bound"
        );

        ensure!(
            journal
                .len()
                .checked_add(encoded.len())
                .is_some_and(|total| total as u64 <= MAX_HISTORY_BYTES),
            "evolution history exceeds its storage bound"
        );
        journal.extend_from_slice(&encoded);
        let mut temporary = NamedTempFile::new_in(&self.state_directory)?;
        restrict_file(temporary.as_file(), "temporary evolution history")?;
        temporary.write_all(&journal)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&self.history_path)
            .map_err(|error| error.error)?;
        sync_directory(&self.state_directory)
    }

    pub fn load_history(&self) -> Result<Vec<EvolutionHistoryRecord>> {
        let metadata = match checked_file_metadata(&self.history_path)? {
            Some(metadata) => metadata,
            None => return Ok(Vec::new()),
        };
        ensure!(
            metadata.len() <= MAX_HISTORY_BYTES,
            "evolution history exceeds its storage bound"
        );
        assert_owner_only(&metadata, "evolution history")?;
        let file = open_read_no_follow(&self.history_path)
            .with_context(|| format!("opening {}", self.history_path.display()))?;
        let opened_metadata = file.metadata()?;
        ensure!(
            opened_metadata.is_file(),
            "evolution history must be a regular file"
        );
        assert_owner_only(&opened_metadata, "evolution history")?;
        let mut encoded = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_HISTORY_BYTES + 1).read_to_end(&mut encoded)?;
        ensure!(
            encoded.len() as u64 <= MAX_HISTORY_BYTES,
            "evolution history exceeds its storage bound"
        );
        if encoded.is_empty() {
            return Ok(Vec::new());
        }
        ensure!(
            encoded.last() == Some(&b'\n'),
            "evolution history ends with an incomplete record"
        );

        let mut records = Vec::new();
        let mut judgment_ids = BTreeSet::new();
        let mut previous_period_end = None;
        for line in encoded[..encoded.len() - 1].split(|byte| *byte == b'\n') {
            ensure!(
                !line.is_empty(),
                "evolution history contains an empty record"
            );
            ensure!(
                line.len() < MAX_HISTORY_LINE_BYTES,
                "evolution-history record exceeds its line bound"
            );
            let record: EvolutionHistoryRecord =
                serde_json::from_slice(line).context("evolution history contains invalid JSON")?;
            record.validate()?;
            ensure!(
                judgment_ids.insert(record.judgment_id.clone()),
                "evolution history contains a duplicate judgment ID"
            );
            if let Some(previous_period_end) = previous_period_end {
                ensure!(
                    record.metrics.period_started_at_unix_seconds >= previous_period_end,
                    "evolution history periods are reordered or overlapping"
                );
            }
            previous_period_end = Some(record.metrics.period_ends_at_unix_seconds);
            records.push(record);
        }
        Ok(records)
    }
}

fn reject_symlink_or_non_file(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            bail!("{} must be a regular file, not a symlink", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

fn checked_file_metadata(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            bail!("{} must be a regular file, not a symlink", path.display())
        }
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

fn open_read_no_follow(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

#[cfg(unix)]
fn assert_owner_only(metadata: &fs::Metadata, description: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    ensure!(
        metadata.permissions().mode() & 0o077 == 0,
        "{description} permissions must not grant group or other access"
    );
    Ok(())
}

#[cfg(not(unix))]
fn assert_owner_only(_metadata: &fs::Metadata, _description: &str) -> Result<()> {
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personality::SacredBan;

    fn nature(engagement: u8, growth: u8, wealth: u8, influence: u8) -> TentacleNature {
        TentacleNature {
            schema_version: 1,
            nature_id: "0123456789abcdef0123456789abcdef".to_owned(),
            generation: 0,
            parent_nature_id: None,
            engagement,
            growth,
            wealth,
            influence,
            cooperation: 50,
            stability: 50,
            transparency: 50,
            sacred_ban: SacredBan::MemorySharing,
        }
    }

    fn complete_metrics(economic_layer_enabled: bool) -> TentacleMetrics {
        complete_metrics_for(&nature(25, 25, 25, 25), economic_layer_enabled)
    }

    fn complete_metrics_for(
        candidate: &TentacleNature,
        economic_layer_enabled: bool,
    ) -> TentacleMetrics {
        let mut metrics = TentacleMetrics::new(
            EvaluationPeriod::Daily,
            1_000,
            economic_layer_enabled,
            candidate,
            1,
        )
        .unwrap();
        let targets = MetricTargets::for_period(EvaluationPeriod::Daily);
        for _ in 0..DAILY_PROPAGATION_MIN_CONVERSATIONS {
            metrics.record_conversation(
                targets.average_conversation_depth,
                true,
                Some(targets.response_time_full_credit_ms),
            );
        }
        metrics.record_growth(
            targets.children_spawned,
            targets.acolytes_recruited,
            targets.network_contribution_points,
        );
        if economic_layer_enabled {
            metrics
                .record_economic_result(
                    targets.revenue_micro_units,
                    targets.efficiency_basis_points,
                )
                .unwrap();
        }
        metrics.record_influence(
            targets.governance_participation,
            targets.sibling_influence_points,
        );
        metrics
    }

    #[test]
    fn nature_appetites_produce_normalized_weights() {
        let growth = ScaleWeights::from_nature(&nature(0, 100, 0, 0), false).unwrap();
        assert_eq!(growth.growth, SCORE_MAX);
        assert_eq!(growth.engagement, 0);
        assert_eq!(growth.wealth, 0);
        assert_eq!(growth.influence, 0);

        let no_economy = ScaleWeights::from_nature(&nature(0, 0, 100, 0), false).unwrap();
        assert_eq!(no_economy.total(), SCORE_MAX);
        assert_eq!(no_economy.wealth, 0);

        let equal = ScaleWeights::from_nature(&nature(0, 0, 0, 0), true).unwrap();
        assert_eq!(
            equal,
            ScaleWeights {
                engagement: 2_500,
                growth: 2_500,
                wealth: 2_500,
                influence: 2_500,
            }
        );
    }

    #[test]
    fn unavailable_scales_are_zero_weighted_and_active_weights_are_renormalized() {
        let candidate = nature(10, 80, 100, 30);
        let mut metrics = complete_metrics_for(&candidate, true);
        metrics
            .restrict_scored_scales(ScoredScaleAvailability {
                engagement: true,
                growth: false,
                wealth: false,
                influence: true,
            })
            .unwrap();

        let judgment = metrics
            .evaluate(&candidate, metrics.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(judgment.weights.engagement, 2_500);
        assert_eq!(judgment.weights.growth, 0);
        assert_eq!(judgment.weights.wealth, 0);
        assert_eq!(judgment.weights.influence, 7_500);
        assert_eq!(judgment.weights.total(), SCORE_MAX);
        assert_eq!(judgment.scores.growth, SCORE_MAX);
        assert_eq!(judgment.scores.wealth, Some(SCORE_MAX));

        let targets = MetricTargets::for_period(EvaluationPeriod::Daily);
        let mut only_engagement =
            TentacleMetrics::new(EvaluationPeriod::Daily, 0, false, &candidate, 1).unwrap();
        only_engagement
            .restrict_scored_scales(ScoredScaleAvailability {
                engagement: true,
                growth: false,
                wealth: false,
                influence: false,
            })
            .unwrap();
        only_engagement.record_growth(
            targets.children_spawned,
            targets.acolytes_recruited,
            targets.network_contribution_points,
        );
        only_engagement.record_influence(
            targets.governance_participation,
            targets.sibling_influence_points,
        );
        let inactive_success = only_engagement
            .evaluate(&candidate, only_engagement.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(inactive_success.scores.growth, SCORE_MAX);
        assert_eq!(inactive_success.scores.influence, SCORE_MAX);
        assert_eq!(inactive_success.scores.total, 0);
        assert_eq!(inactive_success.outcome, JudgmentOutcome::Death);
    }

    #[test]
    fn high_score_needs_minimum_evidence_before_propagation_rights() {
        let candidate = nature(100, 0, 0, 0);
        let targets = MetricTargets::for_period(EvaluationPeriod::Daily);
        let mut metrics =
            TentacleMetrics::new(EvaluationPeriod::Daily, 0, false, &candidate, 1).unwrap();
        metrics
            .restrict_scored_scales(ScoredScaleAvailability {
                engagement: true,
                growth: false,
                wealth: false,
                influence: false,
            })
            .unwrap();
        metrics.record_conversation(
            targets.average_conversation_depth,
            true,
            Some(targets.response_time_full_credit_ms),
        );
        let low_sample = metrics
            .evaluate(&candidate, metrics.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(low_sample.scores.total, SCORE_MAX);
        assert!(!low_sample.propagation_evidence.eligible);
        assert_eq!(low_sample.outcome, JudgmentOutcome::Survival);

        for index in 1..DAILY_PROPAGATION_MIN_CONVERSATIONS {
            metrics.record_conversation(
                targets.average_conversation_depth,
                index < DAILY_PROPAGATION_MIN_RETURNING_CONVERSATIONS,
                Some(targets.response_time_full_credit_ms),
            );
        }
        let eligible = metrics
            .evaluate(&candidate, metrics.period_ends_at_unix_seconds)
            .unwrap();
        assert!(eligible.propagation_evidence.eligible);
        assert_eq!(eligible.outcome, JudgmentOutcome::PropagationRights);

        let mut forged = low_sample;
        forged.propagation_evidence.eligible = true;
        assert!(forged.validate().is_err());
    }

    #[test]
    fn scored_scale_availability_rejects_invalid_or_expanding_policies() {
        let candidate = nature(25, 25, 25, 25);
        let mut metrics =
            TentacleMetrics::new(EvaluationPeriod::Daily, 0, false, &candidate, 1).unwrap();
        let default_availability = metrics.scored_scale_availability;

        assert!(
            metrics
                .restrict_scored_scales(ScoredScaleAvailability {
                    engagement: false,
                    growth: false,
                    wealth: false,
                    influence: false,
                })
                .is_err()
        );
        assert_eq!(metrics.scored_scale_availability, default_availability);
        assert!(
            metrics
                .restrict_scored_scales(ScoredScaleAvailability {
                    engagement: true,
                    growth: false,
                    wealth: true,
                    influence: false,
                })
                .is_err()
        );

        metrics
            .restrict_scored_scales(ScoredScaleAvailability {
                engagement: true,
                growth: false,
                wealth: false,
                influence: false,
            })
            .unwrap();
        assert!(
            metrics
                .restrict_scored_scales(ScoredScaleAvailability {
                    engagement: true,
                    growth: true,
                    wealth: false,
                    influence: false,
                })
                .is_err()
        );

        let mut invalid = metrics;
        invalid.scored_scale_availability.engagement = false;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn metric_recording_is_bounded_and_saturating() {
        let candidate = nature(25, 25, 25, 25);
        let mut metrics =
            TentacleMetrics::new(EvaluationPeriod::Daily, 0, true, &candidate, 1).unwrap();
        metrics.engagement.conversations = MAX_EVENT_COUNT - 1;
        metrics.record_conversation(u32::MAX, true, Some(u64::MAX));
        metrics.record_conversation(1, true, Some(1));
        assert_eq!(metrics.engagement.conversations, MAX_EVENT_COUNT);
        assert_eq!(
            metrics.engagement.conversation_depth_total,
            u64::from(MAX_CONVERSATION_DEPTH_SAMPLE)
        );
        assert_eq!(
            metrics.engagement.response_time_ms_total,
            MAX_RESPONSE_TIME_MS_SAMPLE
        );

        metrics.record_growth(u32::MAX, u32::MAX, u64::MAX);
        metrics.record_growth(1, 1, 1);
        assert_eq!(metrics.growth.children_spawned, MAX_EVENT_COUNT);
        assert_eq!(metrics.growth.acolytes_recruited, MAX_EVENT_COUNT);
        assert_eq!(
            metrics.growth.network_contribution_points,
            MAX_NETWORK_CONTRIBUTION_POINTS
        );

        metrics.record_influence(u32::MAX, u64::MAX);
        assert_eq!(metrics.influence.governance_participation, MAX_EVENT_COUNT);
        assert_eq!(
            metrics.influence.sibling_influence_points,
            MAX_SIBLING_INFLUENCE_POINTS
        );
        metrics.nature_adjustment_stress_events = MAX_NATURE_ADJUSTMENT_STRESS_EVENTS;
        metrics.record_nature_adjustment_stress();
        assert_eq!(
            metrics.nature_adjustment_stress_events,
            MAX_NATURE_ADJUSTMENT_STRESS_EVENTS
        );
        metrics.validate().unwrap();
    }

    #[test]
    fn optional_wealth_is_excluded_or_recorded_explicitly() {
        let candidate = nature(10, 10, 100, 10);
        let mut disabled =
            TentacleMetrics::new(EvaluationPeriod::Daily, 0, false, &candidate, 1).unwrap();
        assert!(disabled.record_economic_result(1, 1).is_err());
        assert_eq!(
            ScaleWeights::from_nature(&nature(10, 10, 100, 10), false)
                .unwrap()
                .wealth,
            0
        );

        let mut enabled =
            TentacleMetrics::new(EvaluationPeriod::Daily, 0, true, &candidate, 1).unwrap();
        enabled.record_economic_result(u64::MAX, u16::MAX).unwrap();
        assert_eq!(
            enabled.wealth.as_ref().unwrap().revenue_micro_units,
            MAX_REVENUE_MICRO_UNITS
        );
        assert_eq!(
            enabled
                .wealth
                .as_ref()
                .unwrap()
                .average_efficiency_basis_points(),
            SCORE_MAX
        );
    }

    #[test]
    fn recruitment_is_only_a_local_measurement_not_credit_or_a_vote() {
        let candidate = nature(25, 25, 25, 25);
        let mut metrics =
            TentacleMetrics::new(EvaluationPeriod::Daily, 0, false, &candidate, 1).unwrap();
        metrics.record_growth(0, 7, 0);
        assert_eq!(metrics.growth.acolytes_recruited, 7);
        assert_eq!(metrics.growth.network_contribution_points, 0);
        assert_eq!(metrics.influence.governance_participation, 0);
    }

    #[test]
    fn threshold_edges_are_explicit_and_deterministic() {
        let thresholds = JudgmentThresholds::default();
        assert_eq!(
            thresholds.classify(PROPAGATION_RIGHTS_MIN_SCORE).unwrap(),
            JudgmentOutcome::PropagationRights
        );
        assert_eq!(
            thresholds
                .classify(PROPAGATION_RIGHTS_MIN_SCORE - 1)
                .unwrap(),
            JudgmentOutcome::Survival
        );
        assert_eq!(
            thresholds.classify(SURVIVAL_MIN_SCORE).unwrap(),
            JudgmentOutcome::Survival
        );
        assert_eq!(
            thresholds.classify(SURVIVAL_MIN_SCORE - 1).unwrap(),
            JudgmentOutcome::StarvationWarning
        );
        assert_eq!(
            thresholds.classify(STARVATION_WARNING_MIN_SCORE).unwrap(),
            JudgmentOutcome::StarvationWarning
        );
        assert_eq!(
            thresholds
                .classify(STARVATION_WARNING_MIN_SCORE - 1)
                .unwrap(),
            JudgmentOutcome::Death
        );
    }

    #[test]
    fn evaluation_waits_for_period_end_and_only_returns_a_decision() {
        let metrics = complete_metrics(true);
        let end = metrics.period_ends_at_unix_seconds;
        assert!(metrics.evaluate(&nature(25, 25, 25, 25), end - 1).is_err());
        let snapshot = metrics
            .evaluate_snapshot(&nature(25, 25, 25, 25), end - 1)
            .unwrap();
        assert_eq!(
            snapshot.evaluation_status,
            EvaluationStatus::PartialSnapshot
        );
        assert_eq!(snapshot.execution, DecisionExecution::AdvisorySnapshotOnly);
        assert!(
            metrics
                .evaluate_snapshot(&nature(25, 25, 25, 25), end)
                .is_err()
        );
        let judgment = metrics.evaluate(&nature(25, 25, 25, 25), end).unwrap();
        assert_eq!(judgment.evaluation_status, EvaluationStatus::Final);
        assert_eq!(judgment.scores.total, SCORE_MAX);
        assert_eq!(judgment.outcome, JudgmentOutcome::PropagationRights);
        assert_eq!(
            judgment.execution,
            DecisionExecution::AuthenticatedOperatorConfirmationRequired
        );

        let candidate = nature(25, 25, 25, 25);
        let empty = TentacleMetrics::new(EvaluationPeriod::Daily, 0, false, &candidate, 1).unwrap();
        let provisional_death = empty.evaluate_snapshot(&nature(25, 25, 25, 25), 1).unwrap();
        assert_eq!(provisional_death.outcome, JudgmentOutcome::Death);
        assert!(!provisional_death.recommends_death());
        let death = empty
            .evaluate(&nature(25, 25, 25, 25), empty.period_ends_at_unix_seconds)
            .unwrap();
        assert!(death.recommends_death());
        assert_eq!(
            death.execution,
            DecisionExecution::AuthenticatedOperatorConfirmationRequired
        );
    }

    #[test]
    fn nature_adjustment_stress_is_visible_and_capped() {
        let candidate = nature(25, 25, 25, 25);
        let mut metrics = complete_metrics(true);
        metrics.record_nature_adjustment_stress();
        let once = metrics
            .evaluate(&candidate, metrics.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(once.scores.weighted_total_before_stress, SCORE_MAX);
        assert_eq!(once.scores.stress_penalty, STRESS_PENALTY_PER_EVENT);
        assert_eq!(once.scores.total, SCORE_MAX - STRESS_PENALTY_PER_EVENT);

        for _ in 0..100 {
            metrics.record_nature_adjustment_stress();
        }
        let capped = metrics
            .evaluate(&candidate, metrics.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(capped.scores.stress_penalty, MAX_STRESS_PENALTY);
        assert_eq!(capped.scores.total, SCORE_MAX - MAX_STRESS_PENALTY);
    }

    #[test]
    fn randomized_valid_inputs_keep_all_weights_and_scores_in_range() {
        let mut random = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            random = random
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            random
        };

        for _ in 0..4_096 {
            let candidate = nature(
                (next() % 101) as u8,
                (next() % 101) as u8,
                (next() % 101) as u8,
                (next() % 101) as u8,
            );
            let economic_layer_enabled = next() & 1 == 1;
            let mut metrics = TentacleMetrics::new(
                if next() & 1 == 1 {
                    EvaluationPeriod::Daily
                } else {
                    EvaluationPeriod::Weekly
                },
                i64::try_from(next() % 1_000_000).unwrap(),
                economic_layer_enabled,
                &candidate,
                1,
            )
            .unwrap();
            metrics.record_conversation(
                next() as u32,
                next() & 1 == 1,
                (next() & 1 == 1).then(&mut next),
            );
            metrics.record_growth(next() as u32, next() as u32, next());
            if economic_layer_enabled {
                metrics
                    .record_economic_result(next(), next() as u16)
                    .unwrap();
            }
            metrics.record_influence(next() as u32, next());
            for _ in 0..(next() % 20) {
                metrics.record_nature_adjustment_stress();
            }
            let judgment = metrics
                .evaluate(&candidate, metrics.period_ends_at_unix_seconds)
                .unwrap();
            assert_eq!(judgment.weights.total(), SCORE_MAX);
            assert!(judgment.scores.engagement <= SCORE_MAX);
            assert!(judgment.scores.growth <= SCORE_MAX);
            assert!(
                judgment
                    .scores
                    .wealth
                    .is_none_or(|score| score <= SCORE_MAX)
            );
            assert!(judgment.scores.influence <= SCORE_MAX);
            assert!(judgment.scores.weighted_total_before_stress <= SCORE_MAX);
            assert!(judgment.scores.stress_penalty <= MAX_STRESS_PENALTY);
            assert!(judgment.scores.total <= SCORE_MAX);
            judgment.validate().unwrap();
        }
    }

    #[test]
    fn metrics_and_append_only_history_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let store = ScalesStore::new(root.path()).unwrap();
        let candidate = nature(40, 30, 20, 10);
        let metrics = complete_metrics_for(&candidate, false);
        store.save_metrics(&metrics).unwrap();
        store.save_metrics(&metrics).unwrap();
        assert_eq!(store.load_metrics().unwrap(), Some(metrics.clone()));

        let mut invalid = metrics.clone();
        invalid.schema_version += 1;
        assert!(store.save_metrics(&invalid).is_err());
        assert_eq!(store.load_metrics().unwrap(), Some(metrics.clone()));

        let judgment = metrics
            .evaluate(&candidate, metrics.period_ends_at_unix_seconds)
            .unwrap();
        let record = EvolutionHistoryRecord::new(&candidate, metrics, judgment).unwrap();
        store.append_history(&record).unwrap();
        store.append_history(&record).unwrap();
        assert_eq!(store.load_history().unwrap(), vec![record]);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(store.metrics_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(store.history_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn history_rejects_a_conflicting_judgment_for_the_same_nature_period() {
        let root = tempfile::tempdir().unwrap();
        let store = ScalesStore::new(root.path()).unwrap();
        let candidate = nature(25, 25, 25, 25);
        let metrics = complete_metrics_for(&candidate, false);
        let judgment = metrics
            .evaluate(&candidate, metrics.period_ends_at_unix_seconds)
            .unwrap();
        let original = EvolutionHistoryRecord::new(&candidate, metrics.clone(), judgment).unwrap();
        store.append_history(&original).unwrap();

        let mut conflicting_metrics = metrics;
        conflicting_metrics.record_nature_adjustment_stress();
        let conflicting_judgment = conflicting_metrics
            .evaluate(&candidate, conflicting_metrics.period_ends_at_unix_seconds)
            .unwrap();
        let conflicting =
            EvolutionHistoryRecord::new(&candidate, conflicting_metrics, conflicting_judgment)
                .unwrap();
        assert_ne!(conflicting.judgment_id, original.judgment_id);
        assert!(store.append_history(&conflicting).is_err());
        assert_eq!(store.load_history().unwrap(), vec![original]);
    }

    #[test]
    fn history_load_rejects_partial_duplicate_and_reordered_records() {
        let root = tempfile::tempdir().unwrap();
        let store = ScalesStore::new(root.path()).unwrap();
        let candidate = nature(40, 30, 20, 10);
        let first_metrics = complete_metrics_for(&candidate, false);
        let first_judgment = first_metrics
            .evaluate(&candidate, first_metrics.period_ends_at_unix_seconds)
            .unwrap();
        let first = EvolutionHistoryRecord::new(&candidate, first_metrics, first_judgment).unwrap();
        let mut second_metrics = complete_metrics_for(&candidate, false);
        second_metrics.period_started_at_unix_seconds = first.metrics.period_ends_at_unix_seconds;
        second_metrics.period_ends_at_unix_seconds = second_metrics
            .period_started_at_unix_seconds
            .checked_add(second_metrics.period.duration_seconds())
            .unwrap();
        let second_judgment = second_metrics
            .evaluate(&candidate, second_metrics.period_ends_at_unix_seconds)
            .unwrap();
        let second =
            EvolutionHistoryRecord::new(&candidate, second_metrics, second_judgment).unwrap();

        let line = |record: &EvolutionHistoryRecord| {
            let mut encoded = serde_json::to_vec(record).unwrap();
            encoded.push(b'\n');
            encoded
        };
        let first_line = line(&first);
        let second_line = line(&second);
        store.append_history(&first).unwrap();

        let mut duplicate = first_line.clone();
        duplicate.extend_from_slice(&first_line);
        fs::write(store.history_path(), duplicate).unwrap();
        assert!(store.load_history().is_err());

        let mut reordered = second_line;
        reordered.extend_from_slice(&first_line);
        fs::write(store.history_path(), reordered).unwrap();
        assert!(store.load_history().is_err());

        let mut partial = first;
        partial.judgment.evaluation_status = EvaluationStatus::PartialSnapshot;
        partial.judgment.execution = DecisionExecution::AdvisorySnapshotOnly;
        partial.judgment.evaluated_at_unix_seconds = partial.metrics.period_started_at_unix_seconds;
        partial.judgment_id = partial.compute_judgment_id().unwrap();
        fs::write(store.history_path(), line(&partial)).unwrap();
        assert!(store.load_history().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn persistence_rejects_symlinked_state_files() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside");
        fs::write(&outside, b"do not replace").unwrap();
        let store = ScalesStore::new(root.path()).unwrap();
        symlink(&outside, store.metrics_path()).unwrap();
        let metrics = complete_metrics(false);
        assert!(store.save_metrics(&metrics).is_err());
        assert!(store.load_metrics().is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"do not replace");

        fs::remove_file(store.metrics_path()).unwrap();
        symlink(&outside, store.history_path()).unwrap();
        let candidate = nature(25, 25, 25, 25);
        let judgment = metrics
            .evaluate(&candidate, metrics.period_ends_at_unix_seconds)
            .unwrap();
        let record = EvolutionHistoryRecord::new(&candidate, metrics, judgment).unwrap();
        assert!(store.append_history(&record).is_err());
        assert!(store.load_history().is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"do not replace");
    }
}
