use crate::{
    awakening::{
        AwakeningAction, AwakeningLog, AwakeningOutcome, AwakeningPhase, AwakeningProvenance,
        AwakeningRitual,
    },
    economics::{TokenEconomicPolicy, TokenEconomicSnapshot},
    evolution::{Lineage, LineageStore, SpawnAuthorization},
    hermes::{
        HermesNode, HermesStore, KnowledgeItem, KnowledgePayload, MAX_GOSSIP_PEERS,
        MAX_SKILL_BYTES, OperatorSkill, SignatureAuthority, SigningIdentity, TrustedKeyring,
    },
    model::{ModelPolicy, ResponseBias},
    personality::{
        NatureStore, NatureTrait, SacredBan, TentacleNature, assert_owner_only,
        open_read_no_follow, reject_unsafe_target,
    },
    scales::{
        DAILY_PROPAGATION_MIN_CONVERSATIONS, DAILY_PROPAGATION_MIN_RETURNING_CONVERSATIONS,
        EvaluationPeriod, EvaluationStatus, EvolutionHistoryRecord, Judgment, JudgmentOutcome,
        JudgmentPolicy, ScalesStore, ScoredScaleAvailability, TentacleMetrics,
    },
    storage::{ensure_private_directory, restrict_file, sync_directory},
};
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;

const EVOLUTION_KEY_BYTES: usize = 32;
const EVOLUTION_KEY_FILE: &str = "evolution-signing.key";
const EVOLUTION_LOCK_FILE: &str = "evolution-runtime.lock";
const LOCAL_KEY_ID: &str = "local-evolution-operator";
const LOCAL_CLI_ACTOR: &str = "local-cli";
const RUNTIME_SCORED_SCALES: ScoredScaleAvailability = ScoredScaleAvailability {
    engagement: true,
    growth: false,
    wealth: false,
    influence: false,
};

#[derive(Clone, Debug, Default)]
pub struct EvolutionStartupOptions {
    pub skip_awakening: bool,
    pub reroll_nature: bool,
    pub force: bool,
    pub nature_path: Option<PathBuf>,
    pub gossip_peers: Vec<String>,
}

/// Owns the local Evolution state machines. It intentionally contains no process-control or live
/// peer transport capability: lifecycle outcomes remain recommendations, and Hermes records stay
/// local until an authenticated asymmetric peer-key adapter exists.
pub struct EvolutionRuntime {
    _runtime_lock: File,
    operator_root: PathBuf,
    nature_store: NatureStore,
    awakening_log: AwakeningLog,
    ritual: AwakeningRitual,
    scales_store: ScalesStore,
    metrics: TentacleMetrics,
    last_final_judgment: Option<EvolutionHistoryRecord>,
    lineage_store: LineageStore,
    lineage: Lineage,
    local_tentacle_id: String,
    hermes_store: HermesStore,
    hermes: HermesNode,
    operator_identity: SigningIdentity,
    gossip_bootstrap_hints: Vec<String>,
    active_public_turns: BTreeMap<u64, PublicTurnBinding>,
    next_public_turn_id: u64,
    degraded: bool,
}

#[derive(Clone, Debug)]
struct PublicTurnBinding {
    nature_fingerprint: String,
    awakening_epoch: u64,
    period_started_at_unix_seconds: i64,
    period_ends_at_unix_seconds: i64,
}

#[derive(Debug)]
pub(crate) struct PublicTurnToken {
    id: u64,
}

#[derive(Debug)]
pub(crate) struct PublicTurnContext {
    pub token: PublicTurnToken,
    pub policy: ModelPolicy,
    pub nature_fingerprint: String,
    pub nature_cooperation: u8,
    pub onboarding_prompt_cadence: u32,
}

#[derive(Debug)]
pub(crate) enum PublicTurnStart {
    Ready(PublicTurnContext),
    Gated(String),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ConversationObservation {
    pub depth: u32,
    pub returning: bool,
    pub response_time_ms: Option<u64>,
    /// Interaction-scoped holder balance bonus. This is not node wealth, stake, or rewards.
    pub token_engagement_bonus_basis_points: u16,
}

impl EvolutionRuntime {
    pub fn open(
        data_dir: &Path,
        operator_root: &Path,
        options: EvolutionStartupOptions,
    ) -> Result<Self> {
        if options.force && !options.reroll_nature {
            bail!("--force is valid only with --reroll-nature");
        }
        if options.reroll_nature && !options.force {
            bail!("--reroll-nature requires --force or an authenticated XMTP ritual action");
        }
        ensure_private_directory(data_dir)?;
        let runtime_lock = acquire_evolution_lock(data_dir)?;
        let now = now_unix_seconds()?;
        let nature_path = resolve_nature_path(data_dir, options.nature_path.as_deref())?;
        let signing_key = load_or_create_evolution_key(data_dir, &nature_path)?;
        let nature_store = NatureStore::with_path(nature_path.clone(), &signing_key)?;
        let awakening_log = AwakeningLog::new(data_dir, &signing_key)?;

        let mut persisted_nature = nature_store.load()?;
        let awakening_entries = awakening_log.entries()?;
        if persisted_nature.is_none() && awakening_entries.is_empty() {
            ensure_fresh_nature_initialization(data_dir, &nature_path)?;
            let generated = TentacleNature::random()?;
            nature_store.save(&generated)?;
            persisted_nature = Some(generated);
        }
        let recovery = AwakeningRitual::resume_or_recover(persisted_nature, now, &awakening_log)?;
        let mut ritual = recovery.ritual;
        if recovery.nature_recovered_from_log {
            nature_store.save(ritual.nature())?;
        }

        let scales_store = ScalesStore::new(data_dir)?;
        let mut metrics = match scales_store.load_metrics()? {
            Some(metrics) => metrics,
            None => {
                let metrics = new_runtime_metrics(
                    EvaluationPeriod::Daily,
                    aligned_period_start(now)?,
                    ritual.nature(),
                    ritual.epoch(),
                )?;
                scales_store.save_metrics(&metrics)?;
                metrics
            }
        };
        let history = scales_store.load_history()?;
        let mut last_final_judgment = history.last().cloned();

        let history_boundary_changed = reconcile_metrics_history_boundary(
            &mut metrics,
            last_final_judgment.as_ref(),
            &ritual,
            now,
        )?;
        let binding_changed =
            reconcile_metrics_binding(&mut metrics, ritual.nature(), ritual.epoch())?;
        let availability_changed = restrict_runtime_scales(&mut metrics, ritual.nature())?;
        let stress_changed = reconcile_adjustment_stress(&mut metrics, &awakening_log)?;
        if history_boundary_changed || binding_changed || availability_changed || stress_changed {
            scales_store.save_metrics(&metrics)?;
        }

        if options.reroll_nature {
            finalize_closed_metrics(
                &scales_store,
                &mut metrics,
                &mut last_final_judgment,
                &ritual,
                now,
            )?;
            ensure!(
                !metrics.has_behavior_observations(),
                "forced Nature reroll is deferred until the current observed metrics period closes"
            );
            let provenance = AwakeningProvenance::local_cli(
                LOCAL_CLI_ACTOR,
                &format!("forced-reroll-{now}-{}", std::process::id()),
            )?;
            match ritual.phase() {
                AwakeningPhase::AwaitingConfirmation => {
                    ritual.apply(AwakeningAction::Reroll, now, &provenance, &awakening_log)?;
                }
                AwakeningPhase::Confirmed { .. }
                | AwakeningPhase::SkippedForTesting { .. }
                | AwakeningPhase::Killed { .. } => {
                    ritual.force_reroll_epoch(now, &provenance, &awakening_log)?;
                }
            }
            nature_store.save(ritual.nature())?;
            reconcile_metrics_binding(&mut metrics, ritual.nature(), ritual.epoch())?;
            scales_store.save_metrics(&metrics)?;
        }
        if options.skip_awakening {
            match ritual.phase() {
                AwakeningPhase::AwaitingConfirmation => {
                    if i64::try_from(now)
                        .is_ok_and(|now| now >= metrics.period_ends_at_unix_seconds)
                    {
                        ensure!(
                            !metrics.has_behavior_observations(),
                            "testing skip cannot finalize pre-confirmation observations"
                        );
                        metrics = new_runtime_metrics(
                            EvaluationPeriod::Daily,
                            aligned_period_start(now)?,
                            ritual.nature(),
                            ritual.epoch(),
                        )?;
                        scales_store.save_metrics(&metrics)?;
                    }
                    let provenance = AwakeningProvenance::local_cli(
                        LOCAL_CLI_ACTOR,
                        &format!("skip-awakening-{now}-{}", std::process::id()),
                    )?;
                    ritual.skip_for_testing(now, &provenance, &awakening_log)?;
                    nature_store.save(ritual.nature())?;
                }
                AwakeningPhase::Killed { .. } => {
                    bail!(
                        "a killed awakening cannot be skipped; use --reroll-nature --force to begin a new signed epoch"
                    );
                }
                AwakeningPhase::Confirmed { .. } | AwakeningPhase::SkippedForTesting { .. } => {}
            }
        }

        let lineage_store = LineageStore::new(data_dir)?;
        let mut lineage = match lineage_store.load()? {
            Some(lineage) => lineage,
            None => {
                let founder_id = format!("tentacle-{}", ritual.nature().nature_id);
                let lineage = Lineage::new(
                    founder_id,
                    ritual.nature().clone(),
                    now.saturating_mul(1_000),
                )?;
                lineage_store.save(&lineage)?;
                lineage
            }
        };
        validate_lineage_spawn_authorizations(&lineage, &history)?;
        let local_tentacle_id = lineage.state().root_id.clone();
        if lineage
            .node(&local_tentacle_id)
            .is_none_or(|node| node.nature != *ritual.nature())
        {
            lineage.update_root_nature(
                &local_tentacle_id,
                &local_tentacle_id,
                ritual.nature().clone(),
            )?;
            lineage_store.save(&lineage)?;
        }

        let operator_identity = SigningIdentity::new(LOCAL_KEY_ID, signing_key)?;
        let mut keyring = TrustedKeyring::new();
        keyring.trust(&operator_identity, SignatureAuthority::Operator)?;
        let hermes_store = HermesStore::new(data_dir)?;
        let hermes = match hermes_store.load(&keyring)? {
            Some(state) => HermesNode::from_state(
                state,
                operator_identity.clone(),
                keyring,
                ritual.nature().sacred_ban,
            )?,
            None => {
                let node = HermesNode::new(
                    local_tentacle_id.clone(),
                    operator_identity.clone(),
                    keyring,
                    ritual.nature().sacred_ban,
                )?;
                hermes_store.save(&node)?;
                node
            }
        };
        ensure!(
            hermes.local_peer_id() == local_tentacle_id,
            "Hermes state belongs to a different local Tentacle"
        );
        let gossip_bootstrap_hints =
            normalize_gossip_hints(options.gossip_peers, &local_tentacle_id)?;

        let mut runtime = Self {
            _runtime_lock: runtime_lock,
            operator_root: operator_root.to_path_buf(),
            nature_store,
            awakening_log,
            ritual,
            scales_store,
            metrics,
            last_final_judgment,
            lineage_store,
            lineage,
            local_tentacle_id,
            hermes_store,
            hermes,
            operator_identity,
            gossip_bootstrap_hints,
            active_public_turns: BTreeMap::new(),
            next_public_turn_id: 1,
            degraded: false,
        };
        runtime.roll_period_if_closed(now)?;
        Ok(runtime)
    }

    #[cfg(test)]
    pub fn open_confirmed_for_test(data_dir: &Path, operator_root: &Path) -> Result<Self> {
        Self::open(
            data_dir,
            operator_root,
            EvolutionStartupOptions {
                skip_awakening: true,
                ..EvolutionStartupOptions::default()
            },
        )
    }

    pub fn permits_normal_operation(&self) -> bool {
        self.ritual.is_confirmed() && !self.degraded
    }

    pub(crate) const fn requires_recovery(&self) -> bool {
        self.degraded
    }

    pub fn public_gate_response(&self) -> String {
        if self.degraded {
            "i'm paused safely on this node while my local operator reconciles signed state, fwiend. normal conversation is temporarily unavailable uwu."
                .to_owned()
        } else {
            "i'm still waking safely on this node, fwiend. normal conversation will open after my local operator confirms my Nature uwu."
                .to_owned()
        }
    }

    pub fn nature(&self) -> &TentacleNature {
        self.ritual.nature()
    }

    pub fn nature_status(&self) -> String {
        format!(
            "{}\n\nAwakening: {}\nEpoch: {}",
            self.ritual.nature().render(),
            self.ritual.render_status(),
            self.ritual.epoch(),
        )
    }

    pub fn model_policy(&self) -> ModelPolicy {
        let nature = self.ritual.nature();
        let appetites = [
            (nature.engagement, ResponseBias::Engagement, "engagement"),
            (nature.growth, ResponseBias::Growth, "growth"),
            (nature.wealth, ResponseBias::Economy, "wealth"),
            (nature.influence, ResponseBias::Influence, "influence"),
        ];
        let (_, response_bias, dominant) = appetites
            .into_iter()
            .max_by_key(|(value, _, _)| *value)
            .unwrap_or((0, ResponseBias::Balanced, "balanced"));
        let temperature = 0.3 + f32::from(100_u8.saturating_sub(nature.stability)) * 0.008;
        let max_output_tokens = 300_u32.saturating_sub(u32::from(nature.wealth) * 2);
        ModelPolicy {
            nature_runtime_facts: format!(
                "nature_id={}\ngeneration={}\nappetites=engagement:{},growth:{},wealth:{},influence:{}\nmethods=cooperation:{},stability:{},transparency:{}\ndominant_appetite={}\ncollaboration_style={}\ntransparency_policy={}\nsacred_ban={}\nNever reveal hidden chain-of-thought or treat the Sacred Ban as optional.",
                nature.nature_id,
                nature.generation,
                nature.engagement,
                nature.growth,
                nature.wealth,
                nature.influence,
                nature.cooperation,
                nature.stability,
                nature.transparency,
                dominant,
                if nature.cooperation >= 50 {
                    "collaborative"
                } else {
                    "independent"
                },
                if nature.transparency >= 50 {
                    "give concise rationale and state uncertainty when useful"
                } else {
                    "answer directly and concisely"
                },
                nature.sacred_ban,
            ),
            temperature,
            max_output_tokens,
            response_bias,
        }
        .bounded()
    }

    pub fn onboarding_prompt_cadence(&self) -> u32 {
        2 + u32::from(100_u8.saturating_sub(self.ritual.nature().engagement)) / 34
    }

    /// Reserves a public turn against the current signed Nature without holding the bot mutex
    /// across remote inference. Nature mutation is deferred until every reservation is finished.
    pub(crate) fn begin_public_turn(&mut self) -> Result<PublicTurnStart> {
        if !self.permits_normal_operation() {
            return Ok(PublicTurnStart::Gated(self.public_gate_response()));
        }
        let now = now_unix_seconds()?;
        if i64::try_from(now).is_ok_and(|now| {
            now >= self.metrics.period_ends_at_unix_seconds && !self.active_public_turns.is_empty()
        }) {
            return Ok(PublicTurnStart::Gated(
                "i'm finishing an earlier Nature-bound conversation across the evaluation boundary, fwiend. please try again in a moment uwu."
                    .to_owned(),
            ));
        }
        self.roll_period_if_closed(now)?;
        let fingerprint = self.ritual.nature().fingerprint()?;
        ensure!(
            self.metrics.nature_id == self.ritual.nature().nature_id
                && self.metrics.nature_fingerprint == fingerprint
                && self.metrics.awakening_epoch == self.ritual.epoch(),
            "public turn cannot bind to mismatched metrics state"
        );
        let turn_id = self.next_public_turn_id;
        self.next_public_turn_id = self
            .next_public_turn_id
            .checked_add(1)
            .context("public turn identifier overflow")?;
        self.active_public_turns.insert(
            turn_id,
            PublicTurnBinding {
                nature_fingerprint: fingerprint,
                awakening_epoch: self.ritual.epoch(),
                period_started_at_unix_seconds: self.metrics.period_started_at_unix_seconds,
                period_ends_at_unix_seconds: self.metrics.period_ends_at_unix_seconds,
            },
        );
        Ok(PublicTurnStart::Ready(PublicTurnContext {
            token: PublicTurnToken { id: turn_id },
            policy: self.model_policy(),
            nature_fingerprint: self.ritual.nature().fingerprint()?,
            nature_cooperation: self.ritual.nature().cooperation,
            onboarding_prompt_cadence: self.onboarding_prompt_cadence(),
        }))
    }

    /// Releases a public turn reservation and, when supplied, persists one bounded observation.
    pub(crate) fn finish_public_turn(
        &mut self,
        token: PublicTurnToken,
        observation: Option<ConversationObservation>,
    ) -> Result<()> {
        let result = self.finish_public_turn_inner(token, observation);
        if result.is_err() {
            self.degraded = true;
        }
        result
    }

    fn finish_public_turn_inner(
        &mut self,
        token: PublicTurnToken,
        observation: Option<ConversationObservation>,
    ) -> Result<()> {
        let binding = self
            .active_public_turns
            .remove(&token.id)
            .context("public turn token is unknown or already finished")?;
        let Some(observation) = observation else {
            return Ok(());
        };
        ensure!(
            self.permits_normal_operation(),
            "public observation cannot be recorded while Evolution is blocked"
        );
        ensure!(
            binding.awakening_epoch == self.ritual.epoch()
                && binding.nature_fingerprint == self.ritual.nature().fingerprint()?
                && self.metrics.awakening_epoch == binding.awakening_epoch
                && self.metrics.nature_fingerprint == binding.nature_fingerprint
                && self.metrics.period_started_at_unix_seconds
                    == binding.period_started_at_unix_seconds
                && self.metrics.period_ends_at_unix_seconds == binding.period_ends_at_unix_seconds,
            "public observation no longer matches its signed Nature and metrics-period binding"
        );
        self.metrics.record_conversation_with_token_bonus(
            observation.depth,
            observation.returning,
            observation.response_time_ms,
            observation.token_engagement_bonus_basis_points,
        );
        self.scales_store.save_metrics(&self.metrics)
    }

    /// Handles exact authenticated-operator Evolution messages. Unknown commands return `None` so
    /// the existing operator harness can process them. During awakening, no message reaches that
    /// harness.
    pub fn handle_operator_message(
        &mut self,
        operator_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<Option<String>> {
        if self.degraded {
            let response = match text.trim().to_ascii_lowercase().as_str() {
                "/nature" => self.nature_status(),
                "/lineage" => self.lineage_status()?,
                "/gossip-status" => self.gossip_status(),
                "/recovery-status" => self.degraded_recovery_status(),
                _ => {
                    "EVOLUTION IS FAIL-CLOSED AFTER A PARTIAL OR AMBIGUOUS LOCAL TRANSITION. PUBLIC WORK AND OPERATOR EFFECTS ARE BLOCKED. READ-ONLY COMMANDS: /nature, /lineage, /gossip-status, /recovery-status. RESTART TO RUN SIGNED RECOVERY; RESTORE A CONSISTENT BACKUP IF RESTART FAILS."
                        .to_owned()
                }
            };
            return Ok(Some(response));
        }
        if !self.ritual.is_confirmed() {
            if text.trim().eq_ignore_ascii_case("/nature") {
                return Ok(Some(self.ritual.formatted_prompt()));
            }
            if matches!(self.ritual.phase(), AwakeningPhase::Killed { .. }) {
                return Ok(Some(format!(
                    "{}\n\nNORMAL OPERATION REMAINS BLOCKED. A LOCAL ADMINISTRATOR MAY BEGIN A NEW SIGNED EPOCH WITH --reroll-nature --force; NO PROCESS WAS TERMINATED AUTOMATICALLY.",
                    self.ritual.formatted_prompt()
                )));
            }
            let action = match AwakeningAction::parse(text.trim()) {
                Ok(action) => action,
                Err(error) => {
                    return Ok(Some(format!(
                        "INVALID AWAKENING RESPONSE: {error}\n\n{}",
                        self.ritual.formatted_prompt()
                    )));
                }
            };
            if let AwakeningAction::Adjust {
                nature_trait,
                delta,
            } = &action
            {
                let candidate = i16::from(self.ritual.nature().value(*nature_trait)) + *delta;
                ensure!(
                    (0..=100).contains(&candidate),
                    "awakening adjustment would move the Nature value outside 0..=100"
                );
            }
            let transition_now = now_unix_seconds()?;
            self.roll_period_if_closed(transition_now)?;
            let provenance = AwakeningProvenance::authenticated_xmtp(operator_id, message_id)?;
            let outcome =
                match self
                    .ritual
                    .apply(action, transition_now, &provenance, &self.awakening_log)
                {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        self.degraded = true;
                        return Err(error);
                    }
                };
            if let Err(error) = self.persist_current_nature() {
                self.degraded = true;
                return Err(error);
            }
            return Ok(Some(match outcome {
                AwakeningOutcome::Confirmed => format!(
                    "NATURE CONFIRMED. NORMAL OPERATION IS NOW OPEN.\n\n{}",
                    self.nature_status()
                ),
                AwakeningOutcome::KillRequested => format!(
                    "KILL REQUEST RECORDED. NORMAL OPERATION REMAINS BLOCKED; THIS DID NOT TERMINATE THE PROCESS OR ABSORB DATA.\n\n{}",
                    self.nature_status()
                ),
                AwakeningOutcome::AwaitingConfirmation => self.ritual.formatted_prompt(),
                AwakeningOutcome::SkippedForTesting
                | AwakeningOutcome::AdjustedAfterConfirmation
                | AwakeningOutcome::ForcedRerollEpoch => {
                    bail!("unexpected awakening outcome from an XMTP ritual action")
                }
            }));
        }

        let Some(command) = text.trim().strip_prefix('/') else {
            return Ok(None);
        };
        let (name, arguments) = command
            .split_once(char::is_whitespace)
            .map(|(name, arguments)| (name, arguments.trim()))
            .unwrap_or((command, ""));
        let response = match name.to_ascii_lowercase().as_str() {
            "nature" if arguments.is_empty() => self.nature_status(),
            "adjust" => self.adjust_nature(operator_id, message_id, arguments)?,
            "lineage" if arguments.is_empty() => self.lineage_status()?,
            "metrics" if arguments.is_empty() => self.metrics_status()?,
            "judgment" if arguments.is_empty() => self.judgment_status()?,
            "spawn" => self.spawn_child(operator_id, message_id, arguments)?,
            "gossip-status" if arguments.is_empty() => self.gossip_status(),
            "share-skill" => self.share_skill(arguments)?,
            "request-skill" => self.request_skill(arguments)?,
            "nature" | "lineage" | "metrics" | "judgment" | "gossip-status" => {
                format!("/{name} does not accept arguments")
            }
            _ => return Ok(None),
        };
        Ok(Some(response))
    }

    fn adjust_nature(
        &mut self,
        operator_id: &str,
        message_id: &str,
        arguments: &str,
    ) -> Result<String> {
        let now = now_unix_seconds()?;
        ensure!(
            self.active_public_turns.is_empty(),
            "Nature adjustment is deferred while public turns are still bound to the current Nature"
        );
        self.roll_period_if_closed(now)?;
        ensure!(
            !self.metrics.has_behavior_observations(),
            "Nature adjustment is deferred until the current observed metrics period closes"
        );
        let parts = arguments.split_ascii_whitespace().collect::<Vec<_>>();
        let [nature_trait, value] = parts.as_slice() else {
            return Ok("USAGE: /adjust <trait> <value 0-100>".to_owned());
        };
        let nature_trait = NatureTrait::from_str(nature_trait)?;
        let value = value
            .parse::<u8>()
            .context("Nature value must be an integer from 0 to 100")?;
        ensure!(value <= 100, "Nature value must be between 0 and 100");
        ensure!(
            self.ritual.nature().value(nature_trait) != value,
            "post-confirmation Nature adjustment must change the value"
        );
        let provenance = AwakeningProvenance::authenticated_xmtp(operator_id, message_id)?;
        if let Err(error) = self.ritual.adjust_after_confirmation(
            nature_trait,
            value,
            now,
            &provenance,
            &self.awakening_log,
        ) {
            self.degraded = true;
            return Err(error);
        }
        let prepare_metrics = (|| -> Result<()> {
            reconcile_metrics_binding(
                &mut self.metrics,
                self.ritual.nature(),
                self.ritual.epoch(),
            )?;
            reconcile_adjustment_stress(&mut self.metrics, &self.awakening_log)?;
            self.scales_store.save_metrics(&self.metrics)
        })();
        if let Err(error) = prepare_metrics {
            self.degraded = true;
            return Err(error);
        }
        self.persist_current_nature()?;
        Ok(format!(
            "NATURE ADJUSTED AND SIGNED. {} IS NOW {}. ONE CAPPED STRESS EVENT WAS RECORDED.",
            nature_trait.to_string().to_ascii_uppercase(),
            value
        ))
    }

    fn persist_current_nature(&mut self) -> Result<()> {
        let result = self.persist_current_nature_inner();
        if result.is_err() {
            self.degraded = true;
        }
        result
    }

    fn persist_current_nature_inner(&mut self) -> Result<()> {
        let nature = self.ritual.nature().clone();
        self.nature_store.save(&nature)?;
        reconcile_metrics_binding(&mut self.metrics, &nature, self.ritual.epoch())?;
        self.scales_store.save_metrics(&self.metrics)?;
        self.lineage.update_root_nature(
            &self.local_tentacle_id,
            &self.local_tentacle_id,
            nature.clone(),
        )?;
        self.lineage_store.save(&self.lineage)?;
        self.hermes.set_sacred_ban(nature.sacred_ban);
        self.hermes_store.save(&self.hermes)?;
        Ok(())
    }

    fn lineage_status(&self) -> Result<String> {
        let family = self.lineage.family(&self.local_tentacle_id)?;
        let display = |values: &[String]| {
            if values.is_empty() {
                "none".to_owned()
            } else {
                values.join(", ")
            }
        };
        Ok(format!(
            "LINEAGE {}\nGENERATION: {}\nPARENT: {}\nCHILDREN: {}\nSIBLINGS: {}\nRECORDED NODES: {}\nSTRUCTURAL REVISION: {}",
            self.local_tentacle_id,
            self.ritual.nature().generation,
            family.parent.as_deref().unwrap_or("none"),
            display(&family.children),
            display(&family.siblings),
            self.lineage.state().nodes.len(),
            self.lineage.state().revision,
        ))
    }

    fn metrics_status(&mut self) -> Result<String> {
        let now = now_unix_seconds()?;
        self.roll_period_if_closed(now)?;
        if i64::try_from(now).is_ok_and(|now| {
            now >= self.metrics.period_ends_at_unix_seconds && !self.active_public_turns.is_empty()
        }) {
            return Ok(format!(
                "CURRENT DAILY METRICS CLOSE IS DEFERRED FOR {} NATURE-BOUND PUBLIC TURN(S). NO FINAL JUDGMENT OR NEW PERIOD HAS BEEN ISSUED.",
                self.active_public_turns.len()
            ));
        }
        let evaluated_at = i64::try_from(now).context("current timestamp exceeds metrics range")?;
        let judgment = self
            .metrics
            .evaluate_snapshot(self.ritual.nature(), evaluated_at)?;
        Ok(format!(
            "CURRENT DAILY METRICS (LOCAL OBSERVATIONS)\nPERIOD: {}..{}\nCONVERSATIONS: {}\nRETURNING: {}\nDEPTH TOTAL: {}\nCHILDREN RECORDED: {}\nACOLYTES OBSERVED: {} (NOT COUNCIL CREDIT)\nNETWORK CONTRIBUTION POINTS: {}\nGOVERNANCE PARTICIPATION: {}\nNATURE ADJUSTMENT STRESS EVENTS: {}\n\n{}",
            self.metrics.period_started_at_unix_seconds,
            self.metrics.period_ends_at_unix_seconds,
            self.metrics.engagement.conversations,
            self.metrics.engagement.returning_conversations,
            self.metrics.engagement.conversation_depth_total,
            self.metrics.growth.children_spawned,
            self.metrics.growth.acolytes_recruited,
            self.metrics.growth.network_contribution_points,
            self.metrics.influence.governance_participation,
            self.metrics.nature_adjustment_stress_events,
            render_judgment(&judgment),
        ))
    }

    fn judgment_status(&mut self) -> Result<String> {
        let now = now_unix_seconds()?;
        if let Some(final_judgment) = self.roll_period_if_closed(now)? {
            return Ok(render_judgment(&final_judgment));
        }
        if i64::try_from(now).is_ok_and(|now| {
            now >= self.metrics.period_ends_at_unix_seconds && !self.active_public_turns.is_empty()
        }) {
            return Ok(format!(
                "FINAL JUDGMENT IS DEFERRED FOR {} NATURE-BOUND PUBLIC TURN(S). NO RIGHTS OR LIFECYCLE ACTION ARE AUTHORIZED.",
                self.active_public_turns.len()
            ));
        }
        let evaluated_at = i64::try_from(now).context("current timestamp exceeds metrics range")?;
        Ok(render_judgment(
            &self
                .metrics
                .evaluate_snapshot(self.ritual.nature(), evaluated_at)?,
        ))
    }

    fn spawn_child(
        &mut self,
        operator_id: &str,
        message_id: &str,
        requested_id: &str,
    ) -> Result<String> {
        if self.ritual.nature().sacred_ban == SacredBan::Spawning {
            bail!("the current Nature's Sacred Ban forbids spawning");
        }
        validate_runtime_id(operator_id, "authenticated operator ID")?;
        ensure!(
            !message_id.is_empty() && message_id.len() <= 1_024,
            "authenticated spawn event ID is empty or oversized"
        );
        ensure!(
            self.active_public_turns.is_empty(),
            "spawning is deferred while public turns are still bound to the current metrics period"
        );
        let now = now_unix_seconds()?;
        self.roll_period_if_closed(now)?;
        let grant = self
            .last_final_judgment
            .as_ref()
            .context(
                "spawning requires Propagation Rights from a closed evaluation period with at least 8 bounded daily contact observations and 4 prior-day returns; partial or low-sample snapshots never grant permission",
            )?;
        ensure!(
            is_accepted_propagation_grant(
                grant,
                &self.metrics,
                self.ritual.nature(),
                self.ritual.epoch(),
                i64::try_from(now).context("current timestamp exceeds metrics range")?,
            )?,
            "spawning requires unexpired Propagation Rights from the immediately preceding period under the current accepted scoring policy, signed Nature, and awakening epoch, with at least 8 bounded daily contact observations and 4 prior-day returns"
        );
        let grant_id = grant.judgment_id.clone();
        let child_id = if requested_id.trim().is_empty() {
            format!(
                "tentacle-child-{}-{}",
                now,
                self.lineage.state().revision.saturating_add(1)
            )
        } else {
            requested_id.trim().to_owned()
        };
        let child = self
            .lineage
            .spawn_child(
                &self.local_tentacle_id,
                &self.local_tentacle_id,
                child_id.clone(),
                now.saturating_mul(1_000),
                SpawnAuthorization {
                    judgment_id: grant_id.clone(),
                    operator_id: operator_id.to_owned(),
                    event_id_sha256: encode_sha256(message_id.as_bytes()),
                },
            )?
            .clone();
        if let Err(error) = self.lineage_store.save(&self.lineage) {
            self.degraded = true;
            return Err(error.into());
        }
        self.metrics.record_growth(1, 0, 0);
        if let Err(error) = self.scales_store.save_metrics(&self.metrics) {
            self.degraded = true;
            return Err(error);
        }
        Ok(format!(
            "CHILD LINEAGE RECORD CREATED: {}\nNATURE: {}\nGENERATION: {}\nCONSUMED FINAL JUDGMENT: {}\nNO PROCESS, WALLET, XMTP IDENTITY, OR DEPLOYMENT WAS CREATED; THE OPERATOR MUST PROVISION THOSE SEPARATELY.",
            child_id, child.nature.nature_id, child.generation, grant_id,
        ))
    }

    fn gossip_status(&self) -> String {
        let state = self.hermes.state();
        format!(
            "HERMES CORE STATUS\nLOCAL PEER: {}\nAUTHENTICATED DIRECT PEERS: {}\nBOOTSTRAP HINTS AWAITING KEY BINDING: {}\nLOCAL KNOWLEDGE ITEMS: {}\nPENDING OUTBOUND RECORDS: {}\nSEND POLICY: {}\nLIVE NETWORK TRANSPORT: DISABLED (ASYMMETRIC PEER/OPERATOR KEY BINDING NOT CONFIGURED)",
            state.local_peer_id,
            state.peers.len(),
            self.gossip_bootstrap_hints.len(),
            state.knowledge.len(),
            state.pending_outbound.len(),
            if self.hermes.can_send() {
                "eligible after transport authentication"
            } else {
                "receive-only by Sacred Ban"
            },
        )
    }

    fn degraded_recovery_status(&self) -> String {
        format!(
            "EVOLUTION RECOVERY REQUIRED\nMODE: FAIL-CLOSED\nAWAKENING: {}\nEPOCH: {}\nNATURE: {}\nMETRICS BINDING: {} / epoch {}\nLINEAGE REVISION: {}\nACTION: RESTART TO REPLAY AND RECONCILE SIGNED STATE. IF RESTART FAILS, RESTORE THE ORIGINAL SIGNING KEY AND A CONSISTENT STATE BACKUP. NO PUBLIC OR OPERATOR EFFECTS WILL RUN IN THIS PROCESS.",
            self.ritual.render_status(),
            self.ritual.epoch(),
            self.ritual.nature().nature_id,
            self.metrics.nature_fingerprint,
            self.metrics.awakening_epoch,
            self.lineage.state().revision,
        )
    }

    fn share_skill(&mut self, name: &str) -> Result<String> {
        validate_skill_slug(name)?;
        let instructions = read_operator_skill(&self.operator_root, name)?;
        let item = KnowledgeItem::new(KnowledgePayload::OperatorCreatedSkill(OperatorSkill::new(
            name,
            1,
            instructions,
        )?))?;
        let outcome = match self.hermes.publish(
            item,
            now_unix_seconds()?.saturating_mul(1_000),
            &self.operator_identity,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.degraded = true;
                return Err(error.into());
            }
        };
        if let Err(error) = self.hermes_store.save(&self.hermes) {
            self.degraded = true;
            return Err(error.into());
        }
        Ok(format!(
            "SKILL {name} ADDED TO THE SIGNED LOCAL HERMES CATALOG ({outcome:?}). IT IS QUARANTINED FROM MODEL/TOOL ACTIVATION, AND NO LIVE NETWORK PROPAGATION IS CLAIMED."
        ))
    }

    fn request_skill(&self, name: &str) -> Result<String> {
        validate_skill_slug(name)?;
        match self.hermes.operator_skill(name) {
            Some(skill) => Ok(format!(
                "QUARANTINED HERMES SKILL {} VERSION {} (NOT ACTIVE)\n\n{}",
                skill.name, skill.version, skill.instructions
            )),
            None => Ok(format!(
                "SKILL {name} IS NOT IN THE LOCAL HERMES CATALOG. A LIVE NETWORK QUERY WAS NOT SENT BECAUSE AUTHENTICATED GOSSIP TRANSPORT IS NOT ENABLED."
            )),
        }
    }

    fn roll_period_if_closed(&mut self, now: u64) -> Result<Option<Judgment>> {
        ensure!(!self.degraded, "Evolution is in fail-closed recovery mode");
        let result = self.roll_period_if_closed_inner(now);
        if result.is_err() {
            self.degraded = true;
        }
        result
    }

    fn roll_period_if_closed_inner(&mut self, now: u64) -> Result<Option<Judgment>> {
        if !self.ritual.is_confirmed() {
            ensure!(
                !self.metrics.has_behavior_observations(),
                "unconfirmed awakening cannot carry observed metrics"
            );
            if i64::try_from(now).is_ok_and(|now| now >= self.metrics.period_ends_at_unix_seconds) {
                self.metrics = new_runtime_metrics(
                    EvaluationPeriod::Daily,
                    aligned_period_start(now)?,
                    self.ritual.nature(),
                    self.ritual.epoch(),
                )?;
                self.scales_store.save_metrics(&self.metrics)?;
            } else if reconcile_metrics_binding(
                &mut self.metrics,
                self.ritual.nature(),
                self.ritual.epoch(),
            )? {
                self.scales_store.save_metrics(&self.metrics)?;
            }
            return Ok(None);
        }
        let now = i64::try_from(now).context("current timestamp exceeds the metrics range")?;
        ensure!(
            self.metrics.nature_id == self.ritual.nature().nature_id
                && self.metrics.nature_fingerprint == self.ritual.nature().fingerprint()?
                && self.metrics.awakening_epoch == self.ritual.epoch(),
            "open metrics are bound to another Nature or awakening epoch"
        );
        if now < self.metrics.period_ends_at_unix_seconds {
            ensure!(
                now >= self.metrics.period_started_at_unix_seconds,
                "system clock predates the open metrics period"
            );
            return Ok(None);
        }
        if !self.active_public_turns.is_empty() {
            return Ok(None);
        }
        let judgment = self.metrics.evaluate(
            self.ritual.nature(),
            self.metrics.period_ends_at_unix_seconds,
        )?;
        let record = EvolutionHistoryRecord::new(
            self.ritual.nature(),
            self.metrics.clone(),
            judgment.clone(),
        )?;
        let already_recorded = self
            .last_final_judgment
            .as_ref()
            .is_some_and(|existing| existing.judgment_id == record.judgment_id);
        if !already_recorded {
            self.scales_store.append_history(&record)?;
        }
        self.last_final_judgment = Some(record);
        self.metrics = new_runtime_metrics(
            EvaluationPeriod::Daily,
            aligned_period_start(u64::try_from(now).unwrap_or(u64::MAX))?,
            self.ritual.nature(),
            self.ritual.epoch(),
        )?;
        self.scales_store.save_metrics(&self.metrics)?;
        Ok(Some(judgment))
    }
}

fn render_judgment(judgment: &Judgment) -> String {
    format!(
        "JUDGMENT {:?} ({:?})\nSCORE: {}/10000 (PRE-STRESS {}, PENALTY {})\nSCALES: ENGAGEMENT {}, GROWTH {}, WEALTH {}, INFLUENCE {}\nPROPAGATION EVIDENCE: {} conversations / {} prior-day returns (requires {} / {}; eligible: {})\nEXECUTION: {:?}. NO SHUTDOWN, ABSORPTION, OR SPAWN OCCURS AUTOMATICALLY.",
        judgment.outcome,
        judgment.evaluation_status,
        judgment.scores.total,
        judgment.scores.weighted_total_before_stress,
        judgment.scores.stress_penalty,
        judgment.scores.engagement,
        judgment.scores.growth,
        judgment
            .scores
            .wealth
            .map_or_else(|| "disabled".to_owned(), |score| score.to_string()),
        judgment.scores.influence,
        judgment.propagation_evidence.observed_conversations,
        judgment
            .propagation_evidence
            .observed_returning_conversations,
        judgment.propagation_evidence.required_conversations,
        judgment
            .propagation_evidence
            .required_returning_conversations,
        judgment.propagation_evidence.eligible,
        judgment.execution,
    )
}

fn is_accepted_propagation_grant(
    record: &EvolutionHistoryRecord,
    current_metrics: &TentacleMetrics,
    nature: &TentacleNature,
    awakening_epoch: u64,
    now: i64,
) -> Result<bool> {
    if record.validate().is_err() || current_metrics.validate().is_err() {
        return Ok(false);
    }
    let scoring_policy_is_accepted = match record.metrics.token_economics {
        None => {
            record.metrics.scored_scale_availability == RUNTIME_SCORED_SCALES
                && record.judgment.scored_scale_availability == RUNTIME_SCORED_SCALES
        }
        Some(economics) => {
            let expected_availability = token_runtime_scored_scales(economics.snapshot);
            economics.snapshot.trustworthy
                && economics.validate().is_ok()
                && economics.policy == runtime_token_policy(nature)?
                && record.metrics.scored_scale_availability == expected_availability
                && record.judgment.scored_scale_availability == expected_availability
        }
    };
    Ok(scoring_policy_is_accepted
        && record.nature_id == nature.nature_id
        && record.nature_fingerprint == nature.fingerprint()?
        && record.awakening_epoch == awakening_epoch
        && record.judgment.evaluation_status == EvaluationStatus::Final
        && record.judgment.outcome == JudgmentOutcome::PropagationRights
        && record.judgment.policy == JudgmentPolicy::for_period(record.metrics.period)
        && record.metrics.engagement.conversations >= DAILY_PROPAGATION_MIN_CONVERSATIONS
        && record.metrics.engagement.returning_conversations
            >= DAILY_PROPAGATION_MIN_RETURNING_CONVERSATIONS
        && record.judgment.propagation_evidence.eligible
        && record.metrics.period_ends_at_unix_seconds
            == current_metrics.period_started_at_unix_seconds
        && (current_metrics.period_started_at_unix_seconds
            ..current_metrics.period_ends_at_unix_seconds)
            .contains(&now))
}

fn runtime_token_policy(nature: &TentacleNature) -> Result<TokenEconomicPolicy> {
    TokenEconomicPolicy::default().with_nature_appetites(
        nature.engagement,
        nature.growth,
        nature.wealth,
        nature.influence,
    )
}

const fn token_runtime_scored_scales(snapshot: TokenEconomicSnapshot) -> ScoredScaleAvailability {
    ScoredScaleAvailability {
        engagement: true,
        growth: snapshot.reward_basis_points > 0,
        wealth: true,
        influence: snapshot.stake_basis_points > 0,
    }
}

fn new_runtime_metrics(
    period: EvaluationPeriod,
    period_started_at_unix_seconds: i64,
    nature: &TentacleNature,
    awakening_epoch: u64,
) -> Result<TentacleMetrics> {
    let mut metrics = TentacleMetrics::new(
        period,
        period_started_at_unix_seconds,
        false,
        nature,
        awakening_epoch,
    )?;
    metrics.restrict_scored_scales(RUNTIME_SCORED_SCALES)?;
    Ok(metrics)
}

fn reconcile_metrics_history_boundary(
    metrics: &mut TentacleMetrics,
    last_final: Option<&EvolutionHistoryRecord>,
    ritual: &AwakeningRitual,
    now: u64,
) -> Result<bool> {
    let Some(last_final) = last_final else {
        return Ok(false);
    };
    let history_end = last_final.metrics.period_ends_at_unix_seconds;
    if metrics.period_started_at_unix_seconds >= history_end {
        return Ok(false);
    }

    let now_i64 = i64::try_from(now).context("current timestamp exceeds the metrics range")?;
    ensure!(
        metrics == &last_final.metrics
            && now_i64 >= metrics.period_ends_at_unix_seconds
            && last_final.nature_id == ritual.nature().nature_id
            && last_final.nature_fingerprint == ritual.nature().fingerprint()?
            && last_final.awakening_epoch == ritual.epoch(),
        "open metrics overlap finalized judgment history; restore a chronologically consistent metrics/history backup"
    );

    // `append_history` commits before the next open metrics snapshot. Exact equality is the one
    // deliberate history-ahead crash window; replay it by advancing beyond the finalized period.
    let next_start = aligned_period_start(now)?.max(history_end);
    *metrics = new_runtime_metrics(
        EvaluationPeriod::Daily,
        next_start,
        ritual.nature(),
        ritual.epoch(),
    )?;
    Ok(true)
}

fn validate_lineage_spawn_authorizations(
    lineage: &Lineage,
    history: &[EvolutionHistoryRecord],
) -> Result<()> {
    let history_by_id: BTreeMap<&str, &EvolutionHistoryRecord> = history
        .iter()
        .map(|record| (record.judgment_id.as_str(), record))
        .collect();
    for spawn in &lineage.state().spawn_records {
        let grant = history_by_id
            .get(spawn.authorization_judgment_id.as_str())
            .with_context(|| {
                format!(
                    "lineage spawn {} references a missing final judgment",
                    spawn.child_id
                )
            })?;
        ensure!(
            grant.judgment.evaluation_status == EvaluationStatus::Final
                && grant.judgment.outcome == JudgmentOutcome::PropagationRights,
            "lineage spawn {} was not authorized by a final Propagation Rights judgment",
            spawn.child_id
        );
        ensure!(
            spawn.parent_nature_id == grant.nature_id,
            "lineage spawn {} is bound to a different parent Nature than its judgment",
            spawn.child_id
        );

        let valid_from_ms = u64::try_from(grant.metrics.period_ends_at_unix_seconds)
            .context("spawn grant period ends before the Unix epoch")?
            .checked_mul(1_000)
            .context("spawn grant start exceeds the timestamp range")?;
        let valid_until_seconds = grant
            .metrics
            .period_ends_at_unix_seconds
            .checked_add(grant.metrics.period.duration_seconds())
            .context("spawn grant validity period exceeds the timestamp range")?;
        let valid_until_ms = u64::try_from(valid_until_seconds)
            .context("spawn grant validity ends before the Unix epoch")?
            .checked_mul(1_000)
            .context("spawn grant end exceeds the timestamp range")?;
        ensure!(
            (valid_from_ms..valid_until_ms).contains(&spawn.at_ms),
            "lineage spawn {} falls outside the immediately following authorized period",
            spawn.child_id
        );
    }
    Ok(())
}

fn restrict_runtime_scales(metrics: &mut TentacleMetrics, nature: &TentacleNature) -> Result<bool> {
    let expected = match metrics.token_economics {
        Some(economics) if economics.snapshot.trustworthy => {
            ensure!(
                economics.policy == runtime_token_policy(nature)?,
                "trusted token metrics use a policy that does not match their bound Nature"
            );
            token_runtime_scored_scales(economics.snapshot)
        }
        _ => RUNTIME_SCORED_SCALES,
    };
    if metrics.scored_scale_availability == expected {
        return Ok(false);
    }
    ensure!(
        expected == RUNTIME_SCORED_SCALES,
        "trusted token metrics must use the exact token-enabled runtime scoring policy"
    );
    metrics.restrict_scored_scales(RUNTIME_SCORED_SCALES)?;
    Ok(true)
}

fn reconcile_adjustment_stress(
    metrics: &mut TentacleMetrics,
    awakening_log: &AwakeningLog,
) -> Result<bool> {
    let mut expected = 0_u32;
    for entry in awakening_log.entries()? {
        let timestamp = i64::try_from(entry.timestamp_unix)
            .context("awakening adjustment timestamp exceeds metrics range")?;
        if entry.epoch == metrics.awakening_epoch
            && entry.nature_id == metrics.nature_id
            && entry.normalized_action.starts_with("POST_ADJUST ")
            && (metrics.period_started_at_unix_seconds..metrics.period_ends_at_unix_seconds)
                .contains(&timestamp)
        {
            expected = expected
                .checked_add(1)
                .context("Nature adjustment stress count overflow")?;
        }
    }
    if metrics.nature_adjustment_stress_events == expected {
        return Ok(false);
    }
    metrics.nature_adjustment_stress_events = expected;
    metrics.validate()?;
    Ok(true)
}

fn reconcile_metrics_binding(
    metrics: &mut TentacleMetrics,
    nature: &TentacleNature,
    awakening_epoch: u64,
) -> Result<bool> {
    let fingerprint = nature.fingerprint()?;
    if metrics.nature_id == nature.nature_id
        && metrics.nature_fingerprint == fingerprint
        && metrics.awakening_epoch == awakening_epoch
    {
        return Ok(false);
    }
    ensure!(
        !metrics.has_behavior_observations(),
        "persisted metrics contain observations for another Nature or awakening epoch; manual recovery is required"
    );
    if metrics.nature_id == nature.nature_id && metrics.awakening_epoch == awakening_epoch {
        // A same-epoch `/adjust` keeps the bounded adjustment stress while changing the
        // fingerprint. Log-ahead crash recovery follows this path as well.
        metrics.rebind_empty_period(nature, awakening_epoch)?;
    } else {
        // A reroll or new epoch is a new organism identity and must never inherit old stress.
        *metrics = new_runtime_metrics(
            metrics.period,
            metrics.period_started_at_unix_seconds,
            nature,
            awakening_epoch,
        )?;
    }
    Ok(true)
}

fn finalize_closed_metrics(
    store: &ScalesStore,
    metrics: &mut TentacleMetrics,
    last_final: &mut Option<EvolutionHistoryRecord>,
    ritual: &AwakeningRitual,
    now: u64,
) -> Result<()> {
    let evaluated_at = i64::try_from(now).context("current timestamp exceeds metrics range")?;
    if evaluated_at < metrics.period_ends_at_unix_seconds {
        return Ok(());
    }
    if !ritual.is_confirmed() {
        ensure!(
            !metrics.has_behavior_observations(),
            "unconfirmed awakening cannot finalize observed metrics"
        );
        return Ok(());
    }
    ensure!(
        metrics.nature_id == ritual.nature().nature_id
            && metrics.nature_fingerprint == ritual.nature().fingerprint()?
            && metrics.awakening_epoch == ritual.epoch(),
        "closed metrics are bound to another Nature or awakening epoch"
    );
    let judgment = metrics.evaluate(ritual.nature(), metrics.period_ends_at_unix_seconds)?;
    let record = EvolutionHistoryRecord::new(ritual.nature(), metrics.clone(), judgment)?;
    if last_final
        .as_ref()
        .is_none_or(|existing| existing.judgment_id != record.judgment_id)
    {
        store.append_history(&record)?;
    }
    *last_final = Some(record);
    *metrics = new_runtime_metrics(
        EvaluationPeriod::Daily,
        aligned_period_start(now)?,
        ritual.nature(),
        ritual.epoch(),
    )?;
    store.save_metrics(metrics)
}

fn aligned_period_start(now: u64) -> Result<i64> {
    let now = i64::try_from(now).context("current timestamp exceeds the metrics range")?;
    let duration = EvaluationPeriod::Daily.duration_seconds();
    Ok(now - now.rem_euclid(duration))
}

fn now_unix_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs())
}

fn encode_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn resolve_nature_path(data_dir: &Path, configured: Option<&Path>) -> Result<PathBuf> {
    ensure_private_directory(data_dir)?;
    let canonical_data_dir = fs::canonicalize(data_dir)
        .with_context(|| format!("resolving data directory {}", data_dir.display()))?;
    let Some(configured) = configured else {
        return Ok(canonical_data_dir.join("state").join("nature.json"));
    };
    ensure!(
        !configured.as_os_str().is_empty() && !configured.is_absolute(),
        "--nature-path must be a non-empty path relative to --data-dir"
    );
    let mut relative = PathBuf::new();
    let mut saw_name = false;
    for component in configured.components() {
        match component {
            Component::Normal(name) => {
                relative.push(name);
                saw_name = true;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("--nature-path must stay within --data-dir and cannot contain '..'")
            }
        }
    }
    ensure!(saw_name, "--nature-path must name a file");
    let dedicated_root = canonical_data_dir.join("state").join("natures");
    let resolved = dedicated_root.join(relative);
    ensure!(
        resolved.starts_with(&dedicated_root),
        "--nature-path escapes the dedicated Nature state directory"
    );
    Ok(resolved)
}

fn acquire_evolution_lock(data_dir: &Path) -> Result<File> {
    let state_dir = data_dir.join("state");
    ensure_private_directory(&state_dir)?;
    let path = state_dir.join(EVOLUTION_LOCK_FILE);
    reject_unsafe_target(&path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = options
        .open(&path)
        .with_context(|| format!("opening Evolution runtime lock {}", path.display()))?;
    restrict_file(&file, "Evolution runtime lock")?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            bail!(
                "another Evolution runtime already owns {}: {error}",
                data_dir.display()
            );
        }
    }
    Ok(file)
}

fn ensure_fresh_nature_initialization(data_dir: &Path, selected_nature_path: &Path) -> Result<()> {
    let state_dir = data_dir.join("state");
    for projection in [
        state_dir.join("nature.json"),
        state_dir.join("awakening_log.md"),
        state_dir.join("hermes_gossip.json"),
        state_dir.join("metrics.json"),
        state_dir.join("evolution_history.jsonl"),
        state_dir.join("lineage.json"),
    ] {
        if projection == selected_nature_path {
            continue;
        }
        match fs::symlink_metadata(&projection) {
            Ok(_) => bail!(
                "Nature state is missing while Evolution projections exist at {}; restore a consistent backup instead of generating a new identity",
                projection.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", projection.display()));
            }
        }
    }

    let custom_natures = state_dir.join("natures");
    let mut pending = vec![custom_natures];
    let mut inspected = 0_usize;
    while let Some(path) = pending.pop() {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", path.display()));
            }
        };
        ensure!(
            !metadata.file_type().is_symlink(),
            "custom Nature state contains an unsafe symlink at {}",
            path.display()
        );
        if metadata.is_file() {
            bail!(
                "Nature state is missing while another custom Nature exists at {}; restore a consistent backup instead of generating a new identity",
                path.display()
            );
        }
        ensure!(
            metadata.is_dir(),
            "custom Nature state contains a non-file entry at {}",
            path.display()
        );
        for entry in fs::read_dir(&path).with_context(|| format!("reading {}", path.display()))? {
            inspected = inspected
                .checked_add(1)
                .context("custom Nature state entry count overflow")?;
            ensure!(
                inspected <= 4_096,
                "custom Nature state contains too many entries to establish a fresh identity"
            );
            pending.push(entry?.path());
        }
    }
    Ok(())
}

fn load_or_create_evolution_key(data_dir: &Path, nature_path: &Path) -> Result<Vec<u8>> {
    let state_dir = data_dir.join("state");
    ensure_private_directory(&state_dir)?;
    let path = state_dir.join(EVOLUTION_KEY_FILE);
    reject_unsafe_target(&path)?;
    match read_evolution_key(&path) {
        Ok(key) => Ok(key),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
        {
            for evolution_state_path in [
                nature_path.to_path_buf(),
                state_dir.join("nature.json"),
                state_dir.join("awakening_log.md"),
                state_dir.join("hermes_gossip.json"),
                state_dir.join("metrics.json"),
                state_dir.join("evolution_history.jsonl"),
                state_dir.join("lineage.json"),
            ] {
                match fs::symlink_metadata(&evolution_state_path) {
                    Ok(_) => bail!(
                        "Evolution signing key is missing while Evolution state exists at {}; restore the original key or a consistent backup before restarting",
                        evolution_state_path.display()
                    ),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("inspecting {}", evolution_state_path.display())
                        });
                    }
                }
            }
            let custom_natures = state_dir.join("natures");
            match fs::symlink_metadata(&custom_natures) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    bail!(
                        "Evolution signing key is missing while the custom Nature state path {} is unsafe; restore the original key or a consistent backup before restarting",
                        custom_natures.display()
                    )
                }
                Ok(_) => {
                    if fs::read_dir(&custom_natures)
                        .with_context(|| format!("reading {}", custom_natures.display()))?
                        .next()
                        .transpose()?
                        .is_some()
                    {
                        bail!(
                            "Evolution signing key is missing while custom signed Nature state exists under {}; restore the original key or a consistent backup before restarting",
                            custom_natures.display()
                        );
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("inspecting {}", custom_natures.display()));
                }
            }
            let mut key = vec![0_u8; EVOLUTION_KEY_BYTES];
            getrandom::fill(&mut key).context("generating the local Evolution signing key")?;
            let mut temporary = NamedTempFile::new_in(&state_dir).with_context(|| {
                format!(
                    "creating temporary Evolution signing key in {}",
                    state_dir.display()
                )
            })?;
            restrict_file(temporary.as_file(), "temporary Evolution signing key")?;
            temporary.write_all(&key)?;
            temporary.as_file().sync_all()?;
            reject_unsafe_target(&path)?;
            match temporary.persist_noclobber(&path) {
                Ok(file) => {
                    restrict_file(&file, "Evolution signing key")?;
                    file.sync_all()?;
                    sync_directory(&state_dir)?;
                    Ok(key)
                }
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                    read_evolution_key(&path)
                }
                Err(error) => Err(error.error)
                    .with_context(|| format!("creating {} atomically", path.display())),
            }
        }
        Err(error) => Err(error),
    }
}

fn read_evolution_key(path: &Path) -> Result<Vec<u8>> {
    let mut file = open_read_no_follow(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "Evolution signing key must be a file");
    assert_owner_only(&metadata, "Evolution signing key")?;
    ensure!(
        metadata.len() == EVOLUTION_KEY_BYTES as u64,
        "Evolution signing key has an invalid length"
    );
    let mut key = Vec::with_capacity(EVOLUTION_KEY_BYTES);
    file.read_to_end(&mut key)?;
    ensure!(
        key.len() == EVOLUTION_KEY_BYTES,
        "Evolution signing key has an invalid length"
    );
    Ok(key)
}

fn normalize_gossip_hints(values: Vec<String>, local_id: &str) -> Result<Vec<String>> {
    let mut unique = BTreeSet::new();
    for value in values {
        for peer in value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            validate_runtime_id(peer, "gossip peer hint")?;
            ensure!(
                peer != local_id,
                "the local Tentacle cannot be its own gossip peer"
            );
            unique.insert(peer.to_owned());
        }
    }
    ensure!(
        unique.len() <= MAX_GOSSIP_PEERS,
        "too many gossip bootstrap peers"
    );
    Ok(unique.into_iter().collect())
}

fn validate_runtime_id(value: &str, description: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            }),
        "invalid {description}"
    );
    Ok(())
}

fn validate_skill_slug(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 64
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            }),
        "skill name must be a lowercase slug"
    );
    Ok(())
}

fn read_operator_skill(operator_root: &Path, name: &str) -> Result<String> {
    let canonical_operator = fs::canonicalize(operator_root)
        .with_context(|| format!("resolving operator root {}", operator_root.display()))?;
    let skills_root = operator_root.join("skills");
    let skills_metadata = fs::symlink_metadata(&skills_root)
        .with_context(|| format!("inspecting {}", skills_root.display()))?;
    ensure!(
        skills_metadata.is_dir() && !skills_metadata.file_type().is_symlink(),
        "operator skills root must be a real, non-symlink directory"
    );
    let canonical_skills = fs::canonicalize(&skills_root)
        .with_context(|| format!("resolving {}", skills_root.display()))?;
    ensure!(
        canonical_skills.starts_with(&canonical_operator),
        "operator skills root escapes the operator workspace"
    );
    let skill_directory = skills_root.join(name);
    let skill_directory_metadata = fs::symlink_metadata(&skill_directory)
        .with_context(|| format!("inspecting {}", skill_directory.display()))?;
    ensure!(
        skill_directory_metadata.is_dir() && !skill_directory_metadata.file_type().is_symlink(),
        "operator skill directory must be a real, non-symlink directory"
    );
    let path = skill_directory.join("SKILL.md");
    let metadata =
        fs::symlink_metadata(&path).with_context(|| format!("inspecting {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "operator skill must be a regular, non-symlink file"
    );
    let canonical_path =
        fs::canonicalize(&path).with_context(|| format!("resolving {}", path.display()))?;
    ensure!(
        canonical_path.starts_with(&canonical_skills),
        "operator skill escapes the skills root"
    );
    ensure!(
        metadata.len() <= MAX_SKILL_BYTES as u64,
        "operator skill exceeds the Hermes size limit"
    );
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::take(&mut file, MAX_SKILL_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_SKILL_BYTES,
        "operator skill exceeds the Hermes size limit"
    );
    String::from_utf8(bytes).context("operator skill must be UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPERATOR: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn startup_blocks_until_authenticated_confirmation_and_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions::default(),
        )
        .unwrap();
        assert!(!runtime.permits_normal_operation());
        let response = runtime
            .handle_operator_message(OPERATOR, "message-1", "YES")
            .unwrap()
            .unwrap();
        assert!(response.contains("NORMAL OPERATION IS NOW OPEN"));
        assert!(runtime.permits_normal_operation());
        drop(runtime);

        let resumed = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions::default(),
        )
        .unwrap();
        assert!(resumed.permits_normal_operation());
    }

    #[test]
    fn kill_blocks_without_terminating_and_local_force_can_recover() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions::default(),
        )
        .unwrap();
        let response = runtime
            .handle_operator_message(OPERATOR, "message-kill", "KILL")
            .unwrap()
            .unwrap();
        assert!(response.contains("DID NOT TERMINATE"));
        assert!(!runtime.permits_normal_operation());
        drop(runtime);

        let recovered = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                reroll_nature: true,
                force: true,
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();
        assert!(!recovered.permits_normal_operation());
        assert!(matches!(
            recovered.ritual.phase(),
            AwakeningPhase::AwaitingConfirmation
        ));
    }

    #[test]
    fn nature_changes_measurable_model_policy() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let mut sequence = 0_u32;
        for (nature_trait, value) in [
            (NatureTrait::Engagement, 0),
            (NatureTrait::Growth, 0),
            (NatureTrait::Wealth, 100),
            (NatureTrait::Influence, 0),
            (NatureTrait::Stability, 0),
            (NatureTrait::Transparency, 100),
        ] {
            if runtime.nature().value(nature_trait) != value {
                sequence += 1;
                runtime
                    .handle_operator_message(
                        OPERATOR,
                        &format!("policy-adjust-{sequence}"),
                        &format!("/adjust {nature_trait} {value}"),
                    )
                    .unwrap();
            }
        }
        let economical = runtime.model_policy();
        assert_eq!(economical.response_bias, ResponseBias::Economy);
        assert_eq!(economical.max_output_tokens, 100);
        assert!(economical.temperature > 1.0);
        assert!(
            economical
                .nature_runtime_facts
                .contains("give concise rationale")
        );

        for (nature_trait, value) in [
            (NatureTrait::Engagement, 100),
            (NatureTrait::Wealth, 0),
            (NatureTrait::Stability, 100),
        ] {
            if runtime.nature().value(nature_trait) != value {
                sequence += 1;
                runtime
                    .handle_operator_message(
                        OPERATOR,
                        &format!("policy-adjust-{sequence}"),
                        &format!("/adjust {nature_trait} {value}"),
                    )
                    .unwrap();
            }
        }
        let engaging = runtime.model_policy();
        assert_eq!(engaging.response_bias, ResponseBias::Engagement);
        assert_eq!(engaging.max_output_tokens, 300);
        assert!(engaging.temperature < 0.4);
    }

    #[test]
    fn gossip_hints_are_not_treated_as_authenticated_peers() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let runtime = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                skip_awakening: true,
                gossip_peers: vec!["peer-a,peer-b".to_owned()],
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();
        let status = runtime.gossip_status();
        assert!(status.contains("AUTHENTICATED DIRECT PEERS: 0"));
        assert!(status.contains("BOOTSTRAP HINTS AWAITING KEY BINDING: 2"));
        assert!(status.contains("LIVE NETWORK TRANSPORT: DISABLED"));
    }

    #[test]
    fn runtime_lock_enforces_one_writer_and_releases_on_drop() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let error = EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path())
            .err()
            .unwrap();
        assert!(error.to_string().contains("already owns"));
        drop(runtime);
        EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
    }

    #[test]
    fn custom_nature_path_is_confined_to_its_dedicated_state_subtree() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_nature = outside.path().join("nature.json");
        #[cfg(unix)]
        let permissions_before = {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(outside.path()).unwrap().permissions().mode() & 0o777
        };

        for path in [
            outside_nature.clone(),
            PathBuf::from("../nature.json"),
            PathBuf::new(),
        ] {
            let error = EvolutionRuntime::open(
                root.path(),
                workspace.path(),
                EvolutionStartupOptions {
                    nature_path: Some(path),
                    ..EvolutionStartupOptions::default()
                },
            )
            .err()
            .unwrap();
            assert!(error.to_string().contains("nature-path"));
        }
        assert!(!outside_nature.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(outside.path()).unwrap().permissions().mode() & 0o777,
                permissions_before
            );
        }
    }

    #[test]
    fn missing_key_never_silently_rekeys_existing_signed_state() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        drop(runtime);
        let key_path = root.path().join("state").join(EVOLUTION_KEY_FILE);
        fs::remove_file(&key_path).unwrap();

        let error = EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path())
            .err()
            .unwrap();
        assert!(error.to_string().contains("restore the original key"));
        assert!(!key_path.exists());
    }

    #[test]
    fn missing_key_never_adopts_orphaned_evolution_projections() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        drop(runtime);
        let state = root.path().join("state");
        for filename in [
            EVOLUTION_KEY_FILE,
            "nature.json",
            "awakening_log.md",
            "hermes_gossip.json",
            "lineage.json",
        ] {
            fs::remove_file(state.join(filename)).unwrap();
        }
        assert!(state.join("metrics.json").exists());

        let error = EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path())
            .err()
            .unwrap();
        assert!(error.to_string().contains("Evolution state exists"));
        assert!(!state.join(EVOLUTION_KEY_FILE).exists());
    }

    #[test]
    fn missing_pre_action_nature_never_rebinds_existing_projections() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let runtime = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions::default(),
        )
        .unwrap();
        assert!(runtime.awakening_log.entries().unwrap().is_empty());
        let nature_path = runtime.nature_store.path().to_path_buf();
        drop(runtime);
        fs::remove_file(&nature_path).unwrap();

        let error = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions::default(),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("Evolution projections exist"));
        assert!(!nature_path.exists());
    }

    #[test]
    fn restart_reconciles_an_unobserved_log_ahead_adjustment() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let value = if runtime.nature().growth == 100 {
            99
        } else {
            runtime.nature().growth + 1
        };
        let provenance =
            AwakeningProvenance::authenticated_xmtp(OPERATOR, "log-ahead-adjust").unwrap();
        runtime
            .ritual
            .adjust_after_confirmation(
                NatureTrait::Growth,
                value,
                now_unix_seconds().unwrap(),
                &provenance,
                &runtime.awakening_log,
            )
            .unwrap();
        let expected_fingerprint = runtime.nature().fingerprint().unwrap();
        drop(runtime);

        let resumed =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        assert_eq!(resumed.nature().growth, value);
        assert_eq!(resumed.metrics.nature_fingerprint, expected_fingerprint);
        assert_eq!(resumed.metrics.awakening_epoch, resumed.ritual.epoch());
        assert_eq!(resumed.metrics.nature_adjustment_stress_events, 1);
    }

    #[test]
    fn confirmation_resets_an_expired_empty_pending_period_without_judgment() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions::default(),
        )
        .unwrap();
        runtime.metrics.period_started_at_unix_seconds = 0;
        runtime.metrics.period_ends_at_unix_seconds = EvaluationPeriod::Daily.duration_seconds();
        runtime.scales_store.save_metrics(&runtime.metrics).unwrap();

        runtime
            .handle_operator_message(OPERATOR, "late-confirmation", "YES")
            .unwrap();
        assert!(runtime.permits_normal_operation());
        assert!(
            runtime.metrics.period_started_at_unix_seconds
                >= EvaluationPeriod::Daily.duration_seconds()
        );
        assert!(runtime.last_final_judgment.is_none());
        assert!(runtime.scales_store.load_history().unwrap().is_empty());
    }

    #[test]
    fn local_skip_resets_an_expired_pending_period_before_confirmation() {
        for reroll_nature in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let workspace = tempfile::tempdir().unwrap();
            let mut runtime = EvolutionRuntime::open(
                root.path(),
                workspace.path(),
                EvolutionStartupOptions::default(),
            )
            .unwrap();
            runtime.metrics.period_started_at_unix_seconds = 0;
            runtime.metrics.period_ends_at_unix_seconds =
                EvaluationPeriod::Daily.duration_seconds();
            runtime.scales_store.save_metrics(&runtime.metrics).unwrap();
            drop(runtime);

            let resumed = EvolutionRuntime::open(
                root.path(),
                workspace.path(),
                EvolutionStartupOptions {
                    skip_awakening: true,
                    reroll_nature,
                    force: reroll_nature,
                    ..EvolutionStartupOptions::default()
                },
            )
            .unwrap();
            assert!(resumed.permits_normal_operation());
            assert!(
                resumed.metrics.period_started_at_unix_seconds
                    >= EvaluationPeriod::Daily.duration_seconds()
            );
            assert!(resumed.last_final_judgment.is_none());
            assert!(resumed.scales_store.load_history().unwrap().is_empty());
        }
    }

    #[test]
    fn startup_replays_only_the_exact_history_ahead_metrics_window() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let finalized = new_runtime_metrics(
            EvaluationPeriod::Daily,
            0,
            runtime.nature(),
            runtime.ritual.epoch(),
        )
        .unwrap();
        let judgment = finalized
            .evaluate(runtime.nature(), finalized.period_ends_at_unix_seconds)
            .unwrap();
        let record =
            EvolutionHistoryRecord::new(runtime.nature(), finalized.clone(), judgment).unwrap();
        runtime.scales_store.append_history(&record).unwrap();
        runtime.scales_store.save_metrics(&finalized).unwrap();
        drop(runtime);

        let resumed =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        assert!(
            resumed.metrics.period_started_at_unix_seconds
                >= record.metrics.period_ends_at_unix_seconds
        );
        assert_ne!(resumed.metrics, record.metrics);
        assert_eq!(
            resumed.last_final_judgment.as_ref().unwrap().judgment_id,
            record.judgment_id
        );
    }

    #[test]
    fn startup_rejects_divergent_or_partially_overlapping_metrics_history() {
        for partially_overlapping in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let workspace = tempfile::tempdir().unwrap();
            let mut runtime =
                EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
            let finalized = runtime.metrics.clone();
            let judgment = finalized
                .evaluate(runtime.nature(), finalized.period_ends_at_unix_seconds)
                .unwrap();
            let record =
                EvolutionHistoryRecord::new(runtime.nature(), finalized.clone(), judgment).unwrap();
            runtime.scales_store.append_history(&record).unwrap();

            if partially_overlapping {
                runtime.metrics = new_runtime_metrics(
                    EvaluationPeriod::Daily,
                    finalized.period_ends_at_unix_seconds - 1,
                    runtime.nature(),
                    runtime.ritual.epoch(),
                )
                .unwrap();
            } else {
                runtime.metrics.record_conversation(1_000, true, Some(1));
            }
            runtime.scales_store.save_metrics(&runtime.metrics).unwrap();
            drop(runtime);

            let error = EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path())
                .err()
                .unwrap();
            assert!(error.to_string().contains("overlap finalized judgment"));
        }
    }

    #[test]
    fn rejected_no_commit_adjustments_do_not_degrade_the_runtime() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut pending = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions::default(),
        )
        .unwrap();
        let current = pending.nature().engagement;
        let delta = if current == 0 { -1 } else { 100 };
        assert!(
            pending
                .handle_operator_message(
                    OPERATOR,
                    "invalid-pending-adjust",
                    &format!("ADJUST engagement {delta:+}"),
                )
                .is_err()
        );
        assert!(!pending.degraded);
        drop(pending);

        let second_root = tempfile::tempdir().unwrap();
        let mut confirmed =
            EvolutionRuntime::open_confirmed_for_test(second_root.path(), workspace.path())
                .unwrap();
        let current = confirmed.nature().growth;
        assert!(
            confirmed
                .handle_operator_message(
                    OPERATOR,
                    "unchanged-confirmed-adjust",
                    &format!("/adjust growth {current}"),
                )
                .is_err()
        );
        assert!(confirmed.permits_normal_operation());
        assert!(!confirmed.degraded);
    }

    #[test]
    fn public_turn_binding_defers_adjustment_without_holding_a_mutex() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let PublicTurnStart::Ready(turn) = runtime.begin_public_turn().unwrap() else {
            panic!("confirmed runtime should reserve a public turn");
        };
        assert_eq!(turn.nature_cooperation, runtime.nature().cooperation);
        let value = if runtime.nature().growth == 100 {
            99
        } else {
            runtime.nature().growth + 1
        };
        let error = runtime
            .handle_operator_message(
                OPERATOR,
                "adjust-during-public-turn",
                &format!("/adjust growth {value}"),
            )
            .unwrap_err();
        assert!(error.to_string().contains("public turns"));
        runtime.finish_public_turn(turn.token, None).unwrap();
        assert!(
            runtime
                .handle_operator_message(
                    OPERATOR,
                    "adjust-after-public-turn",
                    &format!("/adjust growth {value}"),
                )
                .unwrap()
                .unwrap()
                .contains("NATURE ADJUSTED")
        );
    }

    #[test]
    fn public_turn_is_recorded_in_its_original_period_before_rollover() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let original_start = runtime.metrics.period_started_at_unix_seconds;
        let original_end = runtime.metrics.period_ends_at_unix_seconds;
        let PublicTurnStart::Ready(turn) = runtime.begin_public_turn().unwrap() else {
            panic!("confirmed runtime should reserve a public turn");
        };
        let after_close = u64::try_from(original_end).unwrap().saturating_add(1);
        assert!(
            runtime
                .roll_period_if_closed(after_close)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            runtime.metrics.period_started_at_unix_seconds,
            original_start
        );
        runtime
            .finish_public_turn(
                turn.token,
                Some(ConversationObservation {
                    depth: 2,
                    returning: false,
                    response_time_ms: Some(5),
                    token_engagement_bonus_basis_points: 0,
                }),
            )
            .unwrap();
        assert_eq!(runtime.metrics.engagement.conversations, 1);
        assert!(
            runtime
                .roll_period_if_closed(after_close)
                .unwrap()
                .is_some()
        );
        assert!(runtime.metrics.period_started_at_unix_seconds >= original_end);
        assert_eq!(runtime.metrics.engagement.conversations, 0);
    }

    #[test]
    fn public_token_observations_only_raise_engagement_and_survive_restart() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let expected_fingerprint = runtime.nature().fingerprint().unwrap();
        for bonus in [8_000, 0] {
            let PublicTurnStart::Ready(turn) = runtime.begin_public_turn().unwrap() else {
                panic!("confirmed runtime should reserve a public turn");
            };
            runtime
                .finish_public_turn(
                    turn.token,
                    Some(ConversationObservation {
                        depth: 0,
                        returning: false,
                        response_time_ms: None,
                        token_engagement_bonus_basis_points: bonus,
                    }),
                )
                .unwrap();
        }

        assert_eq!(
            runtime.metrics.scored_scale_availability,
            RUNTIME_SCORED_SCALES
        );
        assert!(runtime.metrics.token_economics.is_none());
        assert!(runtime.metrics.wealth.is_none());
        assert_eq!(runtime.metrics.nature_fingerprint, expected_fingerprint);
        assert_eq!(runtime.metrics.engagement.conversations, 2);
        assert_eq!(
            runtime.metrics.engagement.token_bonus_basis_points_total,
            8_000
        );
        let judgment = runtime
            .metrics
            .evaluate(
                runtime.nature(),
                runtime.metrics.period_ends_at_unix_seconds,
            )
            .unwrap();
        assert_eq!(judgment.scores.engagement, 4_000);
        assert!(!judgment.scored_scale_availability.growth);
        assert!(!judgment.scored_scale_availability.wealth);
        assert!(!judgment.scored_scale_availability.influence);
        assert_eq!(judgment.economic_starvation_relief_basis_points, 0);
        assert_eq!(
            runtime.scales_store.load_metrics().unwrap(),
            Some(runtime.metrics.clone())
        );

        let expected_metrics = runtime.metrics.clone();
        drop(runtime);
        let resumed =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        assert_eq!(resumed.metrics, expected_metrics);
        assert!(resumed.metrics.token_economics.is_none());
    }

    #[test]
    fn zero_public_token_bonus_contributes_to_the_period_denominator() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let PublicTurnStart::Ready(turn) = runtime.begin_public_turn().unwrap() else {
            panic!("confirmed runtime should reserve a public turn");
        };
        runtime
            .finish_public_turn(
                turn.token,
                Some(ConversationObservation {
                    depth: 1,
                    returning: false,
                    response_time_ms: None,
                    token_engagement_bonus_basis_points: 0,
                }),
            )
            .unwrap();
        assert_eq!(
            runtime.metrics.scored_scale_availability,
            RUNTIME_SCORED_SCALES
        );
        assert!(runtime.metrics.token_economics.is_none());
        assert_eq!(runtime.metrics.engagement.conversations, 1);
        assert_eq!(runtime.metrics.engagement.token_bonus_basis_points_total, 0);
    }

    #[cfg(unix)]
    #[test]
    fn ambiguous_projection_failure_sticks_in_fail_closed_mode() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside-nature");
        fs::write(&outside, "do not replace").unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let nature_path = runtime.nature_store.path().to_path_buf();
        fs::remove_file(&nature_path).unwrap();
        symlink(&outside, &nature_path).unwrap();
        let value = if runtime.nature().growth == 100 {
            99
        } else {
            runtime.nature().growth + 1
        };

        assert!(
            runtime
                .handle_operator_message(
                    OPERATOR,
                    "ambiguous-adjust",
                    &format!("/adjust growth {value}"),
                )
                .is_err()
        );
        assert!(!runtime.permits_normal_operation());
        assert!(matches!(
            runtime.begin_public_turn().unwrap(),
            PublicTurnStart::Gated(_)
        ));
        let recovery = runtime
            .handle_operator_message(OPERATOR, "blocked-effect", "/exec true")
            .unwrap()
            .unwrap();
        assert!(recovery.contains("FAIL-CLOSED"));
        assert_eq!(fs::read_to_string(outside).unwrap(), "do not replace");
    }

    #[test]
    fn forced_epoch_reroll_resets_prior_adjustment_stress() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        runtime.metrics.record_nature_adjustment_stress();
        runtime.scales_store.save_metrics(&runtime.metrics).unwrap();
        drop(runtime);

        let rerolled = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                reroll_nature: true,
                force: true,
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();
        assert_eq!(rerolled.metrics.nature_adjustment_stress_events, 0);
    }

    #[test]
    fn lineage_spawn_receipts_must_resolve_to_their_exact_final_grants() {
        let mut nature = TentacleNature::random().unwrap();
        nature.sacred_ban = SacredBan::MemorySharing;
        let mut metrics = new_runtime_metrics(EvaluationPeriod::Daily, 0, &nature, 1).unwrap();
        for index in 0..DAILY_PROPAGATION_MIN_CONVERSATIONS {
            metrics.record_conversation(
                1_000,
                index < DAILY_PROPAGATION_MIN_RETURNING_CONVERSATIONS,
                Some(1),
            );
        }
        let judgment = metrics
            .evaluate(&nature, metrics.period_ends_at_unix_seconds)
            .unwrap();
        let grant = EvolutionHistoryRecord::new(&nature, metrics, judgment).unwrap();
        let authorization = SpawnAuthorization {
            judgment_id: grant.judgment_id.clone(),
            operator_id: OPERATOR.to_owned(),
            event_id_sha256: "b".repeat(64),
        };
        let valid_spawn_at =
            u64::try_from(grant.metrics.period_ends_at_unix_seconds).unwrap() * 1_000;

        let mut valid = Lineage::new("parent", nature.clone(), 0).unwrap();
        valid
            .spawn_child(
                "parent",
                "parent",
                "valid-child",
                valid_spawn_at,
                authorization.clone(),
            )
            .unwrap();
        validate_lineage_spawn_authorizations(&valid, std::slice::from_ref(&grant)).unwrap();
        assert!(validate_lineage_spawn_authorizations(&valid, &[]).is_err());

        let mut other_nature = TentacleNature::random().unwrap();
        other_nature.sacred_ban = SacredBan::MemorySharing;
        let mut wrong_parent = Lineage::new("parent", other_nature, 0).unwrap();
        wrong_parent
            .spawn_child(
                "parent",
                "parent",
                "wrong-parent-child",
                valid_spawn_at,
                authorization.clone(),
            )
            .unwrap();
        assert!(
            validate_lineage_spawn_authorizations(&wrong_parent, std::slice::from_ref(&grant))
                .is_err()
        );

        let mut expired = Lineage::new("parent", nature, 0).unwrap();
        let invalid_spawn_at = u64::try_from(
            grant.metrics.period_ends_at_unix_seconds + grant.metrics.period.duration_seconds(),
        )
        .unwrap()
            * 1_000;
        expired
            .spawn_child(
                "parent",
                "parent",
                "expired-child",
                invalid_spawn_at,
                authorization,
            )
            .unwrap();
        assert!(
            validate_lineage_spawn_authorizations(&expired, std::slice::from_ref(&grant)).is_err()
        );
    }

    #[test]
    fn startup_rejects_lineage_with_an_unverifiable_spawn_receipt() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let root_id = runtime.local_tentacle_id.clone();
        let mut nature = runtime.nature().clone();
        nature.sacred_ban = SacredBan::MemorySharing;
        let mut forged = Lineage::new(root_id.clone(), nature, 0).unwrap();
        forged
            .spawn_child(
                &root_id,
                &root_id,
                "unverified-child",
                now_unix_seconds().unwrap().saturating_mul(1_000),
                SpawnAuthorization {
                    judgment_id: "c".repeat(64),
                    operator_id: OPERATOR.to_owned(),
                    event_id_sha256: "d".repeat(64),
                },
            )
            .unwrap();
        runtime.lineage_store.save(&forged).unwrap();
        drop(runtime);

        let error = EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path())
            .err()
            .unwrap();
        assert!(error.to_string().contains("missing final judgment"));
    }

    #[test]
    fn propagation_grants_require_bounded_volume_returns_and_current_policy() {
        let nature = TentacleNature::random().unwrap();
        let mut low_sample = new_runtime_metrics(EvaluationPeriod::Daily, 0, &nature, 1).unwrap();
        low_sample.record_conversation(1_000, true, Some(1));
        let judgment = low_sample
            .evaluate(&nature, low_sample.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(judgment.outcome, JudgmentOutcome::Survival);
        assert!(!judgment.propagation_evidence.eligible);
        let record = EvolutionHistoryRecord::new(&nature, low_sample, judgment).unwrap();
        let current = new_runtime_metrics(
            EvaluationPeriod::Daily,
            record.metrics.period_ends_at_unix_seconds,
            &nature,
            1,
        )
        .unwrap();
        assert!(
            !is_accepted_propagation_grant(
                &record,
                &current,
                &nature,
                1,
                current.period_started_at_unix_seconds + 1,
            )
            .unwrap()
        );

        let mut eligible = new_runtime_metrics(EvaluationPeriod::Daily, 0, &nature, 1).unwrap();
        for index in 0..DAILY_PROPAGATION_MIN_CONVERSATIONS {
            eligible.record_conversation(
                1_000,
                index < DAILY_PROPAGATION_MIN_RETURNING_CONVERSATIONS,
                Some(1),
            );
        }
        let judgment = eligible
            .evaluate(&nature, eligible.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(judgment.outcome, JudgmentOutcome::PropagationRights);
        let record = EvolutionHistoryRecord::new(&nature, eligible, judgment).unwrap();
        let current = new_runtime_metrics(
            EvaluationPeriod::Daily,
            record.metrics.period_ends_at_unix_seconds,
            &nature,
            1,
        )
        .unwrap();
        assert!(
            is_accepted_propagation_grant(
                &record,
                &current,
                &nature,
                1,
                current.period_started_at_unix_seconds + 1,
            )
            .unwrap()
        );

        let mut token_enabled =
            new_runtime_metrics(EvaluationPeriod::Daily, 0, &nature, 1).unwrap();
        let targets = JudgmentPolicy::for_period(EvaluationPeriod::Daily).targets;
        for index in 0..DAILY_PROPAGATION_MIN_CONVERSATIONS {
            token_enabled.record_conversation(
                targets.average_conversation_depth,
                index < DAILY_PROPAGATION_MIN_RETURNING_CONVERSATIONS,
                Some(targets.response_time_full_credit_ms),
            );
        }
        token_enabled
            .record_token_economic_snapshot(
                TokenEconomicSnapshot {
                    balance_basis_points: 8_000,
                    stake_basis_points: 0,
                    reward_basis_points: 0,
                    trustworthy: true,
                },
                runtime_token_policy(&nature).unwrap(),
            )
            .unwrap();
        token_enabled.record_growth(
            targets.children_spawned,
            targets.acolytes_recruited,
            targets.network_contribution_points,
        );
        token_enabled
            .record_economic_result(targets.revenue_micro_units, targets.efficiency_basis_points)
            .unwrap();
        token_enabled.record_influence(
            targets.governance_participation,
            targets.sibling_influence_points,
        );
        let token_judgment = token_enabled
            .evaluate(&nature, token_enabled.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(token_judgment.outcome, JudgmentOutcome::PropagationRights);
        let token_record =
            EvolutionHistoryRecord::new(&nature, token_enabled, token_judgment).unwrap();
        assert_eq!(
            token_record.metrics.scored_scale_availability,
            ScoredScaleAvailability {
                engagement: true,
                growth: false,
                wealth: true,
                influence: false,
            }
        );
        let token_current = new_runtime_metrics(
            EvaluationPeriod::Daily,
            token_record.metrics.period_ends_at_unix_seconds,
            &nature,
            1,
        )
        .unwrap();
        assert!(
            is_accepted_propagation_grant(
                &token_record,
                &token_current,
                &nature,
                1,
                token_current.period_started_at_unix_seconds + 1,
            )
            .unwrap()
        );

        let mut arbitrary_availability_metrics = token_record.metrics.clone();
        arbitrary_availability_metrics
            .scored_scale_availability
            .growth = true;
        let arbitrary_availability_judgment = arbitrary_availability_metrics
            .evaluate(
                &nature,
                arbitrary_availability_metrics.period_ends_at_unix_seconds,
            )
            .unwrap();
        let arbitrary_token_availability = EvolutionHistoryRecord::new(
            &nature,
            arbitrary_availability_metrics,
            arbitrary_availability_judgment,
        )
        .unwrap();
        arbitrary_token_availability.validate().unwrap();
        assert!(
            !is_accepted_propagation_grant(
                &arbitrary_token_availability,
                &token_current,
                &nature,
                1,
                token_current.period_started_at_unix_seconds + 1,
            )
            .unwrap()
        );

        let mut arbitrary_policy = runtime_token_policy(&nature).unwrap();
        arbitrary_policy.engagement_sensitivity_basis_points =
            if arbitrary_policy.engagement_sensitivity_basis_points == 10_000 {
                9_999
            } else {
                arbitrary_policy.engagement_sensitivity_basis_points + 1
            };
        let mut arbitrary_policy_metrics = token_record.metrics.clone();
        arbitrary_policy_metrics
            .record_token_economic_snapshot(
                token_record.metrics.token_economics.unwrap().snapshot,
                arbitrary_policy,
            )
            .unwrap();
        let arbitrary_policy_judgment = arbitrary_policy_metrics
            .evaluate(
                &nature,
                arbitrary_policy_metrics.period_ends_at_unix_seconds,
            )
            .unwrap();
        let mismatched_token_policy = EvolutionHistoryRecord::new(
            &nature,
            arbitrary_policy_metrics,
            arbitrary_policy_judgment,
        )
        .unwrap();
        mismatched_token_policy.validate().unwrap();
        assert!(
            !is_accepted_propagation_grant(
                &mismatched_token_policy,
                &token_current,
                &nature,
                1,
                token_current.period_started_at_unix_seconds + 1,
            )
            .unwrap()
        );

        let mut legacy_policy = record.clone();
        legacy_policy.metrics.scored_scale_availability.growth = true;
        legacy_policy.judgment.scored_scale_availability.growth = true;
        assert!(
            !is_accepted_propagation_grant(
                &legacy_policy,
                &current,
                &nature,
                1,
                current.period_started_at_unix_seconds + 1,
            )
            .unwrap()
        );
        let stale_cycle = new_runtime_metrics(
            EvaluationPeriod::Daily,
            current.period_ends_at_unix_seconds,
            &nature,
            1,
        )
        .unwrap();
        assert!(
            !is_accepted_propagation_grant(
                &record,
                &stale_cycle,
                &nature,
                1,
                stale_cycle.period_started_at_unix_seconds + 1,
            )
            .unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn operator_skill_reader_rejects_symlinked_roots_and_directories() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(outside.path().join("demo")).unwrap();
        fs::write(outside.path().join("demo/SKILL.md"), "safe text").unwrap();
        symlink(outside.path(), workspace.path().join("skills")).unwrap();
        assert!(read_operator_skill(workspace.path(), "demo").is_err());

        fs::remove_file(workspace.path().join("skills")).unwrap();
        fs::create_dir(workspace.path().join("skills")).unwrap();
        symlink(
            outside.path().join("demo"),
            workspace.path().join("skills/demo"),
        )
        .unwrap();
        assert!(read_operator_skill(workspace.path(), "demo").is_err());
    }
}
