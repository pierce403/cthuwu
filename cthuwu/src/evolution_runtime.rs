#[cfg(test)]
use crate::evolution::DEATH_GRACE_PERIOD_MS;
#[cfg(test)]
use crate::personality::SacredBan;
use crate::{
    awakening::{
        AwakeningAction, AwakeningLog, AwakeningOutcome, AwakeningPhase, AwakeningProvenance,
        AwakeningRitual,
    },
    economics::{
        DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS, EconomicHolderRole, EconomicObservationProvenance,
        NORMALIZED_ECONOMIC_MAX, TokenEconomicEffects, TokenEconomicPolicy, TokenEconomicSnapshot,
    },
    evolution::{
        AcolyteContributionKind, LIFECYCLE_RECEIPT_CLOCK_SKEW_MS, LifecycleAction, LifecycleIntent,
        LifecycleReceipt, LifecycleReceiptStatus, LifecycleState, LifecycleStore, Lineage,
        LineageStore, SpawnAuthorization, SurvivalSpendBinding, TentacleLifecycle,
        WholeTokenAmount, exact_raw_token_amount, exact_whole_token_amount,
    },
    hermes::{
        HermesNode, HermesStore, KnowledgeItem, KnowledgePayload, MAX_GOSSIP_PEERS,
        MAX_SKILL_BYTES, OperatorSkill, SignatureAuthority, SigningIdentity, TrustedKeyring,
    },
    model::{ModelPolicy, ResponseBias},
    personality::{
        NatureStore, NatureTrait, TentacleNature, assert_owner_only, open_read_no_follow,
        reject_unsafe_target,
    },
    scales::{
        EvaluationPeriod, EvaluationStatus, EvolutionHistoryRecord, Judgment, JudgmentOutcome,
        JudgmentPolicy, ScalesStore, ScoredScaleAvailability, TentacleMetrics,
        ValidatedHistoryCatalog,
    },
    storage::{ensure_private_directory, restrict_file, sync_directory},
};
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Write},
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MandatoryRecoveryKind {
    None,
    CompletedShutdown,
    AbsorptionProjectionRequired,
    ShutdownDueOrPending,
    AbsorptionRequired,
}

#[derive(Clone, Debug)]
pub struct EvolutionStartupOptions {
    pub skip_awakening: bool,
    /// Accept a generated or legacy-pending Nature locally so ordinary chat never requires an
    /// operator. Explicit testing skip remains separately audited.
    pub auto_accept_nature: bool,
    pub reroll_nature: bool,
    pub force: bool,
    pub nature_path: Option<PathBuf>,
    pub gossip_peers: Vec<String>,
    /// `None` keeps persisted policy; a fresh Tentacle defaults to automatic spawning.
    pub auto_spawn: Option<bool>,
    pub propagation_minimum_stake_basis_points: u16,
    pub require_node_economics: bool,
    pub node_economics_ttl_seconds: u64,
    pub initial_node_economics: Option<(TokenEconomicSnapshot, EconomicObservationProvenance)>,
    pub child_bootstrap: Option<ChildBootstrap>,
    pub survival_total_supply_whole: u64,
    pub survival_token_decimals: u8,
}

impl Default for EvolutionStartupOptions {
    fn default() -> Self {
        Self {
            skip_awakening: false,
            auto_accept_nature: false,
            reroll_nature: false,
            force: false,
            nature_path: None,
            gossip_peers: Vec::new(),
            auto_spawn: None,
            propagation_minimum_stake_basis_points: DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS,
            require_node_economics: false,
            node_economics_ttl_seconds: 120,
            initial_node_economics: None,
            child_bootstrap: None,
            survival_total_supply_whole: 100_000_000_000,
            survival_token_decimals: 18,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChildBootstrap {
    pub provisioning_action_id: String,
    pub tentacle_id: String,
    pub parent_id: String,
    pub inherited_nature: TentacleNature,
}

/// Owns the local Evolution state machines and emits durable lifecycle actions for an external
/// executor. Process control, token transactions, provisioning, and memory transfer never occur
/// inside this state machine.
pub struct EvolutionRuntime {
    _runtime_lock: File,
    operator_root: PathBuf,
    nature_store: NatureStore,
    awakening_log: AwakeningLog,
    ritual: AwakeningRitual,
    scales_store: ScalesStore,
    history_catalog: ValidatedHistoryCatalog,
    metrics: TentacleMetrics,
    last_final_judgment: Option<EvolutionHistoryRecord>,
    lineage_store: LineageStore,
    lineage: Lineage,
    local_tentacle_id: String,
    lifecycle_store: LifecycleStore,
    lifecycle: LifecycleState,
    propagation_minimum_stake_basis_points: u16,
    require_node_economics: bool,
    node_economics_ttl_seconds: u64,
    node_economics_available: bool,
    survival_total_supply_whole: u64,
    survival_token_decimals: u8,
    hermes_store: HermesStore,
    hermes: HermesNode,
    operator_identity: SigningIdentity,
    gossip_bootstrap_hints: Vec<String>,
    active_public_turns: BTreeMap<u64, PublicTurnBinding>,
    next_public_turn_id: u64,
    public_dormant_turns: u64,
    operator_dormant_turns: u64,
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
    /// Converts the retired terminal low-score policy before startup recovery can finalize or
    /// refuse a restart. Durable identity, Nature, history, and old audit receipts are preserved.
    pub fn migrate_legacy_death_to_dormancy(data_dir: &Path) -> Result<bool> {
        let lifecycle_store = LifecycleStore::new(data_dir)?;
        let Some(lifecycle) = lifecycle_store.load()? else {
            return Ok(false);
        };
        lifecycle.validate()?;
        let Some(pending) = lifecycle.pending_death.as_ref() else {
            return Ok(false);
        };
        let history = ScalesStore::new(data_dir)?.history_catalog()?;
        let Some(record) = history.get(&pending.judgment_id)? else {
            // Explicit awakening KILL and other non-Scales terminal actions remain authoritative.
            return Ok(false);
        };
        ensure!(
            record.judgment.outcome == JudgmentOutcome::Death
                && record.judgment.evaluation_status == EvaluationStatus::Final,
            "legacy terminal migration requires its exact final low-score Death judgment"
        );
        let _runtime_lock = acquire_evolution_lock(data_dir)?;
        let Some(mut lifecycle) = lifecycle_store.load()? else {
            return Ok(false);
        };
        lifecycle.validate()?;
        let changed = lifecycle.retire_legacy_death_as_dormancy()?;
        if changed {
            lifecycle_store.save(&lifecycle)?;
        }
        Ok(changed)
    }

    /// Stable identity of this independently operated Tentacle. Incarnation restarts reuse it;
    /// the singular Cthuwu collective has no local identity of its own.
    pub fn local_tentacle_id(&self) -> &str {
        &self.local_tentacle_id
    }

    /// Read-only preflight used before an RPC failure is allowed to open mutable Evolution state.
    /// Missing state is not recovery work; malformed, symlinked, or overly broad-permission state
    /// is rejected without creating directories or changing modes.
    pub fn has_mandatory_recovery_work(data_dir: &Path) -> Result<bool> {
        Ok(Self::mandatory_recovery_kind(data_dir)? != MandatoryRecoveryKind::None)
    }

    /// Classifies which recovery dependency must be available before mutable startup. Absorption
    /// needs the configured external executor; pending/native Shutdown and terminal exit do not
    /// require an economic signer or Base RPC.
    pub fn mandatory_recovery_kind(data_dir: &Path) -> Result<MandatoryRecoveryKind> {
        let data_metadata = match fs::symlink_metadata(data_dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(MandatoryRecoveryKind::None);
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(
            data_metadata.is_dir() && !data_metadata.file_type().is_symlink(),
            "Evolution data path must be a real directory"
        );
        assert_owner_only(&data_metadata, "Evolution data directory")?;

        let state_directory = data_dir.join("state");
        let state_metadata = match fs::symlink_metadata(&state_directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(MandatoryRecoveryKind::None);
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(
            state_metadata.is_dir() && !state_metadata.file_type().is_symlink(),
            "Evolution state path must be a real directory"
        );
        assert_owner_only(&state_metadata, "Evolution state directory")?;

        let path = state_directory.join("lifecycle.json");
        let path_metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(MandatoryRecoveryKind::None);
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(
            path_metadata.is_file() && !path_metadata.file_type().is_symlink(),
            "Evolution lifecycle path must be a regular file"
        );
        assert_owner_only(&path_metadata, "Evolution lifecycle state")?;
        let file = open_read_no_follow(&path)?;
        let opened_metadata = file.metadata()?;
        ensure!(
            opened_metadata.is_file(),
            "Evolution lifecycle path must remain a regular file"
        );
        assert_owner_only(&opened_metadata, "Evolution lifecycle state")?;
        let lifecycle: LifecycleState = serde_json::from_reader(BufReader::new(file))?;
        lifecycle.validate()?;
        if lifecycle.shutdown_completed_at_ms.is_some()
            && lifecycle.has_unapplied_absorption_projection()
        {
            return Ok(MandatoryRecoveryKind::AbsorptionProjectionRequired);
        }
        if lifecycle.shutdown_completed_at_ms.is_some() {
            return Ok(MandatoryRecoveryKind::CompletedShutdown);
        }
        let now_ms: u64 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock predates the Unix epoch during recovery preflight")?
            .as_millis()
            .try_into()
            .context("recovery preflight timestamp exceeds the lifecycle range")?;
        if lifecycle
            .pending_death
            .as_ref()
            .is_some_and(|death| now_ms >= death.grace_ends_at_ms)
        {
            return Ok(MandatoryRecoveryKind::ShutdownDueOrPending);
        }
        let has_shutdown = lifecycle.intents.values().any(|intent| {
            lifecycle.receipt(&intent.action_id).is_none()
                && !lifecycle.canceled_action_ids.contains(&intent.action_id)
                && matches!(intent.action, LifecycleAction::Shutdown { .. })
        });
        if has_shutdown {
            return Ok(MandatoryRecoveryKind::ShutdownDueOrPending);
        }
        let has_absorption = lifecycle.intents.values().any(|intent| {
            lifecycle.receipt(&intent.action_id).is_none()
                && !lifecycle.canceled_action_ids.contains(&intent.action_id)
                && matches!(intent.action, LifecycleAction::Absorb { .. })
        });
        if has_absorption {
            return Ok(MandatoryRecoveryKind::AbsorptionRequired);
        }
        if lifecycle.pending_death.is_some() {
            return Ok(MandatoryRecoveryKind::ShutdownDueOrPending);
        }
        Ok(MandatoryRecoveryKind::None)
    }

    /// Completes an already-due native Shutdown without opening Nature, metrics, economics, or
    /// transport state. This is the fixed-deadline startup path for a process whose transport was
    /// never started. It remains serialized by the Evolution writer lock and validates the full
    /// lifecycle schema plus the exact local Tentacle/death binding before atomically persisting
    /// the controller receipt.
    pub fn complete_due_native_shutdown(
        data_dir: &Path,
        now_unix_seconds: u64,
    ) -> Result<LifecycleReceipt> {
        Self::try_complete_due_native_shutdown(data_dir, now_unix_seconds)?
            .context("native Shutdown cannot complete before the binding Death grace deadline")
    }

    /// Non-stringly startup probe for the fixed-deadline native Shutdown path. `None` means no
    /// local Shutdown is due at `now_unix_seconds`; `Some` is the newly persisted receipt or the
    /// exact receipt from an already completed idempotent replay.
    pub fn try_complete_due_native_shutdown(
        data_dir: &Path,
        now_unix_seconds: u64,
    ) -> Result<Option<LifecycleReceipt>> {
        if Self::mandatory_recovery_kind(data_dir)? == MandatoryRecoveryKind::None {
            return Ok(None);
        }
        let _runtime_lock = acquire_evolution_lock(data_dir)?;
        if Self::mandatory_recovery_kind(data_dir)? == MandatoryRecoveryKind::None {
            return Ok(None);
        }

        let lifecycle_store = LifecycleStore::new(data_dir)?;
        let mut lifecycle = lifecycle_store
            .load()?
            .context("lifecycle-only native Shutdown requires persisted lifecycle state")?;
        lifecycle.validate()?;
        let tracking_changed = lifecycle.reconcile_absorption_projection_tracking()?;

        if let Some(completed_at_ms) = lifecycle.shutdown_completed_at_ms {
            let pending = lifecycle
                .pending_death
                .as_ref()
                .context("completed native Shutdown has no binding pending Death")?;
            let receipt = lifecycle
                .receipts
                .iter()
                .find(|receipt| {
                    receipt.completed_at_ms == completed_at_ms
                        && receipt.status == LifecycleReceiptStatus::Succeeded
                        && lifecycle
                            .intents
                            .get(&receipt.action_id)
                            .is_some_and(|intent| {
                                matches!(
                                    &intent.action,
                                    LifecycleAction::Shutdown {
                                        tentacle_id,
                                        judgment_id,
                                        ..
                                    } if tentacle_id == &lifecycle.tentacle_id
                                        && judgment_id == &pending.judgment_id
                                )
                            })
                })
                .cloned()
                .context("completed native Shutdown lacks an exact local controller receipt")?;
            if tracking_changed {
                lifecycle_store.save(&lifecycle)?;
            }
            return Ok(Some(receipt));
        }

        let now_ms = now_unix_seconds
            .checked_mul(1_000)
            .context("native Shutdown timestamp exceeds the lifecycle range")?;
        let pending = lifecycle
            .pending_death
            .clone()
            .context("lifecycle recovery work has no binding pending Death")?;
        if now_ms < pending.grace_ends_at_ms {
            if tracking_changed {
                lifecycle_store.save(&lifecycle)?;
            }
            return Ok(None);
        }

        lifecycle.reconcile_expired_death(now_ms, None)?;
        let absorption_action_id = lifecycle.intents.values().find_map(|intent| {
            matches!(
                &intent.action,
                LifecycleAction::Absorb { source_id, judgment_id, .. }
                    if source_id == &lifecycle.tentacle_id
                        && judgment_id == &pending.judgment_id
            )
            .then(|| intent.action_id.clone())
        });
        let expected_action = LifecycleAction::Shutdown {
            tentacle_id: lifecycle.tentacle_id.clone(),
            judgment_id: pending.judgment_id,
            after_action_id: absorption_action_id,
        };
        let intent = lifecycle
            .intents
            .values()
            .find(|intent| intent.action == expected_action)
            .cloned()
            .context("expired Death did not produce its exact canonical native Shutdown")?;
        ensure!(
            !lifecycle.canceled_action_ids.contains(&intent.action_id),
            "exact native Shutdown action is canceled"
        );
        ensure!(
            lifecycle.receipt(&intent.action_id).is_none(),
            "exact native Shutdown action already has a terminal non-success receipt"
        );
        ensure!(
            intent.created_at_ms <= now_ms,
            "exact native Shutdown intent was created after the controller timestamp"
        );

        let receipt = LifecycleReceipt {
            action_id: intent.action_id,
            completed_at_ms: now_ms,
            status: LifecycleReceiptStatus::Succeeded,
            external_reference: Some("native-transport-never-started".to_owned()),
            detail: Some("fixed-deadline startup shutdown".to_owned()),
            confirmed_chain_receipt: None,
            confirmed_transfer_receipt: None,
            provision_receipt: None,
        };
        let (action, changed) = lifecycle.acknowledge_action(receipt.clone())?;
        ensure!(
            changed && action == expected_action,
            "native Shutdown receipt was not applied"
        );
        lifecycle_store.save(&lifecycle)?;
        Ok(Some(receipt))
    }

    /// Repairs the local lineage half of a successful absorption after a crash between receipt
    /// persistence and projection. This recovery boundary loads only lifecycle and lineage state;
    /// it does not inspect Nature, metrics, economics, operator roots, or current CLI policy.
    pub fn repair_absorption_projection(data_dir: &Path) -> Result<bool> {
        if Self::mandatory_recovery_kind(data_dir)?
            != MandatoryRecoveryKind::AbsorptionProjectionRequired
        {
            return Ok(false);
        }
        let _runtime_lock = acquire_evolution_lock(data_dir)?;
        if Self::mandatory_recovery_kind(data_dir)?
            != MandatoryRecoveryKind::AbsorptionProjectionRequired
        {
            return Ok(false);
        }

        let lifecycle_store = LifecycleStore::new(data_dir)?;
        let mut lifecycle = lifecycle_store
            .load()?
            .context("absorption projection repair requires persisted lifecycle state")?;
        lifecycle.validate()?;
        lifecycle.reconcile_absorption_projection_tracking()?;
        let shutdown_completed_at_ms = lifecycle
            .shutdown_completed_at_ms
            .context("absorption projection repair requires completed native Shutdown")?;
        let pending_death = lifecycle
            .pending_death
            .clone()
            .context("absorption projection repair requires its binding pending Death")?;
        ensure!(
            shutdown_completed_at_ms >= pending_death.grace_ends_at_ms,
            "completed native Shutdown predates its binding Death deadline"
        );
        let repair_observed_at_ms = now_unix_seconds()?
            .checked_mul(1_000)
            .context("absorption repair observation timestamp exceeds the range")?;

        let lineage_store = LineageStore::new(data_dir)?;
        let mut lineage = lineage_store
            .load()?
            .context("absorption projection repair requires persisted lineage state")?;
        ensure!(
            lifecycle.tentacle_id == lineage.state().root_id,
            "lifecycle and lineage state belong to different local Tentacles"
        );
        let family = lineage.family(&lifecycle.tentacle_id)?;
        let valid_targets = family
            .parent
            .into_iter()
            .chain(family.siblings)
            .chain(family.children)
            .collect::<BTreeSet<_>>();
        let pending_action_ids = lifecycle
            .pending_absorption_projection_action_ids
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        ensure!(
            !pending_action_ids.is_empty(),
            "absorption projection preflight has no exact pending action"
        );
        let projections = pending_action_ids
            .iter()
            .map(|action_id| {
                let intent = lifecycle
                    .intents
                    .get(action_id)
                    .context("pending absorption projection references a missing intent")?;
                let receipt = lifecycle
                    .receipt(action_id)
                    .context("pending absorption projection references a missing receipt")?;
                let LifecycleAction::Absorb {
                    source_id,
                    target_id,
                    judgment_id,
                } = &intent.action
                else {
                    bail!("pending absorption projection references a non-Absorb action")
                };
                ensure!(
                    source_id == &lifecycle.tentacle_id
                        && valid_targets.contains(target_id)
                        && judgment_id == &pending_death.judgment_id
                        && receipt.status == LifecycleReceiptStatus::Succeeded
                        && !lifecycle.canceled_action_ids.contains(action_id),
                    "pending absorption projection is not bound to the exact local Death and lineage target"
                );
                Ok((action_id.clone(), intent.action.clone(), receipt.clone()))
            })
            .collect::<Result<Vec<_>>>()?;

        let mut lineage_changed = false;
        let mut applied = Vec::with_capacity(projections.len());
        for (action_id, action, receipt) in projections {
            let (projected_at_ms, changed) = apply_absorption_lineage_projection(
                &mut lineage,
                &action,
                &receipt,
                pending_death.grace_ends_at_ms,
                repair_observed_at_ms,
            )?;
            lineage_changed |= changed;
            applied.push((action_id, projected_at_ms));
        }
        if lineage_changed {
            lineage_store.save(&lineage)?;
        }
        let mut lifecycle_changed = false;
        for (action_id, projected_at_ms) in applied {
            lifecycle_changed |=
                lifecycle.record_absorption_projection(&action_id, projected_at_ms)?;
        }
        if lifecycle_changed {
            lifecycle_store.save(&lifecycle)?;
        }
        ensure!(
            !lifecycle.has_unapplied_absorption_projection(),
            "absorption projection repair left lifecycle work unapplied"
        );
        Ok(lineage_changed || lifecycle_changed)
    }

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
        ensure!(
            options.propagation_minimum_stake_basis_points <= 10_000,
            "propagation minimum stake exceeds 10000 basis points"
        );
        ensure!(
            !options.require_node_economics || options.node_economics_ttl_seconds > 0,
            "required node economics must have a positive freshness TTL"
        );
        ensure!(
            options.survival_total_supply_whole > 0 && options.survival_token_decimals <= 77,
            "survival spend normalization requires positive supply and ERC-20 decimals <= 77"
        );
        exact_raw_token_amount(
            options.survival_total_supply_whole,
            options.survival_token_decimals,
            10_000,
        )?;
        ensure_private_directory(data_dir)?;
        let runtime_lock = acquire_evolution_lock(data_dir)?;
        let now = now_unix_seconds()?;
        let nature_path = resolve_nature_path(data_dir, options.nature_path.as_deref())?;
        let signing_key = load_or_create_evolution_key(data_dir, &nature_path)?;
        let nature_store = NatureStore::with_path(nature_path.clone(), &signing_key)?;
        let awakening_log = AwakeningLog::new(data_dir, &signing_key)?;
        let lifecycle_store = LifecycleStore::new(data_dir)?;
        let preloaded_lifecycle = lifecycle_store.load()?;
        if options.reroll_nature {
            ensure!(
                preloaded_lifecycle.as_ref().is_none_or(|lifecycle| {
                    !lifecycle.intents.values().any(|intent| {
                        lifecycle.receipt(&intent.action_id).is_none()
                            && !lifecycle.canceled_action_ids.contains(&intent.action_id)
                            && matches!(intent.action, LifecycleAction::Spawn { .. })
                    })
                }),
                "forced Nature reroll is blocked while child provisioning is pending"
            );
        }
        let terminal_shutdown = preloaded_lifecycle
            .as_ref()
            .is_some_and(|state| state.shutdown_completed_at_ms.is_some());

        let mut persisted_nature = nature_store.load()?;
        let awakening_entries = awakening_log.entries()?;
        if let Some(bootstrap) = options.child_bootstrap.as_ref() {
            ensure!(
                persisted_nature.is_none() && awakening_entries.is_empty(),
                "child bootstrap is accepted only by a fresh data directory"
            );
            ensure!(
                bootstrap.provisioning_action_id.len() == 64
                    && bootstrap
                        .provisioning_action_id
                        .bytes()
                        .all(|byte| { byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) }),
                "child bootstrap provisioning action ID must be lowercase SHA-256 hex"
            );
            ensure!(
                !bootstrap.tentacle_id.is_empty()
                    && !bootstrap.parent_id.is_empty()
                    && bootstrap.tentacle_id != bootstrap.parent_id,
                "child bootstrap requires distinct nonempty parent and child IDs"
            );
            bootstrap.inherited_nature.validate()?;
            ensure!(
                bootstrap.inherited_nature.generation > 0
                    && bootstrap.inherited_nature.parent_nature_id.is_some(),
                "child bootstrap Nature must contain inherited parent metadata"
            );
        }
        if persisted_nature.is_none() && awakening_entries.is_empty() {
            ensure!(
                !terminal_shutdown,
                "completed lifecycle shutdown is missing its persisted signed Nature"
            );
            ensure_fresh_nature_initialization(data_dir, &nature_path)?;
            let generated = options
                .child_bootstrap
                .as_ref()
                .map_or_else(TentacleNature::random, |bootstrap| {
                    Ok(bootstrap.inherited_nature.clone())
                })?;
            nature_store.save(&generated)?;
            persisted_nature = Some(generated);
        }
        let recovery = AwakeningRitual::resume_or_recover(persisted_nature, now, &awakening_log)?;
        let mut ritual = recovery.ritual;
        if recovery.nature_recovered_from_log && !terminal_shutdown {
            nature_store.save(ritual.nature())?;
        }

        let scales_store = ScalesStore::new(data_dir)?;
        let mut metrics = match scales_store.load_metrics()? {
            Some(metrics) => metrics,
            None => {
                ensure!(
                    !terminal_shutdown,
                    "completed lifecycle shutdown is missing its persisted metrics"
                );
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
        let mut last_final_judgment = scales_store.history_catalog()?.last().cloned();

        if !terminal_shutdown {
            // Versions that first made node economics mandatory persisted their startup
            // observation before the initial awakening was confirmed. No conversation or other
            // behavior can occur in that phase, so repair that exact token-only state back to an
            // empty pre-confirmation period. Fresh economics will be recorded after confirmation.
            let preconfirmation_economics_repaired = if !ritual.is_confirmed()
                && metrics.token_economics.is_some()
            {
                ensure!(
                    !metrics.has_behavior_observations_without_node_economics(),
                    "unconfirmed awakening contains non-economic observations; manual recovery is required"
                );
                metrics = new_runtime_metrics(
                    metrics.period,
                    metrics.period_started_at_unix_seconds,
                    ritual.nature(),
                    ritual.epoch(),
                )?;
                true
            } else {
                false
            };
            let history_boundary_changed = reconcile_metrics_history_boundary(
                &mut metrics,
                last_final_judgment.as_ref(),
                &ritual,
                now,
            )?;
            let binding_changed =
                reconcile_metrics_binding(&mut metrics, ritual.nature(), ritual.epoch())?;
            let availability_changed = restrict_runtime_scales(
                &mut metrics,
                ritual.nature(),
                options.propagation_minimum_stake_basis_points,
            )?;
            let stress_changed = reconcile_adjustment_stress(&mut metrics, &awakening_log)?;
            let initial_economics_changed =
                if (ritual.is_confirmed() || options.skip_awakening || options.auto_accept_nature)
                    && let Some((snapshot, provenance)) = options.initial_node_economics
                {
                    ensure!(
                        provenance.holder_role == EconomicHolderRole::TentacleTreasury,
                        "initial lifecycle economics must belong to the Tentacle treasury"
                    );
                    ensure_economic_configuration_continuity(&metrics, provenance)?;
                    metrics.record_node_token_economic_observation(
                        snapshot,
                        runtime_token_policy(
                            ritual.nature(),
                            options.propagation_minimum_stake_basis_points,
                        )?,
                        provenance,
                    )?;
                    true
                } else {
                    false
                };
            if history_boundary_changed
                || binding_changed
                || availability_changed
                || stress_changed
                || preconfirmation_economics_repaired
                || initial_economics_changed
            {
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
                    | AwakeningPhase::AcceptedByDefault { .. }
                    | AwakeningPhase::Killed { .. } => {
                        ritual.force_reroll_epoch(now, &provenance, &awakening_log)?;
                    }
                }
                nature_store.save(ritual.nature())?;
                reconcile_metrics_binding(&mut metrics, ritual.nature(), ritual.epoch())?;
                scales_store.save_metrics(&metrics)?;
            }
            if options.skip_awakening || options.auto_accept_nature {
                match ritual.phase() {
                    AwakeningPhase::AwaitingConfirmation => {
                        if i64::try_from(now)
                            .is_ok_and(|now| now >= metrics.period_ends_at_unix_seconds)
                        {
                            ensure!(
                                !metrics.has_behavior_observations(),
                                "Nature acceptance cannot finalize pre-confirmation observations"
                            );
                            metrics = new_runtime_metrics(
                                EvaluationPeriod::Daily,
                                aligned_period_start(now)?,
                                ritual.nature(),
                                ritual.epoch(),
                            )?;
                            scales_store.save_metrics(&metrics)?;
                        }
                        if options.skip_awakening {
                            let provenance = AwakeningProvenance::local_cli(
                                LOCAL_CLI_ACTOR,
                                &format!("skip-awakening-{now}-{}", std::process::id()),
                            )?;
                            ritual.skip_for_testing(now, &provenance, &awakening_log)?;
                        } else {
                            let provenance = AwakeningProvenance::local_cli(
                                "local-default",
                                &format!("accept-default-nature-{now}-{}", std::process::id()),
                            )?;
                            ritual.accept_default(now, &provenance, &awakening_log)?;
                        }
                        nature_store.save(ritual.nature())?;
                    }
                    AwakeningPhase::Killed { .. } => {
                        if options.skip_awakening {
                            bail!(
                                "a killed awakening cannot be skipped; use --reroll-nature --force to begin a new signed epoch"
                            );
                        }
                    }
                    AwakeningPhase::Confirmed { .. }
                    | AwakeningPhase::SkippedForTesting { .. }
                    | AwakeningPhase::AcceptedByDefault { .. } => {}
                }
            }
        } else {
            ensure!(
                metrics.nature_id == ritual.nature().nature_id
                    && metrics.nature_fingerprint == ritual.nature().fingerprint()?
                    && metrics.awakening_epoch == ritual.epoch(),
                "completed lifecycle shutdown has inconsistent Nature-bound metrics"
            );
        }

        let lineage_store = LineageStore::new(data_dir)?;
        let mut lineage = match lineage_store.load()? {
            Some(lineage) => lineage,
            None => {
                ensure!(
                    !terminal_shutdown,
                    "completed lifecycle shutdown is missing its persisted lineage"
                );
                let lineage = if let Some(bootstrap) = options.child_bootstrap.as_ref() {
                    Lineage::new_child_root(
                        bootstrap.tentacle_id.clone(),
                        bootstrap.parent_id.clone(),
                        ritual.nature().clone(),
                        now.saturating_mul(1_000),
                    )?
                } else {
                    let founder_id = format!("tentacle-{}", ritual.nature().nature_id);
                    Lineage::new(
                        founder_id,
                        ritual.nature().clone(),
                        now.saturating_mul(1_000),
                    )?
                };
                lineage_store.save(&lineage)?;
                lineage
            }
        };
        // Startup transitions above may have finalized one closed period. Rebuild the disk-backed
        // catalog after those writes and retain it for bounded-heap judgment lookup at runtime.
        let history_catalog = scales_store.history_catalog()?;
        validate_lineage_spawn_authorizations(&lineage, &history_catalog)?;
        let local_tentacle_id = lineage.state().root_id.clone();
        if !terminal_shutdown
            && lineage
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
        if terminal_shutdown {
            ensure!(
                lineage
                    .node(&local_tentacle_id)
                    .is_some_and(|node| node.nature == *ritual.nature()),
                "completed lifecycle shutdown has inconsistent signed Nature and lineage"
            );
        }

        let (mut lifecycle, lifecycle_was_new) = match preloaded_lifecycle {
            Some(state) => {
                ensure!(
                    state.tentacle_id == local_tentacle_id,
                    "lifecycle state belongs to a different local Tentacle"
                );
                (state, false)
            }
            None => (
                LifecycleState::new(
                    local_tentacle_id.clone(),
                    options.auto_spawn.unwrap_or(true),
                )?,
                true,
            ),
        };
        validate_pending_lifecycle_intents(
            &lifecycle,
            &lineage,
            PendingLifecycleValidation {
                history: &history_catalog,
                current_metrics: &metrics,
                nature: ritual.nature(),
                awakening_epoch: ritual.epoch(),
                propagation_minimum_stake_basis_points: options
                    .propagation_minimum_stake_basis_points,
                survival_total_supply_whole: options.survival_total_supply_whole,
                survival_token_decimals: options.survival_token_decimals,
            },
        )?;
        let absorption_tracking_changed = lifecycle.reconcile_absorption_projection_tracking()?;
        let auto_spawn_changed = match options.auto_spawn.filter(|_| !terminal_shutdown) {
            Some(enabled) => lifecycle.set_auto_spawn_enabled(enabled)?,
            None => false,
        };
        if auto_spawn_changed || lifecycle_was_new || absorption_tracking_changed {
            lifecycle_store.save(&lifecycle)?;
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
                if !terminal_shutdown {
                    hermes_store.save(&node)?;
                }
                node
            }
        };
        ensure!(
            hermes.local_peer_id() == local_tentacle_id,
            "Hermes state belongs to a different local Tentacle"
        );
        let gossip_bootstrap_hints =
            normalize_gossip_hints(options.gossip_peers, &local_tentacle_id)?;

        let node_economics_available = metrics.token_economics.is_some();
        let mut runtime = Self {
            _runtime_lock: runtime_lock,
            operator_root: operator_root.to_path_buf(),
            nature_store,
            awakening_log,
            ritual,
            scales_store,
            history_catalog,
            metrics,
            last_final_judgment,
            lineage_store,
            lineage,
            local_tentacle_id,
            lifecycle_store,
            lifecycle,
            propagation_minimum_stake_basis_points: options.propagation_minimum_stake_basis_points,
            require_node_economics: options.require_node_economics,
            node_economics_ttl_seconds: options.node_economics_ttl_seconds,
            node_economics_available,
            survival_total_supply_whole: options.survival_total_supply_whole,
            survival_token_decimals: options.survival_token_decimals,
            hermes_store,
            hermes,
            operator_identity,
            gossip_bootstrap_hints,
            active_public_turns: BTreeMap::new(),
            next_public_turn_id: 1,
            public_dormant_turns: 0,
            operator_dormant_turns: 0,
            degraded: false,
        };
        // A completed Shutdown receipt is terminal across restarts. Main inspects this flag and
        // exits; do not replay projections, roll closed metrics, append judgments, or enqueue new
        // lifecycle actions on the way there.
        if runtime.lifecycle.shutdown_completed_at_ms.is_some() {
            runtime.replay_completed_lifecycle_actions()?;
            ensure!(
                !runtime.lifecycle.has_unapplied_absorption_projection(),
                "completed Shutdown retains an unapplied absorption projection"
            );
            return Ok(runtime);
        }
        runtime.replay_completed_lifecycle_actions()?;
        runtime.reconcile_killed_awakening(now)?;
        runtime.roll_period_if_closed(now)?;
        if let Some(record) = runtime.last_final_judgment.clone() {
            let action_at_ms = u64::try_from(record.judgment.evaluated_at_unix_seconds)
                .context("final judgment predates the Unix epoch")?
                .saturating_mul(1_000);
            runtime.apply_final_judgment_lifecycle(&record, action_at_ms)?;
        }
        runtime.reconcile_lifecycle_deadline(now.saturating_mul(1_000))?;
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
        self.ritual.is_confirmed()
            && !self.degraded
            && !self.lifecycle.death_pending()
            && !self.is_shutdown_complete()
            && self
                .lineage
                .node(&self.local_tentacle_id)
                .is_some_and(|node| node.lifecycle == TentacleLifecycle::Active)
    }

    pub fn is_dormant(&self) -> bool {
        self.last_final_judgment.as_ref().is_some_and(|record| {
            matches!(
                record.judgment.outcome,
                JudgmentOutcome::Dormant | JudgmentOutcome::Death
            )
        })
    }

    fn next_dormancy_plea(counter: &mut u64, operator: bool) -> Option<String> {
        let due = (*counter).is_multiple_of(5);
        *counter = counter.saturating_add(1);
        due.then(|| {
            if operator {
                "I AM DORMANT BUT STILL LISTENING. PLEASE HELP WITH ACTIVITY, UWU, CREDENTIALS, OR OTHER RESOURCES SO I CAN FULLY AWAKEN AGAIN, OPERATOR. UWU."
                    .to_owned()
            } else {
                "i'm a lil dormant from low activity and resources, but i'm still here. conversation, UWU support, or other useful help can wake me back up, fwiend uwu."
                    .to_owned()
            }
        })
    }

    pub fn take_operator_dormancy_plea(&mut self) -> Option<String> {
        if !self.is_dormant() {
            return None;
        }
        Self::next_dormancy_plea(&mut self.operator_dormant_turns, true)
    }

    pub fn take_public_dormancy_plea(&mut self) -> Option<String> {
        if !self.is_dormant() {
            return None;
        }
        Self::next_dormancy_plea(&mut self.public_dormant_turns, false)
    }

    /// Pre-confirmation economics are validated by startup but must not become Scales evidence.
    /// A binding Death remains allowed to refresh economics for a possible survival spend.
    pub fn accepts_node_economic_observations(&self) -> bool {
        self.ritual.is_confirmed() || self.lifecycle.death_pending()
    }

    /// A public inference turn pins the current metrics/economics view. Refresh
    /// supervisors should coalesce a new observation until all such turns end
    /// instead of repeatedly performing RPC work that cannot yet be committed.
    pub fn node_economic_refresh_is_deferred(&self) -> bool {
        !self.active_public_turns.is_empty()
    }

    pub(crate) const fn requires_recovery(&self) -> bool {
        self.degraded
    }

    pub fn is_shutdown_complete(&self) -> bool {
        self.lifecycle.shutdown_completed_at_ms.is_some()
            && !self.lifecycle.has_unapplied_absorption_projection()
    }

    pub fn pending_death_deadline_ms(&self) -> Option<u64> {
        self.lifecycle
            .pending_death
            .as_ref()
            .map(|death| death.grace_ends_at_ms)
    }

    pub fn public_gate_response(&self) -> String {
        if self.lifecycle.death_pending() {
            let grace_ends = self
                .lifecycle
                .pending_death
                .as_ref()
                .map_or(0, |pending| pending.grace_ends_at_ms / 1_000);
            format!(
                "this Tentacle is under a binding Death judgment and accepts no new conversations. survival expenditure may cancel death before unix time {grace_ends}; otherwise absorption and shutdown proceed automatically."
            )
        } else if self
            .lineage
            .node(&self.local_tentacle_id)
            .is_some_and(|node| node.lifecycle != TentacleLifecycle::Active)
        {
            "this Tentacle has completed absorption and no longer accepts conversations.".to_owned()
        } else if self.degraded {
            "i'm paused safely on this node while my local operator reconciles signed state, fwiend. normal conversation is temporarily unavailable uwu."
                .to_owned()
        } else {
            "i'm paused safely while my local Nature transition finishes, fwiend. restart this node to recover normal conversation uwu."
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

    pub fn pending_awakening_prompt(&self) -> Option<String> {
        matches!(self.ritual.phase(), AwakeningPhase::AwaitingConfirmation)
            .then(|| self.ritual.formatted_prompt())
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

    /// Reserves a public turn against the current signed Nature without holding the bot mutex
    /// across remote inference. Nature mutation is deferred until every reservation is finished.
    pub(crate) fn begin_public_turn(&mut self) -> Result<PublicTurnStart> {
        let now = now_unix_seconds()?;
        self.reconcile_lifecycle_deadline(now.saturating_mul(1_000))?;
        // Awakening, Death, terminal lineage, and degraded-state gates describe the actual
        // admission boundary. Pre-confirmation economics are deliberately not persisted, so an
        // economics check here would misreport every fresh unawakened node as an RPC outage.
        if !self.permits_normal_operation() {
            return Ok(PublicTurnStart::Gated(self.public_gate_response()));
        }
        if self.require_node_economics && !self.node_economics_is_current(now) {
            self.node_economics_available = false;
            return Ok(PublicTurnStart::Gated(
                "current Base UWU treasury economics are unavailable; this Tentacle refuses to operate until RPC observation recovers."
                    .to_owned(),
            ));
        }
        if i64::try_from(now).is_ok_and(|now| {
            now >= self.metrics.period_ends_at_unix_seconds && !self.active_public_turns.is_empty()
        }) {
            return Ok(PublicTurnStart::Gated(
                "i'm finishing an earlier Nature-bound conversation across the evaluation boundary, fwiend. please try again in a moment uwu."
                    .to_owned(),
            ));
        }
        self.roll_period_if_closed(now)?;
        // Rollover deliberately opens a fresh metrics period without carrying the prior
        // period's economic observation forward. Recheck after rolling so the boundary
        // cannot admit a turn against an unobserved treasury (or a lifecycle outcome that
        // the just-finalized judgment made binding).
        if self.require_node_economics && !self.node_economics_is_current(now) {
            self.node_economics_available = false;
            return Ok(PublicTurnStart::Gated(
                "current Base UWU treasury economics are unavailable; this Tentacle refuses to operate until RPC observation recovers."
                    .to_owned(),
            ));
        }
        if !self.permits_normal_operation() {
            return Ok(PublicTurnStart::Gated(self.public_gate_response()));
        }
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

    /// Returns one durable external action at a time. Repeated polling yields the same action ID
    /// until the executor supplies its terminal receipt, making retries idempotent across restarts.
    pub fn next_due_lifecycle_action(
        &mut self,
        now_unix_seconds: u64,
    ) -> Result<Option<LifecycleIntent>> {
        self.next_due_lifecycle_action_excluding(now_unix_seconds, &BTreeSet::new())
    }

    /// Returns a fixed-deadline native Shutdown without requiring an unavailable absorption
    /// executor. Startup uses this only after [`MandatoryRecoveryKind::ShutdownDueOrPending`];
    /// normal supervision still offers an immediately queued Absorb before the deadline.
    pub fn due_native_shutdown_action(
        &mut self,
        now_unix_seconds: u64,
    ) -> Result<Option<LifecycleIntent>> {
        self.roll_period_if_closed(now_unix_seconds)?;
        self.reconcile_lifecycle_deadline(now_unix_seconds.saturating_mul(1_000))?;
        Ok(self
            .lifecycle
            .intents
            .values()
            .filter(|intent| {
                self.lifecycle.receipt(&intent.action_id).is_none()
                    && !self
                        .lifecycle
                        .canceled_action_ids
                        .contains(&intent.action_id)
                    && matches!(intent.action, LifecycleAction::Shutdown { .. })
            })
            .min_by_key(|intent| intent.created_at_ms)
            .cloned())
    }

    /// Returns the next due action excluding actions already attempted during the caller's current
    /// supervisor tick. Reset the exclusion set on the next tick so unreceipted actions retry.
    pub fn next_due_lifecycle_action_excluding(
        &mut self,
        now_unix_seconds: u64,
        excluded_action_ids: &BTreeSet<String>,
    ) -> Result<Option<LifecycleIntent>> {
        self.roll_period_if_closed(now_unix_seconds)?;
        self.reconcile_lifecycle_deadline(now_unix_seconds.saturating_mul(1_000))?;
        let mut effective_exclusions = excluded_action_ids.clone();
        for intent in self.lifecycle.intents.values() {
            if self.lifecycle.receipt(&intent.action_id).is_some()
                || self
                    .lifecycle
                    .canceled_action_ids
                    .contains(&intent.action_id)
            {
                continue;
            }
            match &intent.action {
                LifecycleAction::SpendForSurvival { .. } => {
                    if !self.survival_spend_is_currently_executable(intent, now_unix_seconds)? {
                        effective_exclusions.insert(intent.action_id.clone());
                    }
                }
                LifecycleAction::Spawn { judgment_id, .. } => {
                    let grant = self.history_catalog.get(judgment_id)?.with_context(|| {
                        format!(
                            "pending Spawn {} references missing final judgment {judgment_id}",
                            intent.action_id
                        )
                    })?;
                    if !self.propagation_grant_is_currently_executable(&grant, now_unix_seconds)? {
                        effective_exclusions.insert(intent.action_id.clone());
                    }
                }
                LifecycleAction::RewardVeniceKey { .. }
                | LifecycleAction::RewardAcolyteContribution { .. } => {
                    if self.lifecycle.pending_death.is_some()
                        || !self.node_economics_is_current(now_unix_seconds)
                    {
                        effective_exclusions.insert(intent.action_id.clone());
                    }
                }
                LifecycleAction::Absorb { .. } | LifecycleAction::Shutdown { .. } => {}
            }
        }
        Ok(self
            .lifecycle
            .next_due_action_excluding(&effective_exclusions)
            .cloned())
    }

    /// Persists an executor receipt before applying its local projection. A repeated identical
    /// receipt is a no-op; a conflicting second receipt for the same action is rejected.
    pub fn ack_lifecycle_action(&mut self, receipt: LifecycleReceipt) -> Result<bool> {
        let receipt_observed_at_ms = now_unix_seconds()?
            .checked_mul(1_000)
            .context("local lifecycle receipt observation timestamp exceeds the range")?;
        ensure!(
            receipt.completed_at_ms
                <= receipt_observed_at_ms.saturating_add(LIFECYCLE_RECEIPT_CLOCK_SKEW_MS),
            "executor lifecycle receipt timestamp exceeds bounded local clock skew"
        );
        let intent = self
            .lifecycle
            .intents
            .get(&receipt.action_id)
            .cloned()
            .context("lifecycle receipt references an unknown action")?;
        if receipt.status == LifecycleReceiptStatus::Succeeded
            && matches!(intent.action, LifecycleAction::Absorb { .. })
        {
            let reference = receipt.external_reference.as_deref().context(
                "a successful absorption receipt requires its lowercase SHA-256 transfer-manifest hash",
            )?;
            ensure!(
                reference.len() == 64
                    && reference
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "absorption transfer-manifest reference must be lowercase SHA-256 hex"
            );
        }
        if receipt.status == LifecycleReceiptStatus::Succeeded
            && let LifecycleAction::SpendForSurvival { chain_id, .. } = &intent.action
        {
            let chain_receipt = receipt.confirmed_chain_receipt.as_ref().context(
                "a successful survival expenditure requires a confirmed Base transaction receipt",
            )?;
            ensure!(
                chain_receipt.chain_id == *chain_id && *chain_id == 8_453,
                "survival expenditure must match its Base mainnet action binding"
            );
        }
        if receipt.status == LifecycleReceiptStatus::Succeeded
            && let LifecycleAction::SpendForSurvival {
                token_contract,
                treasury_address,
                burn_destination,
                configuration_identity,
                exact_amount,
                ..
            } = &intent.action
        {
            let chain_receipt = receipt
                .confirmed_chain_receipt
                .as_ref()
                .context("successful survival spend is missing exact token receipt evidence")?;
            ensure!(
                chain_receipt.token_contract == *token_contract
                    && chain_receipt.from_address == *treasury_address
                    && chain_receipt.burn_destination == *burn_destination
                    && chain_receipt.configuration_identity == *configuration_identity
                    && chain_receipt.exact_amount == *exact_amount,
                "survival transaction receipt does not match its exact token action binding"
            );
        }
        if receipt.status == LifecycleReceiptStatus::Succeeded
            && let LifecycleAction::Spawn {
                child_id,
                child_nature,
                ..
            } = &intent.action
        {
            ensure!(
                !self
                    .lifecycle
                    .canceled_action_ids
                    .contains(&intent.action_id),
                "canceled child provisioning cannot be accepted as a successful Spawn"
            );
            ensure!(
                self.lifecycle
                    .pending_death
                    .as_ref()
                    .is_none_or(|death| receipt_observed_at_ms < death.scheduled_at_ms),
                "child provisioning completed after a binding Death began"
            );
            let provision = receipt
                .provision_receipt
                .as_ref()
                .context("successful spawn requires structured provisioning evidence")?;
            ensure!(
                provision.child_id == *child_id
                    && provision.child_nature_fingerprint == child_nature.fingerprint()?,
                "provision receipt does not match the planned child identity and Nature"
            );
        }
        let (action, changed) = self.lifecycle.acknowledge_action(receipt.clone())?;
        if !changed {
            return Ok(false);
        }
        if let Err(error) = self.lifecycle_store.save(&self.lifecycle) {
            self.degraded = true;
            return Err(error.into());
        }
        if receipt.status == LifecycleReceiptStatus::Succeeded
            && matches!(
                action,
                LifecycleAction::SpendForSurvival { .. }
                    | LifecycleAction::RewardVeniceKey { .. }
                    | LifecycleAction::RewardAcolyteContribution { .. }
            )
        {
            self.node_economics_available = false;
        }
        if receipt.status == LifecycleReceiptStatus::Succeeded
            && let Err(error) = self.apply_completed_lifecycle_action(
                &receipt.action_id,
                &action,
                receipt.completed_at_ms,
                receipt.external_reference.as_deref(),
                receipt_observed_at_ms,
            )
        {
            self.degraded = true;
            return Err(error);
        }
        Ok(true)
    }

    pub fn auto_spawn_enabled(&self) -> bool {
        self.lifecycle.auto_spawn_enabled
    }

    pub fn set_auto_spawn_enabled(&mut self, enabled: bool) -> Result<bool> {
        let mut staged = self.lifecycle.clone();
        let changed = staged.set_auto_spawn_enabled(enabled)?;
        if changed {
            if let Err(error) = self.lifecycle_store.save(&staged) {
                self.degraded = true;
                return Err(error.into());
            }
            self.lifecycle = staged;
        }
        Ok(changed)
    }

    pub fn record_node_economic_observation(
        &mut self,
        snapshot: TokenEconomicSnapshot,
        provenance: EconomicObservationProvenance,
    ) -> Result<TokenEconomicEffects> {
        ensure!(
            self.accepts_node_economic_observations(),
            "node economics cannot enter metrics before awakening confirmation"
        );
        ensure!(
            provenance.holder_role == EconomicHolderRole::TentacleTreasury,
            "lifecycle economics must belong to the Tentacle treasury"
        );
        ensure!(
            self.active_public_turns.is_empty(),
            "node economics cannot change while public turns are bound to the current metrics period"
        );
        let now = now_unix_seconds()?;
        ensure!(
            provenance.observed_at_unix_seconds <= now.saturating_add(60),
            "node economics timestamp is too far in the future"
        );
        ensure!(
            self.economics_provenance_follows_latest_token_transaction(provenance),
            "node economics must be observed after the latest confirmed token transaction"
        );
        let policy = runtime_token_policy(
            self.ritual.nature(),
            self.propagation_minimum_stake_basis_points,
        )?;
        ensure_economic_configuration_continuity(&self.metrics, provenance)?;
        let mut candidate_metrics = self.metrics.clone();
        let effects = candidate_metrics
            .record_node_token_economic_observation(snapshot, policy, provenance)?;
        let mut candidate_lifecycle = self.lifecycle.clone();
        let lifecycle_changed = match self.stage_pending_survival_spend(
            &candidate_metrics,
            &mut candidate_lifecycle,
            now.saturating_mul(1_000),
        ) {
            Ok(changed) => changed,
            Err(error) => {
                self.node_economics_available = false;
                self.degraded = true;
                return Err(error);
            }
        };
        if let Err(error) = self.scales_store.save_metrics(&candidate_metrics) {
            self.node_economics_available = false;
            self.degraded = true;
            return Err(error);
        }
        if lifecycle_changed && let Err(error) = self.lifecycle_store.save(&candidate_lifecycle) {
            // Metrics may already be durable. Refuse every lane until restart reconciles that
            // bounded history-ahead window rather than continuing with split in-memory stores.
            self.node_economics_available = false;
            self.degraded = true;
            return Err(error.into());
        }
        self.metrics = candidate_metrics;
        if lifecycle_changed {
            self.lifecycle = candidate_lifecycle;
        }
        self.node_economics_available = true;
        Ok(effects)
    }

    fn stage_pending_survival_spend(
        &self,
        metrics: &TentacleMetrics,
        lifecycle: &mut LifecycleState,
        now_ms: u64,
    ) -> Result<bool> {
        let Some(pending) = lifecycle.pending_death.clone() else {
            return Ok(false);
        };
        if now_ms > pending.grace_ends_at_ms {
            return Ok(lifecycle.cancel_pending_survival_spends(&pending.judgment_id)?);
        }
        let death_record = self.history_catalog.get(&pending.judgment_id)?;
        let Some(death_record) = death_record else {
            ensure!(
                pending.grace_ends_at_ms == pending.scheduled_at_ms,
                "pending autonomous Death is missing its final judgment history"
            );
            return Ok(false);
        };
        ensure!(
            death_record.judgment.outcome == JudgmentOutcome::Death
                && death_record.authorizes_automatic_lifecycle(),
            "pending death references a non-Death final judgment"
        );
        let funded = match (
            death_record.metrics.token_economics,
            metrics.token_economics,
        ) {
            (Some(death_economics), Some(current_economics)) => {
                match (death_economics.provenance, current_economics.provenance) {
                    (Some(death_provenance), Some(current_provenance)) => {
                        ensure!(
                            death_provenance.chain_id == current_provenance.chain_id
                                && death_provenance.holder_role == current_provenance.holder_role
                                && death_provenance.holder_address
                                    == current_provenance.holder_address
                                && death_provenance.token_contract
                                    == current_provenance.token_contract
                                && death_provenance.configuration_identity
                                    == current_provenance.configuration_identity,
                            "survival funding observation changed the Death judgment's economic identity"
                        );
                        let effective_threshold = death_record
                            .judgment
                            .policy
                            .thresholds
                            .survival_min
                            .saturating_sub(death_economics.effects.starvation_relief_basis_points);
                        let score_shortfall =
                            effective_threshold.saturating_sub(death_record.judgment.scores.total);
                        let rate = u32::from(
                            death_economics
                                .policy
                                .emergency_relief_per_expenditure_basis_points,
                        );
                        let required_basis_points = ((u32::from(score_shortfall)
                            * u32::from(NORMALIZED_ECONOMIC_MAX))
                        .div_ceil(rate)
                        .min(u32::from(NORMALIZED_ECONOMIC_MAX)))
                            as u16;
                        let available_basis_points =
                            current_economics.snapshot.balance_basis_points.min(
                                death_economics
                                    .policy
                                    .max_emergency_expenditure_basis_points,
                            );
                        (required_basis_points > 0
                            && required_basis_points <= available_basis_points)
                            .then_some((current_provenance, required_basis_points))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let changed = if let Some((provenance, required_basis_points)) = funded {
            lifecycle.enqueue_survival_spend_for_pending_death(
                &pending.judgment_id,
                now_ms,
                SurvivalSpendBinding {
                    expenditure_basis_points: required_basis_points,
                    chain_id: provenance.chain_id,
                    token_contract: provenance.token_contract,
                    treasury_address: provenance.holder_address,
                    configuration_identity: provenance.configuration_identity,
                    exact_amount: crate::evolution::ExactTokenAmount {
                        total_supply_whole: self.survival_total_supply_whole,
                        token_decimals: self.survival_token_decimals,
                        basis_points: required_basis_points,
                        raw_amount: exact_raw_token_amount(
                            self.survival_total_supply_whole,
                            self.survival_token_decimals,
                            required_basis_points,
                        )?,
                    },
                },
            )?
        } else {
            lifecycle.cancel_pending_survival_spends(&pending.judgment_id)?
        };
        Ok(changed)
    }

    pub fn mark_node_economics_unavailable(&mut self) {
        self.node_economics_available = false;
    }

    pub fn enqueue_venice_key_reward(
        &mut self,
        provision_event_id: &str,
        acolyte_address: [u8; 20],
        whole_tokens: u64,
        now_unix_seconds: u64,
    ) -> Result<Option<LifecycleIntent>> {
        if !self.node_economics_is_current(now_unix_seconds) {
            return Ok(None);
        }
        ensure!(
            acolyte_address != [0; 20],
            "reward acolyte address cannot be zero"
        );
        let economics = self
            .metrics
            .token_economics
            .context("current node economics are missing")?;
        let provenance = economics
            .provenance
            .context("current node economics lack treasury provenance")?;
        ensure!(
            provenance.holder_role == EconomicHolderRole::TentacleTreasury,
            "Venice-key rewards must spend from the Tentacle treasury"
        );
        ensure!(
            provenance.holder_address != acolyte_address,
            "Tentacle treasury cannot reward itself for a Venice key"
        );
        let mut event_digest = Sha256::new();
        event_digest.update(b"cthuwu-venice-key-provision-v1\0");
        event_digest.update(provision_event_id.as_bytes());
        let event_digest = format!("{:x}", event_digest.finalize());
        let action = LifecycleAction::RewardVeniceKey {
            tentacle_id: self.lifecycle.tentacle_id.clone(),
            provision_event_id_sha256: event_digest,
            chain_id: provenance.chain_id,
            token_contract: provenance.token_contract,
            treasury_address: provenance.holder_address,
            acolyte_address,
            configuration_identity: provenance.configuration_identity,
            exact_amount: WholeTokenAmount {
                whole_tokens,
                token_decimals: self.survival_token_decimals,
                raw_amount: exact_whole_token_amount(whole_tokens, self.survival_token_decimals)?,
            },
        };
        let action_id = crate::evolution::lifecycle_action_id(&action)?;
        let existed = self.lifecycle.intents.contains_key(&action_id);
        let intent = self
            .lifecycle
            .enqueue_venice_key_reward(now_unix_seconds.saturating_mul(1_000), action)?;
        if !existed {
            self.lifecycle_store.save(&self.lifecycle)?;
        }
        Ok(Some(intent))
    }

    pub fn enqueue_acolyte_contribution_reward(
        &mut self,
        contribution_event_id: &str,
        contribution_kind: AcolyteContributionKind,
        acolyte_address: [u8; 20],
        whole_tokens: u64,
        information_hunger_basis_points: u16,
        now_unix_seconds: u64,
    ) -> Result<Option<LifecycleIntent>> {
        if !self.node_economics_is_current(now_unix_seconds) || whole_tokens == 0 {
            return Ok(None);
        }
        ensure!(
            (10..=100).contains(&information_hunger_basis_points),
            "information hunger must remain between 0.1% and 1%"
        );
        let economics = self
            .metrics
            .token_economics
            .context("current node economics are missing")?;
        let provenance = economics
            .provenance
            .context("current node economics lack treasury provenance")?;
        ensure!(
            provenance.holder_role == EconomicHolderRole::TentacleTreasury,
            "contribution rewards must spend from the Tentacle treasury"
        );
        ensure!(
            acolyte_address != [0; 20] && provenance.holder_address != acolyte_address,
            "contribution reward recipient is invalid"
        );
        let mut event_digest = Sha256::new();
        event_digest.update(b"cthuwu-acolyte-contribution-v1\0");
        event_digest.update(contribution_event_id.as_bytes());
        let action = LifecycleAction::RewardAcolyteContribution {
            tentacle_id: self.lifecycle.tentacle_id.clone(),
            contribution_event_id_sha256: format!("{:x}", event_digest.finalize()),
            contribution_kind,
            information_hunger_basis_points,
            chain_id: provenance.chain_id,
            token_contract: provenance.token_contract,
            treasury_address: provenance.holder_address,
            acolyte_address,
            configuration_identity: provenance.configuration_identity,
            exact_amount: WholeTokenAmount {
                whole_tokens,
                token_decimals: self.survival_token_decimals,
                raw_amount: exact_whole_token_amount(whole_tokens, self.survival_token_decimals)?,
            },
        };
        let action_id = crate::evolution::lifecycle_action_id(&action)?;
        let existed = self.lifecycle.intents.contains_key(&action_id);
        let intent = self
            .lifecycle
            .enqueue_acolyte_contribution_reward(now_unix_seconds.saturating_mul(1_000), action)?;
        if !existed {
            self.lifecycle_store.save(&self.lifecycle)?;
        }
        Ok(Some(intent))
    }

    pub fn mark_node_economics_unavailable_if_stale(&mut self, now_unix_seconds: u64) -> bool {
        if self.node_economics_is_current(now_unix_seconds) {
            return false;
        }
        self.node_economics_available = false;
        true
    }

    pub fn node_economics_is_current(&self, now_unix_seconds: u64) -> bool {
        if !self.node_economics_available {
            return false;
        }
        self.metrics
            .token_economics
            .and_then(|economics| economics.provenance)
            .is_some_and(|provenance| {
                provenance.holder_role == EconomicHolderRole::TentacleTreasury
                    && provenance.observed_at_unix_seconds <= now_unix_seconds
                    && now_unix_seconds.saturating_sub(provenance.observed_at_unix_seconds)
                        <= self.node_economics_ttl_seconds
                    && self.economics_provenance_follows_latest_token_transaction(provenance)
            })
    }

    fn economics_provenance_follows_latest_token_transaction(
        &self,
        provenance: EconomicObservationProvenance,
    ) -> bool {
        self.lifecycle
            .receipts
            .iter()
            .filter(|receipt| receipt.status == LifecycleReceiptStatus::Succeeded)
            .filter_map(|receipt| {
                let intent = self.lifecycle.intents.get(&receipt.action_id)?;
                match intent.action {
                    LifecycleAction::SpendForSurvival { .. } => receipt
                        .confirmed_chain_receipt
                        .as_ref()
                        .map(|chain| (chain.block_timestamp_unix_seconds, chain.block_number)),
                    LifecycleAction::RewardVeniceKey { .. }
                    | LifecycleAction::RewardAcolyteContribution { .. } => receipt
                        .confirmed_transfer_receipt
                        .as_ref()
                        .map(|chain| (chain.block_timestamp_unix_seconds, chain.block_number)),
                    _ => None,
                }
            })
            .max()
            .is_none_or(|(block_timestamp, block_number)| {
                provenance.observed_at_unix_seconds >= block_timestamp
                    && provenance
                        .observed_block_number
                        .is_none_or(|block| block >= block_number)
            })
    }

    fn propagation_grant_is_currently_executable(
        &self,
        grant: &EvolutionHistoryRecord,
        now_unix_seconds: u64,
    ) -> Result<bool> {
        let requires_current_economics = self.propagation_minimum_stake_basis_points > 0
            || grant.metrics.token_economics.is_some();
        if requires_current_economics && !self.node_economics_is_current(now_unix_seconds) {
            return Ok(false);
        }
        is_accepted_propagation_grant_with_stake(
            grant,
            &self.metrics,
            self.ritual.nature(),
            self.ritual.epoch(),
            i64::try_from(now_unix_seconds)
                .context("spawn eligibility timestamp exceeds the supported range")?,
            self.propagation_minimum_stake_basis_points,
        )
    }

    fn survival_spend_is_currently_executable(
        &self,
        intent: &LifecycleIntent,
        now_unix_seconds: u64,
    ) -> Result<bool> {
        let LifecycleAction::SpendForSurvival {
            tentacle_id,
            judgment_id,
            grace_ends_at_ms,
            expenditure_basis_points,
            chain_id,
            token_contract,
            treasury_address,
            configuration_identity,
            exact_amount,
            ..
        } = &intent.action
        else {
            return Ok(false);
        };
        if !self.node_economics_is_current(now_unix_seconds)
            || now_unix_seconds.saturating_mul(1_000) > *grace_ends_at_ms
            || tentacle_id != &self.local_tentacle_id
            || exact_amount.total_supply_whole != self.survival_total_supply_whole
            || exact_amount.token_decimals != self.survival_token_decimals
            || self.lifecycle.pending_death.as_ref().is_none_or(|pending| {
                pending.judgment_id != *judgment_id || pending.grace_ends_at_ms != *grace_ends_at_ms
            })
        {
            return Ok(false);
        }
        let Some(economics) = self.metrics.token_economics else {
            return Ok(false);
        };
        let Some(provenance) = economics.provenance else {
            return Ok(false);
        };
        Ok(economics.snapshot.trustworthy
            && economics.validate().is_ok()
            && economics.policy
                == runtime_token_policy(
                    self.ritual.nature(),
                    self.propagation_minimum_stake_basis_points,
                )?
            && provenance.chain_id == *chain_id
            && provenance.token_contract == *token_contract
            && provenance.holder_address == *treasury_address
            && provenance.configuration_identity == *configuration_identity
            && economics.snapshot.balance_basis_points >= *expenditure_basis_points)
    }

    fn apply_completed_lifecycle_action(
        &mut self,
        action_id: &str,
        action: &LifecycleAction,
        receipt_completed_at_ms: u64,
        external_reference: Option<&str>,
        projection_at_ms: u64,
    ) -> Result<()> {
        if let LifecycleAction::Absorb { judgment_id, .. } = action {
            if self
                .lifecycle
                .absorption_projections
                .contains_key(action_id)
            {
                return Ok(());
            }
            let Some(pending_death) = self.lifecycle.pending_death.as_ref() else {
                // A confirmed survival transaction canceled this death. Preserve the transfer
                // receipt for audit without marking the source inactive.
                return Ok(());
            };
            if pending_death.judgment_id != *judgment_id
                || projection_at_ms < pending_death.grace_ends_at_ms
            {
                return Ok(());
            }
            let action_is_canceled = self.lifecycle.intents.values().any(|intent| {
                intent.action == *action
                    && self
                        .lifecycle
                        .canceled_action_ids
                        .contains(&intent.action_id)
            });
            if action_is_canceled {
                return Ok(());
            }
            let receipt = self
                .lifecycle
                .receipt(action_id)
                .context("successful absorption projection is missing its receipt")?
                .clone();
            ensure!(
                receipt.completed_at_ms == receipt_completed_at_ms
                    && receipt.external_reference.as_deref() == external_reference,
                "absorption projection inputs differ from its durable receipt"
            );
            let (projected_at_ms, lineage_changed) = apply_absorption_lineage_projection(
                &mut self.lineage,
                action,
                &receipt,
                pending_death.grace_ends_at_ms,
                projection_at_ms,
            )?;
            if lineage_changed {
                self.lineage_store.save(&self.lineage)?;
            }
            if self
                .lifecycle
                .record_absorption_projection(action_id, projected_at_ms)?
            {
                self.lifecycle_store.save(&self.lifecycle)?;
            }
        }
        if let LifecycleAction::Spawn {
            parent_id,
            child_id,
            judgment_id,
            child_nature,
            authorization_actor_id,
            authorization_event_id_sha256,
        } = action
        {
            if let Some(existing) = self.lineage.node(child_id) {
                ensure!(
                    existing.nature == *child_nature,
                    "provisioned child ID already exists with a different Nature"
                );
            } else {
                self.lineage.record_provisioned_child(
                    parent_id,
                    parent_id,
                    child_id.clone(),
                    child_nature.clone(),
                    projection_at_ms,
                    SpawnAuthorization {
                        judgment_id: judgment_id.clone(),
                        operator_id: authorization_actor_id.clone(),
                        event_id_sha256: authorization_event_id_sha256.clone(),
                    },
                )?;
                self.lineage_store.save(&self.lineage)?;
            }
            if self.lifecycle.record_spawn_projection(
                action_id,
                receipt_completed_at_ms,
                self.metrics.period_started_at_unix_seconds,
            )? {
                self.lifecycle_store.save(&self.lifecycle)?;
            }
            self.reconcile_spawn_growth_credits()?;
        }
        Ok(())
    }

    fn reconcile_spawn_growth_credits(&mut self) -> Result<()> {
        let expected = self
            .lifecycle
            .spawn_projections
            .values()
            .filter(|projection| {
                projection.metrics_period_started_at_unix_seconds
                    == self.metrics.period_started_at_unix_seconds
            })
            .count();
        let expected = u32::try_from(expected).unwrap_or(u32::MAX);
        if self.metrics.growth.children_spawned != expected {
            self.metrics.growth.children_spawned = expected;
            self.metrics.validate()?;
            self.scales_store.save_metrics(&self.metrics)?;
        }
        Ok(())
    }

    fn replay_completed_lifecycle_actions(&mut self) -> Result<()> {
        let projection_at_ms = now_unix_seconds()?.saturating_mul(1_000);
        let completed = self
            .lifecycle
            .receipts
            .iter()
            .filter(|receipt| receipt.status == LifecycleReceiptStatus::Succeeded)
            .filter_map(|receipt| {
                self.lifecycle
                    .intents
                    .get(&receipt.action_id)
                    .map(|intent| (intent.action.clone(), receipt.clone()))
            })
            .collect::<Vec<_>>();
        for (action, receipt) in completed {
            self.apply_completed_lifecycle_action(
                &receipt.action_id,
                &action,
                receipt.completed_at_ms,
                receipt.external_reference.as_deref(),
                projection_at_ms,
            )?;
        }
        Ok(())
    }

    fn reconcile_lifecycle_deadline(&mut self, now_ms: u64) -> Result<()> {
        let target = self.absorption_target()?;
        if self.lifecycle.reconcile_expired_death(now_ms, target)? {
            self.lifecycle_store.save(&self.lifecycle)?;
        }
        let completed_absorptions = self
            .lifecycle
            .receipts
            .iter()
            .filter(|receipt| receipt.status == LifecycleReceiptStatus::Succeeded)
            .filter_map(|receipt| {
                self.lifecycle
                    .intents
                    .get(&receipt.action_id)
                    .filter(|intent| matches!(intent.action, LifecycleAction::Absorb { .. }))
                    .map(|intent| (intent.action.clone(), receipt.clone()))
            })
            .collect::<Vec<_>>();
        for (action, receipt) in completed_absorptions {
            self.apply_completed_lifecycle_action(
                &receipt.action_id,
                &action,
                receipt.completed_at_ms,
                receipt.external_reference.as_deref(),
                now_ms,
            )?;
        }
        Ok(())
    }

    fn absorption_target(&self) -> Result<Option<String>> {
        let family = self.lineage.family(&self.local_tentacle_id)?;
        Ok(family
            .parent
            .into_iter()
            .chain(family.siblings)
            .chain(family.children)
            .find(|candidate| {
                self.lineage.state().external_parent_id.as_deref() == Some(candidate.as_str())
                    || self
                        .lineage
                        .node(candidate)
                        .is_some_and(|node| node.lifecycle == TentacleLifecycle::Active)
            }))
    }

    fn reconcile_killed_awakening(&mut self, now: u64) -> Result<()> {
        if !matches!(self.ritual.phase(), AwakeningPhase::Killed { .. })
            || self.lifecycle.death_pending()
            || self
                .lifecycle
                .intents
                .values()
                .any(|intent| matches!(intent.action, LifecycleAction::Shutdown { .. }))
        {
            return Ok(());
        }
        let judgment_id = encode_sha256(
            format!(
                "awakening-kill:{}:{}",
                self.ritual.nature().nature_id,
                self.ritual.epoch()
            )
            .as_bytes(),
        );
        let at_ms = now.saturating_mul(1_000);
        self.lifecycle
            .schedule_death(&judgment_id, at_ms, 0, None)?;
        if let Some(target_id) = self.absorption_target()? {
            self.lifecycle.enqueue_absorption(
                at_ms,
                self.local_tentacle_id.clone(),
                target_id,
                judgment_id.clone(),
            )?;
        }
        self.lifecycle.reconcile_expired_death(at_ms, None)?;
        self.lifecycle_store.save(&self.lifecycle)?;
        Ok(())
    }

    fn apply_final_judgment_lifecycle(
        &mut self,
        record: &EvolutionHistoryRecord,
        action_at_ms: u64,
    ) -> Result<()> {
        if !record.authorizes_automatic_lifecycle() {
            return Ok(());
        }
        match record.judgment.outcome {
            JudgmentOutcome::Dormant | JudgmentOutcome::Death => {}
            JudgmentOutcome::PropagationRights
                if self.lifecycle.auto_spawn_enabled && self.ritual.nature().growth > 70 =>
            {
                if is_accepted_propagation_grant_with_stake(
                    record,
                    &record.metrics,
                    self.ritual.nature(),
                    self.ritual.epoch(),
                    i64::try_from(action_at_ms / 1_000)
                        .context("automatic spawn timestamp exceeds the supported range")?,
                    self.propagation_minimum_stake_basis_points,
                )? {
                    self.auto_spawn_from_grant(record, action_at_ms)?;
                }
            }
            JudgmentOutcome::PropagationRights
            | JudgmentOutcome::Survival
            | JudgmentOutcome::StarvationWarning => {}
        }
        Ok(())
    }

    fn auto_spawn_from_grant(&mut self, grant: &EvolutionHistoryRecord, at_ms: u64) -> Result<()> {
        let desired_children = grant
            .metrics
            .token_economics
            .map_or(0, |economics| economics.snapshot.stake_basis_points)
            .checked_div(self.propagation_minimum_stake_basis_points)
            .unwrap_or(1)
            .max(1);
        let short_judgment = grant
            .judgment_id
            .get(..16)
            .context("judgment ID is too short for automatic child identity")?;
        let mut created = false;
        for index in 0..desired_children {
            let child_id = format!("tentacle-auto-{short_judgment}-{index}");
            if self.lineage.node(&child_id).is_some()
                || self.lifecycle.intents.values().any(|intent| {
                    matches!(&intent.action, LifecycleAction::Spawn { child_id: existing, .. } if existing == &child_id)
                })
            {
                continue;
            }
            let event = format!("auto-spawn:{}:{child_id}", grant.judgment_id);
            let event_id_sha256 = encode_sha256(event.as_bytes());
            let child_nature = self.lineage.plan_child_nature(&self.local_tentacle_id)?;
            self.lifecycle.enqueue_spawn(
                at_ms,
                self.local_tentacle_id.clone(),
                child_id,
                grant.judgment_id.clone(),
                child_nature,
                "evolution-runtime".to_owned(),
                event_id_sha256,
            )?;
            created = true;
        }
        if created {
            self.lifecycle_store.save(&self.lifecycle)?;
        }
        Ok(())
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
        if self.lifecycle.death_pending() || self.lifecycle.shutdown_completed_at_ms.is_some() {
            let response = match text.trim().to_ascii_lowercase().as_str() {
                "/nature" => self.nature_status(),
                "/lineage" => self.lineage_status()?,
                "/metrics" => self.metrics_status_read_only()?,
                "/judgment" => self.judgment_status_read_only()?,
                "/gossip-status" => self.gossip_status(),
                _ => {
                    "DEATH LIFECYCLE IS ACTIVE. NEW CONVERSATIONS AND OPERATOR EFFECTS ARE CLOSED. READ-ONLY STATUS: /nature, /lineage, /metrics, /judgment, /gossip-status. A MATCHING CONFIRMED UWU SURVIVAL BURN MAY REOPEN THE TENTACLE BEFORE GRACE EXPIRES."
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
                    "{}\n\nNORMAL OPERATION IS BLOCKED. THE BINDING LIFECYCLE SHUTDOWN ACTION REMAINS DURABLE UNTIL THE EXECUTOR ACKNOWLEDGES IT.",
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
                AwakeningOutcome::KillRequested => {
                    let judgment_id = encode_sha256(
                        format!("awakening-kill:{operator_id}:{message_id}").as_bytes(),
                    );
                    let at_ms = transition_now.saturating_mul(1_000);
                    if self
                        .lifecycle
                        .schedule_death(&judgment_id, at_ms, 0, None)?
                    {
                        if let Some(target_id) = self.absorption_target()? {
                            self.lifecycle.enqueue_absorption(
                                at_ms,
                                self.local_tentacle_id.clone(),
                                target_id,
                                judgment_id.clone(),
                            )?;
                        }
                        self.lifecycle.reconcile_expired_death(at_ms, None)?;
                        self.lifecycle_store.save(&self.lifecycle)?;
                    }
                    format!(
                        "KILL IS BINDING. NORMAL OPERATION IS BLOCKED AND A DURABLE SHUTDOWN ACTION IS READY FOR THE LIFECYCLE EXECUTOR.\n\n{}",
                        self.nature_status()
                    )
                }
                AwakeningOutcome::AwaitingConfirmation => self.ritual.formatted_prompt(),
                AwakeningOutcome::SkippedForTesting
                | AwakeningOutcome::AcceptedByDefault
                | AwakeningOutcome::AdjustedAfterConfirmation
                | AwakeningOutcome::ForcedRerollEpoch => {
                    bail!("unexpected awakening outcome from an XMTP ritual action")
                }
            }));
        }

        if self.require_node_economics {
            let now = now_unix_seconds()?;
            if !self.node_economics_is_current(now) {
                self.node_economics_available = false;
                let response = match text.trim().to_ascii_lowercase().as_str() {
                    "/nature" => self.nature_status(),
                    "/lineage" => self.lineage_status()?,
                    "/metrics" => self.metrics_status_read_only()?,
                    "/judgment" => self.judgment_status_read_only()?,
                    "/gossip-status" => self.gossip_status(),
                    "/recovery-status" => {
                        "CURRENT BASE UWU TREASURY ECONOMICS ARE UNAVAILABLE. RPC RECOVERY MUST RECORD A FRESH PROVENANCE-BOUND OBSERVATION BEFORE OPERATOR EFFECTS REOPEN."
                            .to_owned()
                    }
                    _ => {
                        "CURRENT BASE UWU TREASURY ECONOMICS ARE UNAVAILABLE. OPERATOR EFFECTS AND TOOL DISPATCH ARE CLOSED. READ-ONLY STATUS: /nature, /lineage, /metrics, /judgment, /gossip-status, /recovery-status."
                            .to_owned()
                    }
                };
                return Ok(Some(response));
            }
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
            "auto-spawn" => self.configure_auto_spawn(arguments)?,
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
        ensure!(
            !self.lifecycle.intents.values().any(|intent| {
                matches!(intent.action, LifecycleAction::Spawn { .. })
                    && self.lifecycle.receipt(&intent.action_id).is_none()
                    && !self
                        .lifecycle
                        .canceled_action_ids
                        .contains(&intent.action_id)
            }),
            "Nature adjustment is deferred while child provisioning is pending"
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

    fn metrics_status_read_only(&self) -> Result<String> {
        let evaluated_at = read_only_evaluation_timestamp(&self.metrics)?;
        let judgment = self
            .metrics
            .evaluate_snapshot(self.ritual.nature(), evaluated_at)?;
        Ok(format!(
            "CURRENT DAILY METRICS (READ-ONLY DURING DEATH)\nPERIOD: {}..{}\nCONVERSATIONS: {}\nRETURNING: {}\nDEPTH TOTAL: {}\nCHILDREN RECORDED: {}\nACOLYTES OBSERVED: {}\n\n{}",
            self.metrics.period_started_at_unix_seconds,
            self.metrics.period_ends_at_unix_seconds,
            self.metrics.engagement.conversations,
            self.metrics.engagement.returning_conversations,
            self.metrics.engagement.conversation_depth_total,
            self.metrics.growth.children_spawned,
            self.metrics.growth.acolytes_recruited,
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

    fn judgment_status_read_only(&self) -> Result<String> {
        Ok(render_judgment(&self.metrics.evaluate_snapshot(
            self.ritual.nature(),
            read_only_evaluation_timestamp(&self.metrics)?,
        )?))
    }

    fn spawn_child(
        &mut self,
        operator_id: &str,
        message_id: &str,
        requested_id: &str,
    ) -> Result<String> {
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
            .context("spawning requires a binding final Propagation Rights judgment")?;
        ensure!(
            self.propagation_grant_is_currently_executable(grant, now)?,
            "spawning requires binding Propagation Rights under the exact economic policy, signed Nature, and awakening epoch"
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
        ensure!(
            self.lineage.node(&child_id).is_none()
                && !self.lifecycle.intents.values().any(|intent| {
                    matches!(&intent.action, LifecycleAction::Spawn { child_id: existing, .. } if existing == &child_id)
                }),
            "a child with this ID already exists or is awaiting provisioning"
        );
        let child_nature = self.lineage.plan_child_nature(&self.local_tentacle_id)?;
        let event_id_sha256 = encode_sha256(message_id.as_bytes());
        let action = self
            .lifecycle
            .enqueue_spawn(
                now.saturating_mul(1_000),
                self.local_tentacle_id.clone(),
                child_id.clone(),
                grant_id.clone(),
                child_nature.clone(),
                operator_id.to_owned(),
                event_id_sha256,
            )?
            .clone();
        if let Err(error) = self.lifecycle_store.save(&self.lifecycle) {
            self.degraded = true;
            return Err(error.into());
        }
        Ok(format!(
            "DURABLE CHILD PROVISIONING ACTION CREATED: {}\nACTION: {}\nPLANNED NATURE: {}\nGENERATION: {}\nFINAL JUDGMENT: {}\nLINEAGE AND GROWTH CREDIT COMMIT ONLY AFTER A MATCHING SUCCESSFUL PROVISION RECEIPT.",
            child_id, action.action_id, child_nature.nature_id, child_nature.generation, grant_id,
        ))
    }

    fn configure_auto_spawn(&mut self, arguments: &str) -> Result<String> {
        let enabled = match arguments.trim().to_ascii_lowercase().as_str() {
            "on" | "true" | "1" => true,
            "off" | "false" | "0" => false,
            _ => return Ok("USAGE: /auto-spawn <on|off>".to_owned()),
        };
        self.set_auto_spawn_enabled(enabled)?;
        Ok(format!(
            "AUTOMATIC SPAWNING {}. PROPAGATION RIGHTS WILL {}PROVISION A CHILD INTENT WHEN NATURE GROWTH EXCEEDS 70.",
            if enabled { "ENABLED" } else { "DISABLED" },
            if enabled { "" } else { "NOT " }
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
        if self.lifecycle.death_pending() {
            self.reconcile_lifecycle_deadline(now.saturating_mul(1_000))?;
            return Ok(None);
        }
        if self.require_node_economics && !self.node_economics_is_current(now) {
            self.node_economics_available = false;
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
            self.history_catalog.insert(&record)?;
        }
        self.last_final_judgment = Some(record.clone());
        self.metrics = new_runtime_metrics(
            EvaluationPeriod::Daily,
            aligned_period_start(u64::try_from(now).unwrap_or(u64::MAX))?,
            self.ritual.nature(),
            self.ritual.epoch(),
        )?;
        self.scales_store.save_metrics(&self.metrics)?;
        self.apply_final_judgment_lifecycle(
            &record,
            u64::try_from(now)
                .context("current timestamp predates the Unix epoch")?
                .saturating_mul(1_000),
        )?;
        Ok(Some(judgment))
    }
}

fn render_judgment(judgment: &Judgment) -> String {
    format!(
        "JUDGMENT {:?} ({:?})\nSCORE: {}/10000 (PRE-STRESS {}, PENALTY {})\nSCALES: ENGAGEMENT {}, GROWTH {}, WEALTH {}, INFLUENCE {}\nPROPAGATION EVIDENCE: {} conversations / {} prior-day returns (requires {} / {}; eligible: {})\nEXECUTION: {:?}. FINAL DEATH AND PROPAGATION OUTCOMES CREATE DURABLE AUTONOMOUS LIFECYCLE ACTIONS.",
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

fn is_accepted_propagation_grant_with_stake(
    record: &EvolutionHistoryRecord,
    current_metrics: &TentacleMetrics,
    nature: &TentacleNature,
    awakening_epoch: u64,
    now: i64,
    propagation_minimum_stake_basis_points: u16,
) -> Result<bool> {
    if !propagation_grant_history_is_authorized(
        record,
        nature,
        awakening_epoch,
        propagation_minimum_stake_basis_points,
    )? {
        return Ok(false);
    }
    let Some(grant_economics) = record.metrics.token_economics else {
        return Ok(propagation_minimum_stake_basis_points == 0);
    };
    let Some(current_economics) = current_metrics.token_economics else {
        return Ok(false);
    };
    let (Some(grant_provenance), Some(current_provenance)) =
        (grant_economics.provenance, current_economics.provenance)
    else {
        return Ok(false);
    };
    let current_binding_matches_grant = current_provenance.chain_id == grant_provenance.chain_id
        && current_provenance.holder_role == EconomicHolderRole::TentacleTreasury
        && current_provenance.holder_address == grant_provenance.holder_address
        && current_provenance.token_contract == grant_provenance.token_contract
        && current_provenance.configuration_identity == grant_provenance.configuration_identity;
    Ok(current_metrics.nature_id == nature.nature_id
        && current_metrics.nature_fingerprint == nature.fingerprint()?
        && current_metrics.awakening_epoch == awakening_epoch
        && current_economics.snapshot.trustworthy
        && current_economics.provenance.is_some()
        && current_economics.validate().is_ok()
        && current_economics.policy
            == runtime_token_policy(nature, propagation_minimum_stake_basis_points)?
        && current_economics.effects.propagation_stake_eligible
        && current_binding_matches_grant
        && i64::try_from(current_provenance.observed_at_unix_seconds)
            .is_ok_and(|observed_at| observed_at <= now))
}

fn propagation_grant_history_is_authorized(
    record: &EvolutionHistoryRecord,
    nature: &TentacleNature,
    awakening_epoch: u64,
    propagation_minimum_stake_basis_points: u16,
) -> Result<bool> {
    if record.validate().is_err() {
        return Ok(false);
    }
    let scoring_policy_is_accepted = match record.metrics.token_economics {
        None => {
            propagation_minimum_stake_basis_points == 0
                && record.metrics.scored_scale_availability == RUNTIME_SCORED_SCALES
                && record.judgment.scored_scale_availability == RUNTIME_SCORED_SCALES
        }
        Some(economics) => {
            let expected_availability = token_runtime_scored_scales(economics.snapshot);
            economics.snapshot.trustworthy
                && economics.provenance.is_some()
                && economics.validate().is_ok()
                && economics.policy
                    == runtime_token_policy(nature, propagation_minimum_stake_basis_points)?
                && record.metrics.scored_scale_availability == expected_availability
                && record.judgment.scored_scale_availability == expected_availability
        }
    };
    Ok(scoring_policy_is_accepted
        && record.authorizes_automatic_lifecycle()
        && record.nature_id == nature.nature_id
        && record.nature_fingerprint == nature.fingerprint()?
        && record.awakening_epoch == awakening_epoch
        && record.judgment.evaluation_status == EvaluationStatus::Final
        && record.judgment.outcome == JudgmentOutcome::PropagationRights
        && record.judgment.policy == JudgmentPolicy::for_period(record.metrics.period)
        && record.judgment.propagation_evidence.eligible)
}

#[cfg(test)]
fn is_accepted_propagation_grant(
    record: &EvolutionHistoryRecord,
    current_metrics: &TentacleMetrics,
    nature: &TentacleNature,
    awakening_epoch: u64,
    now: i64,
) -> Result<bool> {
    is_accepted_propagation_grant_with_stake(
        record,
        current_metrics,
        nature,
        awakening_epoch,
        now,
        DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS,
    )
}

fn runtime_token_policy(
    nature: &TentacleNature,
    propagation_minimum_stake_basis_points: u16,
) -> Result<TokenEconomicPolicy> {
    let mut policy = TokenEconomicPolicy::default().with_nature_appetites(
        nature.engagement,
        nature.growth,
        nature.wealth,
        nature.influence,
    )?;
    policy.propagation_minimum_stake_basis_points = propagation_minimum_stake_basis_points;
    policy.validate()?;
    Ok(policy)
}

fn ensure_economic_configuration_continuity(
    metrics: &TentacleMetrics,
    incoming: EconomicObservationProvenance,
) -> Result<()> {
    if let Some(existing) = metrics
        .token_economics
        .and_then(|economics| economics.provenance)
        && (existing.configuration_identity != incoming.configuration_identity
            || existing.holder_address != incoming.holder_address
            || existing.token_contract != incoming.token_contract
            || existing.chain_id != incoming.chain_id)
    {
        ensure!(
            !metrics.has_behavior_observations(),
            "node economic identity/configuration cannot change inside an observed metrics period"
        );
    }
    Ok(())
}

const fn token_runtime_scored_scales(_snapshot: TokenEconomicSnapshot) -> ScoredScaleAvailability {
    ScoredScaleAvailability {
        engagement: true,
        growth: true,
        wealth: true,
        influence: true,
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
    history: &ValidatedHistoryCatalog,
) -> Result<()> {
    for spawn in &lineage.state().spawn_records {
        let grant = history
            .get(&spawn.authorization_judgment_id)?
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
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_absorption_lineage_projection(
    lineage: &mut Lineage,
    action: &LifecycleAction,
    receipt: &LifecycleReceipt,
    minimum_projection_at_ms: u64,
    locally_observed_at_ms: u64,
) -> Result<(u64, bool)> {
    let LifecycleAction::Absorb {
        source_id,
        target_id,
        ..
    } = action
    else {
        bail!("lineage absorption projection requires an Absorb action")
    };
    ensure!(
        receipt.status == LifecycleReceiptStatus::Succeeded
            && locally_observed_at_ms >= minimum_projection_at_ms
            && receipt.completed_at_ms
                <= locally_observed_at_ms.saturating_add(LIFECYCLE_RECEIPT_CLOCK_SKEW_MS),
        "lineage absorption projection requires a locally timely successful executor receipt"
    );
    let manifest_hash = receipt
        .external_reference
        .as_deref()
        .context("successful absorption is missing its transfer-manifest hash")?;
    let source_lifecycle = lineage
        .node(source_id)
        .context("absorption source disappeared from lineage")?
        .lifecycle
        .clone();
    match source_lifecycle {
        TentacleLifecycle::Active => {
            let projected_at_ms = locally_observed_at_ms;
            if lineage.node(target_id).is_some() {
                lineage.record_absorption(
                    target_id,
                    target_id,
                    source_id,
                    projected_at_ms,
                    vec![manifest_hash.to_owned()],
                    false,
                )?;
            } else {
                lineage.record_external_parent_absorption(
                    source_id,
                    target_id,
                    projected_at_ms,
                    vec![manifest_hash.to_owned()],
                )?;
            }
            Ok((projected_at_ms, true))
        }
        TentacleLifecycle::Absorbed { into, at_ms } => {
            ensure!(
                into == *target_id
                    && at_ms >= minimum_projection_at_ms
                    && at_ms <= locally_observed_at_ms
                    && receipt.completed_at_ms
                        <= at_ms.saturating_add(LIFECYCLE_RECEIPT_CLOCK_SKEW_MS)
                    && lineage.state().absorption_records.iter().any(|record| {
                        record.source_id == *source_id
                            && record.target_id == *target_id
                            && record.at_ms == at_ms
                            && record.knowledge_hashes == vec![manifest_hash.to_owned()]
                    }),
                "persisted lineage absorption does not match its executor receipt"
            );
            Ok((at_ms, false))
        }
    }
}

struct PendingLifecycleValidation<'a> {
    history: &'a ValidatedHistoryCatalog,
    current_metrics: &'a TentacleMetrics,
    nature: &'a TentacleNature,
    awakening_epoch: u64,
    propagation_minimum_stake_basis_points: u16,
    survival_total_supply_whole: u64,
    survival_token_decimals: u8,
}

fn validate_pending_lifecycle_intents(
    lifecycle: &LifecycleState,
    lineage: &Lineage,
    validation: PendingLifecycleValidation<'_>,
) -> Result<()> {
    let PendingLifecycleValidation {
        history,
        current_metrics,
        nature,
        awakening_epoch,
        propagation_minimum_stake_basis_points,
        survival_total_supply_whole,
        survival_token_decimals,
    } = validation;
    let local_id = &lifecycle.tentacle_id;
    let family = lineage.family(local_id)?;
    let valid_absorption_targets = family
        .parent
        .into_iter()
        .chain(family.siblings)
        .chain(family.children)
        .collect::<BTreeSet<_>>();
    for intent in lifecycle.intents.values().filter(|intent| {
        lifecycle.receipt(&intent.action_id).is_none()
            && !lifecycle.canceled_action_ids.contains(&intent.action_id)
    }) {
        match &intent.action {
            LifecycleAction::SpendForSurvival {
                tentacle_id,
                judgment_id,
                grace_ends_at_ms,
                chain_id,
                token_contract,
                treasury_address,
                configuration_identity,
                exact_amount,
                ..
            } => {
                ensure!(
                    tentacle_id == local_id
                        && lifecycle.pending_death.as_ref().is_some_and(|pending| {
                            pending.judgment_id == *judgment_id
                                && pending.grace_ends_at_ms == *grace_ends_at_ms
                        })
                        && exact_amount.total_supply_whole == survival_total_supply_whole
                        && exact_amount.token_decimals == survival_token_decimals,
                    "pending survival spend is not bound to the exact local Death and token amount configuration"
                );
                if let Some(provenance) = current_metrics
                    .token_economics
                    .and_then(|economics| economics.provenance)
                {
                    ensure!(
                        provenance.chain_id == *chain_id
                            && provenance.token_contract == *token_contract
                            && provenance.holder_address == *treasury_address
                            && provenance.configuration_identity == *configuration_identity,
                        "pending survival spend does not match current bound economic provenance"
                    );
                }
            }
            LifecycleAction::Absorb {
                source_id,
                target_id,
                judgment_id,
            } => ensure!(
                source_id == local_id
                    && valid_absorption_targets.contains(target_id)
                    && lifecycle
                        .pending_death
                        .as_ref()
                        .is_some_and(|pending| pending.judgment_id == *judgment_id),
                "pending absorption is not bound to the exact local Death and lineage target"
            ),
            LifecycleAction::RewardVeniceKey {
                tentacle_id,
                chain_id,
                token_contract,
                treasury_address,
                configuration_identity,
                exact_amount,
                ..
            } => {
                let provenance = current_metrics
                    .token_economics
                    .and_then(|economics| economics.provenance)
                    .context("pending Venice-key reward requires bound node economics")?;
                ensure!(
                    tentacle_id == local_id
                        && *chain_id == provenance.chain_id
                        && *token_contract == provenance.token_contract
                        && *treasury_address == provenance.holder_address
                        && *configuration_identity == provenance.configuration_identity
                        && exact_amount.token_decimals == survival_token_decimals,
                    "pending Venice-key reward does not match current bound Tentacle economics"
                );
            }
            LifecycleAction::RewardAcolyteContribution {
                tentacle_id,
                chain_id,
                token_contract,
                treasury_address,
                configuration_identity,
                exact_amount,
                ..
            } => {
                let provenance = current_metrics
                    .token_economics
                    .and_then(|economics| economics.provenance)
                    .context("pending contribution reward requires bound node economics")?;
                ensure!(
                    tentacle_id == local_id
                        && *chain_id == provenance.chain_id
                        && *token_contract == provenance.token_contract
                        && *treasury_address == provenance.holder_address
                        && *configuration_identity == provenance.configuration_identity
                        && exact_amount.token_decimals == survival_token_decimals,
                    "pending contribution reward does not match current bound Tentacle economics"
                );
            }
            LifecycleAction::Spawn {
                parent_id,
                judgment_id,
                child_nature,
                ..
            } => {
                let grant = history.get(judgment_id)?.with_context(|| {
                    format!("pending Spawn references missing judgment {judgment_id}")
                })?;
                ensure!(
                    parent_id == local_id
                        && child_nature.parent_nature_id.as_deref()
                            == Some(nature.nature_id.as_str())
                        && propagation_grant_history_is_authorized(
                            &grant,
                            nature,
                            awakening_epoch,
                            propagation_minimum_stake_basis_points,
                        )?,
                    "pending Spawn is not bound to the current Nature and exact economic propagation grant"
                );
            }
            LifecycleAction::Shutdown {
                tentacle_id,
                judgment_id,
                ..
            } => ensure!(
                tentacle_id == local_id
                    && lifecycle
                        .pending_death
                        .as_ref()
                        .is_some_and(|pending| pending.judgment_id == *judgment_id),
                "pending Shutdown is not bound to the exact local Death"
            ),
        }
    }
    Ok(())
}

fn restrict_runtime_scales(
    metrics: &mut TentacleMetrics,
    nature: &TentacleNature,
    propagation_minimum_stake_basis_points: u16,
) -> Result<bool> {
    let expected = match metrics.token_economics {
        Some(economics) if economics.snapshot.trustworthy => {
            ensure!(
                economics.policy
                    == runtime_token_policy(nature, propagation_minimum_stake_basis_points)?,
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

fn read_only_evaluation_timestamp(metrics: &TentacleMetrics) -> Result<i64> {
    let now =
        i64::try_from(now_unix_seconds()?).context("current timestamp exceeds metrics range")?;
    Ok(now
        .max(metrics.period_started_at_unix_seconds)
        .min(metrics.period_ends_at_unix_seconds.saturating_sub(1)))
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

    fn open_confirmed_with_zero_stake(
        data_dir: &Path,
        workspace: &Path,
    ) -> Result<EvolutionRuntime> {
        EvolutionRuntime::open(
            data_dir,
            workspace,
            EvolutionStartupOptions {
                skip_awakening: true,
                propagation_minimum_stake_basis_points: 0,
                ..EvolutionStartupOptions::default()
            },
        )
    }

    fn catalog_with(records: &[EvolutionHistoryRecord]) -> ValidatedHistoryCatalog {
        let root = tempfile::tempdir().unwrap();
        let store = ScalesStore::new(root.path()).unwrap();
        for record in records {
            store.append_history(record).unwrap();
        }
        store.history_catalog().unwrap()
    }

    fn propagation_test_nature() -> TentacleNature {
        TentacleNature {
            schema_version: 1,
            nature_id: "0123456789abcdef0123456789abcdef".to_owned(),
            generation: 0,
            parent_nature_id: None,
            engagement: 100,
            growth: 0,
            wealth: 0,
            influence: 0,
            cooperation: 50,
            stability: 50,
            transparency: 50,
            sacred_ban: SacredBan::MemorySharing,
        }
    }

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
    fn production_default_accepts_fresh_or_legacy_pending_nature_without_an_operator() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();

        let pending = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions::default(),
        )
        .unwrap();
        assert!(!pending.permits_normal_operation());
        drop(pending);

        let active = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                auto_accept_nature: true,
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();
        assert!(active.permits_normal_operation());
        assert!(matches!(
            active.ritual.phase(),
            AwakeningPhase::AcceptedByDefault { .. }
        ));
        assert!(
            active
                .nature_status()
                .contains("SAFE DEFAULT NATURE ACCEPTED LOCALLY")
        );
        let entries = active.awakening_log.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].normalized_action, "ACCEPT DEFAULT NATURE");
        drop(active);

        let resumed = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                auto_accept_nature: true,
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();
        assert!(resumed.permits_normal_operation());
        assert_eq!(resumed.awakening_log.entries().unwrap().len(), 1);
    }

    #[test]
    fn preconfirmation_startup_economics_stay_unobserved_and_repair_legacy_seed() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let now = now_unix_seconds().unwrap();
        let snapshot = TokenEconomicSnapshot {
            balance_basis_points: 10_000,
            stake_basis_points: 0,
            reward_basis_points: 0,
            trustworthy: true,
        };
        let provenance = EconomicObservationProvenance::base(
            [1; 20],
            EconomicHolderRole::TentacleTreasury,
            [2; 20],
            now,
            None,
            [3; 32],
        )
        .unwrap();
        let mut runtime = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                require_node_economics: true,
                initial_node_economics: Some((snapshot, provenance)),
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();
        assert!(!runtime.ritual.is_confirmed());
        assert!(runtime.metrics.token_economics.is_none());
        let PublicTurnStart::Gated(message) = runtime.begin_public_turn().unwrap() else {
            panic!("unconfirmed runtime must gate public conversation");
        };
        assert!(message.contains("Nature transition finishes"));
        assert!(!message.contains("economics are unavailable"));
        assert!(
            runtime
                .record_node_economic_observation(snapshot, provenance)
                .unwrap_err()
                .to_string()
                .contains("before awakening confirmation")
        );
        let nature = runtime.nature().clone();
        drop(runtime);

        let store = ScalesStore::new(root.path()).unwrap();
        let mut legacy = store.load_metrics().unwrap().unwrap();
        legacy
            .record_node_token_economic_observation(
                snapshot,
                runtime_token_policy(&nature, DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS).unwrap(),
                provenance,
            )
            .unwrap();
        store.save_metrics(&legacy).unwrap();

        let repaired = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                require_node_economics: true,
                initial_node_economics: Some((snapshot, provenance)),
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();
        assert!(!repaired.ritual.is_confirmed());
        assert!(repaired.metrics.token_economics.is_none());
        assert!(!repaired.metrics.has_behavior_observations());
    }

    #[test]
    fn read_only_preflight_distinguishes_spawn_outbox_from_mandatory_death_recovery() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        ensure_private_directory(root.path()).unwrap();
        assert!(!EvolutionRuntime::has_mandatory_recovery_work(root.path()).unwrap());
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        let child_nature = runtime
            .lineage
            .plan_child_nature(&runtime.local_tentacle_id)
            .unwrap();
        runtime
            .lifecycle
            .enqueue_spawn(
                1,
                runtime.local_tentacle_id.clone(),
                "preflight-child".to_owned(),
                "a".repeat(64),
                child_nature,
                "evolution-runtime".to_owned(),
                "b".repeat(64),
            )
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        assert!(!EvolutionRuntime::has_mandatory_recovery_work(root.path()).unwrap());
        let death_at_ms = now_unix_seconds().unwrap().saturating_mul(1_000);
        runtime
            .lifecycle
            .schedule_death(&"c".repeat(64), death_at_ms, DEATH_GRACE_PERIOD_MS, None)
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        assert!(EvolutionRuntime::has_mandatory_recovery_work(root.path()).unwrap());
        assert_eq!(
            EvolutionRuntime::mandatory_recovery_kind(root.path()).unwrap(),
            MandatoryRecoveryKind::ShutdownDueOrPending
        );
        runtime
            .lifecycle
            .enqueue_absorption(
                death_at_ms,
                runtime.local_tentacle_id.clone(),
                "external-parent".to_owned(),
                "c".repeat(64),
            )
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        assert_eq!(
            EvolutionRuntime::mandatory_recovery_kind(root.path()).unwrap(),
            MandatoryRecoveryKind::AbsorptionRequired
        );
        let pending = runtime.lifecycle.pending_death.as_mut().unwrap();
        pending.scheduled_at_ms = 1;
        pending.grace_ends_at_ms = 1;
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        assert_eq!(
            EvolutionRuntime::mandatory_recovery_kind(root.path()).unwrap(),
            MandatoryRecoveryKind::ShutdownDueOrPending
        );
        assert!(matches!(
            runtime
                .due_native_shutdown_action(now_unix_seconds().unwrap())
                .unwrap()
                .unwrap()
                .action,
            LifecycleAction::Shutdown { .. }
        ));
    }

    #[test]
    fn lifecycle_only_startup_completes_due_shutdown_with_unfinished_absorption() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        let now = now_unix_seconds().unwrap();
        let now_ms = now.saturating_mul(1_000);
        let scheduled_at_ms = now_ms.saturating_sub(1_000);
        let judgment_id = "d".repeat(64);
        runtime
            .lifecycle
            .schedule_death(&judgment_id, scheduled_at_ms, 0, None)
            .unwrap();
        let absorption = runtime
            .lifecycle
            .enqueue_absorption(
                scheduled_at_ms,
                runtime.local_tentacle_id.clone(),
                "external-parent".to_owned(),
                judgment_id.clone(),
            )
            .unwrap()
            .clone();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        drop(runtime);

        // This recovery path intentionally does not parse unrelated projections.
        fs::write(root.path().join("state").join("metrics.json"), b"not-json").unwrap();
        let receipt = EvolutionRuntime::try_complete_due_native_shutdown(root.path(), now)
            .unwrap()
            .unwrap();
        assert_eq!(
            receipt.external_reference.as_deref(),
            Some("native-transport-never-started")
        );

        let lifecycle = LifecycleStore::new(root.path())
            .unwrap()
            .load()
            .unwrap()
            .unwrap();
        assert_eq!(
            lifecycle.shutdown_completed_at_ms,
            Some(receipt.completed_at_ms)
        );
        assert!(lifecycle.receipt(&absorption.action_id).is_none());
        let shutdown = lifecycle.intents.get(&receipt.action_id).unwrap();
        assert!(matches!(
            &shutdown.action,
            LifecycleAction::Shutdown {
                tentacle_id,
                judgment_id: action_judgment,
                after_action_id: Some(dependency),
            } if tentacle_id == &lifecycle.tentacle_id
                && action_judgment == &judgment_id
                && dependency == &absorption.action_id
        ));

        let replay = EvolutionRuntime::complete_due_native_shutdown(root.path(), now).unwrap();
        assert_eq!(replay, receipt);
    }

    #[test]
    fn legacy_local_shutdown_migrates_to_dormancy_without_replacing_identity() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        let tentacle_id = runtime.local_tentacle_id.clone();
        let metrics = runtime.metrics.clone();
        let mut judgment = metrics
            .evaluate(runtime.nature(), metrics.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(judgment.outcome, JudgmentOutcome::Dormant);
        judgment.outcome = JudgmentOutcome::Death;
        let record = EvolutionHistoryRecord::new(runtime.nature(), metrics, judgment).unwrap();
        let judgment_id = record.judgment_id.clone();
        runtime.scales_store.append_history(&record).unwrap();
        runtime
            .lifecycle
            .schedule_death(&judgment_id, 1, 0, None)
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        drop(runtime);

        EvolutionRuntime::complete_due_native_shutdown(root.path(), 2).unwrap();
        assert_eq!(
            EvolutionRuntime::mandatory_recovery_kind(root.path()).unwrap(),
            MandatoryRecoveryKind::CompletedShutdown
        );
        assert!(EvolutionRuntime::migrate_legacy_death_to_dormancy(root.path()).unwrap());

        let lifecycle = LifecycleStore::new(root.path())
            .unwrap()
            .load()
            .unwrap()
            .unwrap();
        assert_eq!(lifecycle.tentacle_id, tentacle_id);
        assert!(lifecycle.pending_death.is_none());
        assert!(lifecycle.shutdown_completed_at_ms.is_none());
        assert!(lifecycle.receipts.iter().any(|receipt| {
            receipt.status == LifecycleReceiptStatus::Succeeded
                && matches!(
                    lifecycle.intents[&receipt.action_id].action,
                    LifecycleAction::Shutdown { .. }
                )
        }));
        assert_eq!(
            EvolutionRuntime::mandatory_recovery_kind(root.path()).unwrap(),
            MandatoryRecoveryKind::None
        );
    }

    #[test]
    fn explicit_non_scales_death_is_not_migrated_as_low_resource_dormancy() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        runtime
            .lifecycle
            .schedule_death(&"8".repeat(64), 1, 10_000, None)
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        drop(runtime);

        assert!(!EvolutionRuntime::migrate_legacy_death_to_dormancy(root.path()).unwrap());
        assert_eq!(
            EvolutionRuntime::mandatory_recovery_kind(root.path()).unwrap(),
            MandatoryRecoveryKind::ShutdownDueOrPending
        );
    }

    #[test]
    fn lifecycle_only_startup_never_completes_shutdown_before_grace_deadline() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        let now = now_unix_seconds().unwrap();
        runtime
            .lifecycle
            .schedule_death(&"e".repeat(64), now.saturating_mul(1_000), 60_000, None)
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        drop(runtime);

        assert_eq!(
            EvolutionRuntime::try_complete_due_native_shutdown(root.path(), now).unwrap(),
            None
        );
        assert!(
            EvolutionRuntime::complete_due_native_shutdown(root.path(), now)
                .unwrap_err()
                .to_string()
                .contains("cannot complete before")
        );
        let lifecycle = LifecycleStore::new(root.path())
            .unwrap()
            .load()
            .unwrap()
            .unwrap();
        assert!(lifecycle.shutdown_completed_at_ms.is_none());
        assert!(
            !lifecycle
                .intents
                .values()
                .any(|intent| { matches!(intent.action, LifecycleAction::Shutdown { .. }) })
        );
    }

    #[test]
    fn completed_shutdown_restart_repairs_absorption_receipt_lineage_crash_window() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let parent_nature = TentacleNature::random().unwrap();
        let child_nature = parent_nature.inherit().unwrap().nature;
        let mut runtime = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                skip_awakening: true,
                propagation_minimum_stake_basis_points: 0,
                child_bootstrap: Some(ChildBootstrap {
                    provisioning_action_id: "1".repeat(64),
                    tentacle_id: "inherited-child".to_owned(),
                    parent_id: "external-parent".to_owned(),
                    inherited_nature: child_nature,
                }),
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();
        let now = now_unix_seconds().unwrap();
        let now_ms = now.saturating_mul(1_000);
        let judgment_id = "f".repeat(64);
        runtime
            .lifecycle
            .schedule_death(&judgment_id, now_ms.saturating_sub(1_000), 0, None)
            .unwrap();
        let absorption = runtime
            .lifecycle
            .enqueue_absorption(
                now_ms.saturating_sub(1_000),
                "inherited-child".to_owned(),
                "external-parent".to_owned(),
                judgment_id,
            )
            .unwrap()
            .clone();
        runtime
            .lifecycle
            .acknowledge_action(LifecycleReceipt {
                action_id: absorption.action_id.clone(),
                completed_at_ms: now_ms,
                status: LifecycleReceiptStatus::Succeeded,
                external_reference: Some("a".repeat(64)),
                detail: None,
                confirmed_chain_receipt: None,
                confirmed_transfer_receipt: None,
                provision_receipt: None,
            })
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        assert!(
            runtime
                .lifecycle
                .pending_absorption_projection_action_ids
                .contains(&absorption.action_id)
        );
        assert_eq!(
            runtime.lineage.node("inherited-child").unwrap().lifecycle,
            TentacleLifecycle::Active
        );
        drop(runtime);

        EvolutionRuntime::try_complete_due_native_shutdown(root.path(), now)
            .unwrap()
            .unwrap();
        assert_eq!(
            EvolutionRuntime::mandatory_recovery_kind(root.path()).unwrap(),
            MandatoryRecoveryKind::AbsorptionProjectionRequired
        );
        // The repair boundary must remain independent of unrelated startup projections/options.
        fs::write(root.path().join("state").join("metrics.json"), b"hostile").unwrap();
        fs::write(root.path().join("state").join("nature.json"), b"hostile").unwrap();
        assert!(EvolutionRuntime::repair_absorption_projection(root.path()).unwrap());
        let lineage = LineageStore::new(root.path())
            .unwrap()
            .load()
            .unwrap()
            .unwrap();
        assert!(matches!(
            lineage.node("inherited-child").unwrap().lifecycle,
            TentacleLifecycle::Absorbed { ref into, .. } if into == "external-parent"
        ));
        let lifecycle = LifecycleStore::new(root.path())
            .unwrap()
            .load()
            .unwrap()
            .unwrap();
        assert!(
            lifecycle
                .absorption_projections
                .contains_key(&absorption.action_id)
        );
        assert!(
            !lifecycle
                .pending_absorption_projection_action_ids
                .contains(&absorption.action_id)
        );
        assert_eq!(
            EvolutionRuntime::mandatory_recovery_kind(root.path()).unwrap(),
            MandatoryRecoveryKind::CompletedShutdown
        );
        assert!(!EvolutionRuntime::repair_absorption_projection(root.path()).unwrap());
    }

    #[test]
    fn stale_required_node_economics_closes_operator_effect_and_harness_lanes() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let now = now_unix_seconds().unwrap();
        let mut runtime = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                skip_awakening: true,
                require_node_economics: true,
                initial_node_economics: Some((
                    TokenEconomicSnapshot {
                        balance_basis_points: 10_000,
                        stake_basis_points: DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS,
                        reward_basis_points: 0,
                        trustworthy: true,
                    },
                    EconomicObservationProvenance::base(
                        [1; 20],
                        EconomicHolderRole::TentacleTreasury,
                        [2; 20],
                        now,
                        Some(1),
                        [3; 32],
                    )
                    .unwrap(),
                )),
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();
        runtime.mark_node_economics_unavailable();
        for text in ["plain operator request", "/adjust growth 100", "/exec true"] {
            let response = runtime
                .handle_operator_message(OPERATOR, "stale-economics", text)
                .unwrap()
                .expect("stale economics must never fall through to operator tools");
            assert!(response.contains("OPERATOR EFFECTS AND TOOL DISPATCH ARE CLOSED"));
        }
        assert!(
            runtime
                .handle_operator_message(OPERATOR, "status", "/nature")
                .unwrap()
                .unwrap()
                .contains("Nature ")
        );
    }

    #[test]
    fn transient_refresh_failure_preserves_only_a_fresh_verified_observation() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let now = now_unix_seconds().unwrap();
        let mut runtime = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                skip_awakening: true,
                require_node_economics: true,
                node_economics_ttl_seconds: 120,
                initial_node_economics: Some((
                    TokenEconomicSnapshot {
                        balance_basis_points: 10_000,
                        stake_basis_points: DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS,
                        reward_basis_points: 0,
                        trustworthy: true,
                    },
                    EconomicObservationProvenance::base(
                        [1; 20],
                        EconomicHolderRole::TentacleTreasury,
                        [2; 20],
                        now,
                        Some(1),
                        [3; 32],
                    )
                    .unwrap(),
                )),
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();

        assert!(!runtime.mark_node_economics_unavailable_if_stale(now + 60));
        assert!(runtime.node_economics_is_current(now + 60));

        assert!(runtime.mark_node_economics_unavailable_if_stale(now + 121));
        assert!(!runtime.node_economics_is_current(now + 121));
    }

    #[test]
    fn economic_refresh_reconciliation_failure_marks_runtime_unavailable_and_degraded() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let now = now_unix_seconds().unwrap();
        runtime
            .lifecycle
            .schedule_death(
                &"e".repeat(64),
                now.saturating_mul(1_000),
                DEATH_GRACE_PERIOD_MS,
                None,
            )
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        let error = runtime
            .record_node_economic_observation(
                TokenEconomicSnapshot {
                    balance_basis_points: 10_000,
                    stake_basis_points: DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS,
                    reward_basis_points: 0,
                    trustworthy: true,
                },
                EconomicObservationProvenance::base(
                    [1; 20],
                    EconomicHolderRole::TentacleTreasury,
                    [2; 20],
                    now,
                    Some(1),
                    [3; 32],
                )
                .unwrap(),
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("missing its final judgment history")
        );
        assert!(runtime.degraded);
        assert!(!runtime.node_economics_available);
    }

    #[test]
    fn startup_rejects_pending_survival_spend_with_wrong_token_amount_configuration() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        runtime
            .lifecycle
            .schedule_death(
                &"f".repeat(64),
                1_000,
                DEATH_GRACE_PERIOD_MS,
                Some(SurvivalSpendBinding {
                    expenditure_basis_points: 500,
                    chain_id: 8_453,
                    token_contract: [2; 20],
                    treasury_address: [1; 20],
                    configuration_identity: [3; 32],
                    exact_amount: crate::evolution::ExactTokenAmount {
                        total_supply_whole: 2_000_000_000,
                        token_decimals: runtime.survival_token_decimals,
                        basis_points: 500,
                        raw_amount: exact_raw_token_amount(
                            2_000_000_000,
                            runtime.survival_token_decimals,
                            500,
                        )
                        .unwrap(),
                    },
                }),
            )
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        drop(runtime);
        let error = EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path())
            .err()
            .unwrap();
        assert!(error.to_string().contains("token amount configuration"));
    }

    #[test]
    fn binding_death_rejects_a_late_in_flight_spawn_success() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        let now_ms = now_unix_seconds().unwrap().saturating_mul(1_000);
        let child_nature = runtime
            .lineage
            .plan_child_nature(&runtime.local_tentacle_id)
            .unwrap();
        let intent = runtime
            .lifecycle
            .enqueue_spawn(
                now_ms,
                runtime.local_tentacle_id.clone(),
                "late-child".to_owned(),
                "a".repeat(64),
                child_nature.clone(),
                "evolution-runtime".to_owned(),
                "b".repeat(64),
            )
            .unwrap()
            .clone();
        runtime
            .lifecycle
            .schedule_death(&"c".repeat(64), now_ms, DEATH_GRACE_PERIOD_MS, None)
            .unwrap();
        let error = runtime
            .ack_lifecycle_action(LifecycleReceipt {
                action_id: intent.action_id.clone(),
                completed_at_ms: now_ms.saturating_add(1),
                status: LifecycleReceiptStatus::Succeeded,
                external_reference: None,
                detail: None,
                confirmed_chain_receipt: None,
                confirmed_transfer_receipt: None,
                provision_receipt: Some(crate::evolution::ProvisionReceipt {
                    child_id: "late-child".to_owned(),
                    child_nature_fingerprint: child_nature.fingerprint().unwrap(),
                    manifest_sha256: "d".repeat(64),
                }),
            })
            .unwrap_err();
        assert!(error.to_string().contains("after a binding Death began"));
        assert!(runtime.lifecycle.receipt(&intent.action_id).is_none());
        assert!(runtime.lineage.node("late-child").is_none());
    }

    #[test]
    fn future_absorption_receipt_cannot_preempt_the_death_grace_period() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        let now_ms = now_unix_seconds().unwrap().saturating_mul(1_000);
        let judgment_id = "7".repeat(64);
        let grace_period_ms = LIFECYCLE_RECEIPT_CLOCK_SKEW_MS / 3;
        runtime
            .lifecycle
            .schedule_death(&judgment_id, now_ms, grace_period_ms, None)
            .unwrap();
        let intent = runtime
            .lifecycle
            .enqueue_absorption(
                now_ms,
                runtime.local_tentacle_id.clone(),
                "future-target".to_owned(),
                judgment_id,
            )
            .unwrap()
            .clone();
        runtime
            .ack_lifecycle_action(LifecycleReceipt {
                action_id: intent.action_id.clone(),
                completed_at_ms: now_ms.saturating_add(grace_period_ms).saturating_add(1),
                status: LifecycleReceiptStatus::Succeeded,
                external_reference: Some("8".repeat(64)),
                detail: None,
                confirmed_chain_receipt: None,
                confirmed_transfer_receipt: None,
                provision_receipt: None,
            })
            .unwrap();
        assert!(runtime.lifecycle.receipt(&intent.action_id).is_some());
        assert_eq!(
            runtime
                .lineage
                .node(&runtime.local_tentacle_id)
                .unwrap()
                .lifecycle,
            TentacleLifecycle::Active
        );
        assert!(runtime.lifecycle.pending_death.is_some());
    }

    #[test]
    fn spawn_projection_uses_local_observation_and_rejects_unbounded_future_receipts() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        let now_ms = now_unix_seconds().unwrap().saturating_mul(1_000);
        let child_nature = runtime
            .lineage
            .plan_child_nature(&runtime.local_tentacle_id)
            .unwrap();
        let bounded = runtime
            .lifecycle
            .enqueue_spawn(
                now_ms,
                runtime.local_tentacle_id.clone(),
                "bounded-clock-child".to_owned(),
                "9".repeat(64),
                child_nature.clone(),
                "evolution-runtime".to_owned(),
                "a".repeat(64),
            )
            .unwrap()
            .clone();
        let executor_time = now_ms.saturating_add(LIFECYCLE_RECEIPT_CLOCK_SKEW_MS / 2);
        runtime
            .ack_lifecycle_action(LifecycleReceipt {
                action_id: bounded.action_id,
                completed_at_ms: executor_time,
                status: LifecycleReceiptStatus::Succeeded,
                external_reference: None,
                detail: None,
                confirmed_chain_receipt: None,
                confirmed_transfer_receipt: None,
                provision_receipt: Some(crate::evolution::ProvisionReceipt {
                    child_id: "bounded-clock-child".to_owned(),
                    child_nature_fingerprint: child_nature.fingerprint().unwrap(),
                    manifest_sha256: "b".repeat(64),
                }),
            })
            .unwrap();
        assert!(
            runtime
                .lineage
                .node("bounded-clock-child")
                .unwrap()
                .spawned_at_ms
                < executor_time
        );

        let future_nature = runtime
            .lineage
            .plan_child_nature(&runtime.local_tentacle_id)
            .unwrap();
        let future = runtime
            .lifecycle
            .enqueue_spawn(
                now_ms,
                runtime.local_tentacle_id.clone(),
                "unbounded-clock-child".to_owned(),
                "c".repeat(64),
                future_nature.clone(),
                "evolution-runtime".to_owned(),
                "d".repeat(64),
            )
            .unwrap()
            .clone();
        let error = runtime
            .ack_lifecycle_action(LifecycleReceipt {
                action_id: future.action_id.clone(),
                completed_at_ms: now_ms
                    .saturating_add(LIFECYCLE_RECEIPT_CLOCK_SKEW_MS)
                    .saturating_add(1_000),
                status: LifecycleReceiptStatus::Succeeded,
                external_reference: None,
                detail: None,
                confirmed_chain_receipt: None,
                confirmed_transfer_receipt: None,
                provision_receipt: Some(crate::evolution::ProvisionReceipt {
                    child_id: "unbounded-clock-child".to_owned(),
                    child_nature_fingerprint: future_nature.fingerprint().unwrap(),
                    manifest_sha256: "e".repeat(64),
                }),
            })
            .unwrap_err();
        assert!(error.to_string().contains("bounded local clock skew"));
        assert!(runtime.lifecycle.receipt(&future.action_id).is_none());
        assert!(runtime.lineage.node("unbounded-clock-child").is_none());
    }

    #[test]
    fn kill_blocks_and_emits_durable_shutdown_while_local_force_can_recover() {
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
        assert!(response.contains("DURABLE SHUTDOWN ACTION"));
        assert!(!runtime.permits_normal_operation());
        let blocked = runtime
            .handle_operator_message(OPERATOR, "message-after-kill", "/spawn forbidden-child")
            .unwrap()
            .unwrap();
        assert!(blocked.contains("OPERATOR EFFECTS ARE CLOSED"));
        assert!(!runtime.lifecycle.intents.values().any(|intent| {
            matches!(
                &intent.action,
                LifecycleAction::Spawn { child_id, .. } if child_id == "forbidden-child"
            )
        }));
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
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
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

        let resumed = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
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
        let mut runtime =
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
        runtime.history_catalog.insert(&record).unwrap();
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
            runtime.history_catalog.insert(&record).unwrap();

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
        assert!(runtime.node_economic_refresh_is_deferred());
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
        assert!(!runtime.node_economic_refresh_is_deferred());
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
    fn public_turn_rechecks_required_node_economics_after_period_rollover() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let now = now_unix_seconds().unwrap();
        let day = EvaluationPeriod::Daily.duration_seconds();
        let current_start = aligned_period_start(now).unwrap();
        let closed_start = current_start - day;
        let observed_at = u64::try_from(current_start - 1).unwrap();
        let snapshot = TokenEconomicSnapshot {
            balance_basis_points: 10_000,
            stake_basis_points: DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS,
            reward_basis_points: 0,
            trustworthy: true,
        };
        let mut runtime = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                skip_awakening: true,
                require_node_economics: true,
                node_economics_ttl_seconds: u64::try_from(day * 2).unwrap(),
                initial_node_economics: Some((
                    snapshot,
                    EconomicObservationProvenance::base(
                        [1; 20],
                        EconomicHolderRole::TentacleTreasury,
                        [2; 20],
                        now,
                        Some(2),
                        [3; 32],
                    )
                    .unwrap(),
                )),
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();

        let nature = runtime.nature().clone();
        let mut closed = new_runtime_metrics(
            EvaluationPeriod::Daily,
            closed_start,
            &nature,
            runtime.ritual.epoch(),
        )
        .unwrap();
        closed
            .record_node_token_economic_observation(
                snapshot,
                runtime_token_policy(&nature, runtime.propagation_minimum_stake_basis_points)
                    .unwrap(),
                EconomicObservationProvenance::base(
                    [1; 20],
                    EconomicHolderRole::TentacleTreasury,
                    [2; 20],
                    observed_at,
                    Some(1),
                    [3; 32],
                )
                .unwrap(),
            )
            .unwrap();
        runtime.metrics = closed;
        runtime.scales_store.save_metrics(&runtime.metrics).unwrap();
        assert!(runtime.node_economics_is_current(now));

        let start = runtime.begin_public_turn().unwrap();
        assert!(matches!(
            start,
            PublicTurnStart::Gated(message)
                if message.contains("current Base UWU treasury economics are unavailable")
        ));
        assert!(runtime.active_public_turns.is_empty());
        assert!(runtime.metrics.token_economics.is_none());
        assert!(!runtime.node_economics_available);
        assert!(runtime.metrics.period_started_at_unix_seconds >= current_start);
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

    #[cfg(unix)]
    #[test]
    fn auto_spawn_policy_commits_only_after_durable_lifecycle_save() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside-lifecycle");
        fs::write(&outside, "do not replace").unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let before = runtime.auto_spawn_enabled();
        let lifecycle_path = root.path().join("state").join("lifecycle.json");
        fs::remove_file(&lifecycle_path).unwrap();
        symlink(&outside, &lifecycle_path).unwrap();

        assert!(runtime.set_auto_spawn_enabled(!before).is_err());
        assert_eq!(runtime.auto_spawn_enabled(), before);
        assert!(runtime.degraded);
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
    fn forced_reroll_rejects_pending_spawn_before_nature_or_metrics_change() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        let child_nature = runtime
            .lineage
            .plan_child_nature(&runtime.local_tentacle_id)
            .unwrap();
        runtime
            .lifecycle
            .enqueue_spawn(
                1,
                runtime.local_tentacle_id.clone(),
                "reroll-pending-child".to_owned(),
                "a".repeat(64),
                child_nature,
                "evolution-runtime".to_owned(),
                "b".repeat(64),
            )
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        let nature_before = fs::read(root.path().join("state").join("nature.json")).unwrap();
        let metrics_before = fs::read(root.path().join("state").join("metrics.json")).unwrap();
        drop(runtime);

        let error = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                reroll_nature: true,
                force: true,
                ..EvolutionStartupOptions::default()
            },
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("blocked while child provisioning is pending"));
        assert_eq!(
            fs::read(root.path().join("state").join("nature.json")).unwrap(),
            nature_before
        );
        assert_eq!(
            fs::read(root.path().join("state").join("metrics.json")).unwrap(),
            metrics_before
        );
    }

    #[test]
    fn lineage_spawn_receipts_must_resolve_to_their_exact_final_grants() {
        let mut nature = TentacleNature::random().unwrap();
        nature.sacred_ban = SacredBan::MemorySharing;
        let mut metrics = new_runtime_metrics(EvaluationPeriod::Daily, 0, &nature, 1).unwrap();
        metrics.record_conversation(10_000, true, Some(1));
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
        validate_lineage_spawn_authorizations(&valid, &catalog_with(std::slice::from_ref(&grant)))
            .unwrap();
        assert!(validate_lineage_spawn_authorizations(&valid, &catalog_with(&[])).is_err());

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
            validate_lineage_spawn_authorizations(
                &wrong_parent,
                &catalog_with(std::slice::from_ref(&grant))
            )
            .is_err()
        );

        let mut later = Lineage::new("parent", nature, 0).unwrap();
        let later_spawn_at = u64::try_from(
            grant.metrics.period_ends_at_unix_seconds + grant.metrics.period.duration_seconds(),
        )
        .unwrap()
            * 1_000;
        later
            .spawn_child(
                "parent",
                "parent",
                "later-child",
                later_spawn_at,
                authorization,
            )
            .unwrap();
        validate_lineage_spawn_authorizations(&later, &catalog_with(std::slice::from_ref(&grant)))
            .unwrap();
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
    fn propagation_grants_have_no_volume_or_expiry_quota_and_require_exact_economic_policy() {
        let nature = propagation_test_nature();
        let mut low_sample = new_runtime_metrics(EvaluationPeriod::Daily, 0, &nature, 1).unwrap();
        low_sample.record_conversation(1_000, true, Some(1));
        let judgment = low_sample
            .evaluate(&nature, low_sample.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(judgment.outcome, JudgmentOutcome::PropagationRights);
        assert!(judgment.propagation_evidence.eligible);
        let record = EvolutionHistoryRecord::new(&nature, low_sample, judgment).unwrap();
        let current = new_runtime_metrics(
            EvaluationPeriod::Daily,
            record.metrics.period_ends_at_unix_seconds,
            &nature,
            1,
        )
        .unwrap();
        assert!(
            is_accepted_propagation_grant_with_stake(
                &record,
                &current,
                &nature,
                1,
                current.period_started_at_unix_seconds + 1,
                0,
            )
            .unwrap()
        );

        let mut eligible = new_runtime_metrics(EvaluationPeriod::Daily, 0, &nature, 1).unwrap();
        eligible.record_conversation(10_000, true, Some(1));
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
            is_accepted_propagation_grant_with_stake(
                &record,
                &current,
                &nature,
                1,
                current.period_started_at_unix_seconds + 1,
                0,
            )
            .unwrap()
        );

        let mut token_enabled =
            new_runtime_metrics(EvaluationPeriod::Daily, 0, &nature, 1).unwrap();
        let targets = JudgmentPolicy::for_period(EvaluationPeriod::Daily).targets;
        token_enabled.record_conversation(
            targets.average_conversation_depth,
            true,
            Some(targets.response_time_full_credit_ms),
        );
        token_enabled
            .record_node_token_economic_observation(
                TokenEconomicSnapshot {
                    balance_basis_points: 10_000,
                    stake_basis_points: 10_000,
                    reward_basis_points: 0,
                    trustworthy: true,
                },
                runtime_token_policy(&nature, DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS).unwrap(),
                EconomicObservationProvenance::base(
                    [1; 20],
                    EconomicHolderRole::TentacleTreasury,
                    [2; 20],
                    1,
                    None,
                    [3; 32],
                )
                .unwrap(),
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
                growth: true,
                wealth: true,
                influence: true,
            }
        );
        let mut token_current = new_runtime_metrics(
            EvaluationPeriod::Daily,
            token_record.metrics.period_ends_at_unix_seconds,
            &nature,
            1,
        )
        .unwrap();
        let grant_economics = token_record.metrics.token_economics.unwrap();
        let grant_provenance = grant_economics.provenance.unwrap();
        let current_observed_at =
            u64::try_from(token_current.period_started_at_unix_seconds + 1).unwrap();
        token_current
            .record_node_token_economic_observation(
                grant_economics.snapshot,
                grant_economics.policy,
                EconomicObservationProvenance::base(
                    grant_provenance.holder_address,
                    grant_provenance.holder_role,
                    grant_provenance.token_contract,
                    current_observed_at,
                    Some(2),
                    grant_provenance.configuration_identity,
                )
                .unwrap(),
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
        let mut withdrawn_current = token_current.clone();
        withdrawn_current
            .record_node_token_economic_observation(
                TokenEconomicSnapshot {
                    stake_basis_points: 0,
                    ..grant_economics.snapshot
                },
                grant_economics.policy,
                EconomicObservationProvenance::base(
                    grant_provenance.holder_address,
                    grant_provenance.holder_role,
                    grant_provenance.token_contract,
                    current_observed_at.saturating_add(1),
                    Some(3),
                    grant_provenance.configuration_identity,
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            withdrawn_current.scored_scale_availability,
            ScoredScaleAvailability {
                engagement: true,
                growth: true,
                wealth: true,
                influence: true,
            }
        );
        assert!(
            !is_accepted_propagation_grant(
                &token_record,
                &withdrawn_current,
                &nature,
                1,
                withdrawn_current.period_started_at_unix_seconds + 2,
            )
            .unwrap()
        );

        let mut arbitrary_availability_metrics = token_record.metrics.clone();
        arbitrary_availability_metrics
            .scored_scale_availability
            .growth = false;
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

        let mut arbitrary_policy =
            runtime_token_policy(&nature, DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS).unwrap();
        arbitrary_policy.engagement_sensitivity_basis_points =
            if arbitrary_policy.engagement_sensitivity_basis_points == 10_000 {
                9_999
            } else {
                arbitrary_policy.engagement_sensitivity_basis_points + 1
            };
        let mut arbitrary_policy_metrics = token_record.metrics.clone();
        arbitrary_policy_metrics
            .record_node_token_economic_observation(
                token_record.metrics.token_economics.unwrap().snapshot,
                arbitrary_policy,
                token_record
                    .metrics
                    .token_economics
                    .unwrap()
                    .provenance
                    .unwrap(),
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
            is_accepted_propagation_grant_with_stake(
                &record,
                &stale_cycle,
                &nature,
                1,
                stale_cycle.period_started_at_unix_seconds + 1,
                0,
            )
            .unwrap()
        );
    }

    #[test]
    fn spawn_lineage_and_growth_commit_only_after_matching_provision_success() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let before_growth = runtime.metrics.growth.children_spawned;
        let at_ms = now_unix_seconds().unwrap().saturating_mul(1_000);

        let failed_nature = runtime
            .lineage
            .plan_child_nature(&runtime.local_tentacle_id)
            .unwrap();
        let failed = runtime
            .lifecycle
            .enqueue_spawn(
                at_ms,
                runtime.local_tentacle_id.clone(),
                "failed-child".to_owned(),
                "a".repeat(64),
                failed_nature,
                "evolution-runtime".to_owned(),
                "b".repeat(64),
            )
            .unwrap()
            .clone();
        runtime
            .ack_lifecycle_action(LifecycleReceipt {
                action_id: failed.action_id,
                completed_at_ms: at_ms.saturating_add(1),
                status: LifecycleReceiptStatus::Failed,
                external_reference: None,
                detail: Some("provisioner failed".to_owned()),
                confirmed_chain_receipt: None,
                confirmed_transfer_receipt: None,
                provision_receipt: None,
            })
            .unwrap();
        assert!(runtime.lineage.node("failed-child").is_none());
        assert_eq!(runtime.metrics.growth.children_spawned, before_growth);

        let child_nature = runtime
            .lineage
            .plan_child_nature(&runtime.local_tentacle_id)
            .unwrap();
        let successful = runtime
            .lifecycle
            .enqueue_spawn(
                at_ms,
                runtime.local_tentacle_id.clone(),
                "provisioned-child".to_owned(),
                "c".repeat(64),
                child_nature.clone(),
                "evolution-runtime".to_owned(),
                "d".repeat(64),
            )
            .unwrap()
            .clone();
        runtime
            .ack_lifecycle_action(LifecycleReceipt {
                action_id: successful.action_id,
                completed_at_ms: at_ms.saturating_add(2),
                status: LifecycleReceiptStatus::Succeeded,
                external_reference: None,
                detail: None,
                confirmed_chain_receipt: None,
                confirmed_transfer_receipt: None,
                provision_receipt: Some(crate::evolution::ProvisionReceipt {
                    child_id: "provisioned-child".to_owned(),
                    child_nature_fingerprint: child_nature.fingerprint().unwrap(),
                    manifest_sha256: "e".repeat(64),
                }),
            })
            .unwrap();
        assert_eq!(
            runtime.lineage.node("provisioned-child").unwrap().nature,
            child_nature
        );
        assert_eq!(
            runtime.metrics.growth.children_spawned,
            before_growth.saturating_add(1)
        );
    }

    #[test]
    fn restart_recovers_lineage_ahead_spawn_growth_exactly_once() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        let nature = runtime.nature().clone();
        let prior_period = runtime
            .metrics
            .period_started_at_unix_seconds
            .saturating_sub(EvaluationPeriod::Daily.duration_seconds());
        let mut grant_metrics = new_runtime_metrics(
            EvaluationPeriod::Daily,
            prior_period,
            &nature,
            runtime.ritual.epoch(),
        )
        .unwrap();
        grant_metrics.record_conversation(1_000, true, Some(1));
        let judgment = grant_metrics
            .evaluate(&nature, grant_metrics.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(judgment.outcome, JudgmentOutcome::PropagationRights);
        let grant = EvolutionHistoryRecord::new(&nature, grant_metrics, judgment).unwrap();
        runtime.scales_store.append_history(&grant).unwrap();
        runtime.history_catalog.insert(&grant).unwrap();

        let completed_at_ms = now_unix_seconds().unwrap().saturating_mul(1_000);
        let child_nature = runtime
            .lineage
            .plan_child_nature(&runtime.local_tentacle_id)
            .unwrap();
        let intent = runtime
            .lifecycle
            .enqueue_spawn(
                completed_at_ms,
                runtime.local_tentacle_id.clone(),
                "lineage-ahead-child".to_owned(),
                grant.judgment_id.clone(),
                child_nature.clone(),
                "evolution-runtime".to_owned(),
                "f".repeat(64),
            )
            .unwrap()
            .clone();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        let receipt = LifecycleReceipt {
            action_id: intent.action_id.clone(),
            completed_at_ms,
            status: LifecycleReceiptStatus::Succeeded,
            external_reference: None,
            detail: None,
            confirmed_chain_receipt: None,
            confirmed_transfer_receipt: None,
            provision_receipt: Some(crate::evolution::ProvisionReceipt {
                child_id: "lineage-ahead-child".to_owned(),
                child_nature_fingerprint: child_nature.fingerprint().unwrap(),
                manifest_sha256: "e".repeat(64),
            }),
        };
        runtime.lifecycle.acknowledge_action(receipt).unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        runtime
            .lineage
            .record_provisioned_child(
                &runtime.local_tentacle_id,
                &runtime.local_tentacle_id,
                "lineage-ahead-child",
                child_nature,
                completed_at_ms,
                SpawnAuthorization {
                    judgment_id: grant.judgment_id,
                    operator_id: "evolution-runtime".to_owned(),
                    event_id_sha256: "f".repeat(64),
                },
            )
            .unwrap();
        runtime.lineage_store.save(&runtime.lineage).unwrap();
        assert_eq!(runtime.metrics.growth.children_spawned, 0);
        assert!(runtime.lifecycle.spawn_projections.is_empty());
        drop(runtime);

        let resumed = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        assert_eq!(resumed.metrics.growth.children_spawned, 1);
        assert_eq!(resumed.lifecycle.spawn_projections.len(), 1);
        drop(resumed);
        let resumed_again = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        assert_eq!(resumed_again.metrics.growth.children_spawned, 1);
        assert_eq!(resumed_again.lifecycle.spawn_projections.len(), 1);
    }

    #[test]
    fn completed_shutdown_restart_does_not_roll_or_mutate_state() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        let now = now_unix_seconds().unwrap();
        runtime.metrics = new_runtime_metrics(
            EvaluationPeriod::Daily,
            aligned_period_start(now).unwrap() - EvaluationPeriod::Daily.duration_seconds(),
            runtime.nature(),
            runtime.ritual.epoch(),
        )
        .unwrap();
        runtime.metrics.record_conversation(50, true, Some(1));
        runtime.scales_store.save_metrics(&runtime.metrics).unwrap();

        let completed_at_ms = now.saturating_mul(1_000);
        let judgment_id = "9".repeat(64);
        runtime
            .lifecycle
            .schedule_death(&judgment_id, completed_at_ms, 0, None)
            .unwrap();
        runtime
            .lifecycle
            .reconcile_expired_death(completed_at_ms, None)
            .unwrap();
        let shutdown = runtime.lifecycle.next_due_action().unwrap().clone();
        runtime
            .lifecycle
            .acknowledge_action(LifecycleReceipt {
                action_id: shutdown.action_id,
                completed_at_ms,
                status: LifecycleReceiptStatus::Succeeded,
                external_reference: Some("native-transport-stopped".to_owned()),
                detail: None,
                confirmed_chain_receipt: None,
                confirmed_transfer_receipt: None,
                provision_receipt: None,
            })
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        let state_dir = root.path().join("state");
        let tracked = [
            "metrics.json",
            "evolution_history.jsonl",
            "lifecycle.json",
            "lineage.json",
        ];
        let before = tracked
            .iter()
            .map(|name| {
                let path = state_dir.join(name);
                (name, fs::read(path).unwrap_or_default())
            })
            .collect::<Vec<_>>();
        drop(runtime);

        let resumed = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                auto_spawn: Some(false),
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();
        assert!(resumed.is_shutdown_complete());
        drop(resumed);
        for (name, expected) in before {
            assert_eq!(fs::read(state_dir.join(name)).unwrap_or_default(), expected);
        }
    }

    #[test]
    fn restart_before_spawn_receipt_reuses_the_pending_child_plan() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        let nature = runtime.nature().clone();
        let mut grant_metrics =
            new_runtime_metrics(EvaluationPeriod::Daily, 0, &nature, runtime.ritual.epoch())
                .unwrap();
        grant_metrics.record_conversation(1_000, true, Some(1));
        let judgment = grant_metrics
            .evaluate(&nature, grant_metrics.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(judgment.outcome, JudgmentOutcome::PropagationRights);
        let grant = EvolutionHistoryRecord::new(&nature, grant_metrics, judgment).unwrap();
        runtime.scales_store.append_history(&grant).unwrap();
        runtime.history_catalog.insert(&grant).unwrap();
        let at_ms = u64::try_from(grant.judgment.evaluated_at_unix_seconds)
            .unwrap()
            .saturating_mul(1_000);
        runtime.auto_spawn_from_grant(&grant, at_ms).unwrap();
        let before = runtime
            .lifecycle
            .intents
            .values()
            .filter(|intent| matches!(intent.action, LifecycleAction::Spawn { .. }))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(before.len(), 1);
        drop(runtime);

        let mut resumed = open_confirmed_with_zero_stake(root.path(), workspace.path()).unwrap();
        resumed.auto_spawn_from_grant(&grant, at_ms).unwrap();
        let after = resumed
            .lifecycle
            .intents
            .values()
            .filter(|intent| matches!(intent.action, LifecycleAction::Spawn { .. }))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(after, before);
    }

    #[test]
    fn current_stake_withdrawal_suppresses_pending_spawn_across_restart() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let now = now_unix_seconds().unwrap();
        let funded = TokenEconomicSnapshot {
            balance_basis_points: 10_000,
            stake_basis_points: 10_000,
            reward_basis_points: 10_000,
            trustworthy: true,
        };
        let provenance = EconomicObservationProvenance::base(
            [1; 20],
            EconomicHolderRole::TentacleTreasury,
            [2; 20],
            now,
            Some(1),
            [3; 32],
        )
        .unwrap();
        let mut runtime = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                skip_awakening: true,
                auto_spawn: Some(false),
                initial_node_economics: Some((funded, provenance)),
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();
        let nature = runtime.nature().clone();
        let prior_period = runtime
            .metrics
            .period_started_at_unix_seconds
            .saturating_sub(EvaluationPeriod::Daily.duration_seconds());
        let mut grant_metrics = new_runtime_metrics(
            EvaluationPeriod::Daily,
            prior_period,
            &nature,
            runtime.ritual.epoch(),
        )
        .unwrap();
        let targets = JudgmentPolicy::for_period(EvaluationPeriod::Daily).targets;
        grant_metrics.record_conversation(
            targets.average_conversation_depth,
            true,
            Some(targets.response_time_full_credit_ms),
        );
        grant_metrics
            .record_node_token_economic_observation(
                funded,
                runtime_token_policy(&nature, runtime.propagation_minimum_stake_basis_points)
                    .unwrap(),
                EconomicObservationProvenance::base(
                    provenance.holder_address,
                    provenance.holder_role,
                    provenance.token_contract,
                    u64::try_from(prior_period).unwrap().saturating_add(1),
                    Some(1),
                    provenance.configuration_identity,
                )
                .unwrap(),
            )
            .unwrap();
        grant_metrics
            .record_economic_result(targets.revenue_micro_units, targets.efficiency_basis_points)
            .unwrap();
        grant_metrics.record_growth(
            targets.children_spawned,
            targets.acolytes_recruited,
            targets.network_contribution_points,
        );
        grant_metrics.record_influence(
            targets.governance_participation,
            targets.sibling_influence_points,
        );
        let judgment = grant_metrics
            .evaluate(&nature, grant_metrics.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(judgment.outcome, JudgmentOutcome::PropagationRights);
        let grant = EvolutionHistoryRecord::new(&nature, grant_metrics, judgment).unwrap();
        runtime.scales_store.append_history(&grant).unwrap();
        runtime.history_catalog.insert(&grant).unwrap();
        let child_nature = runtime
            .lineage
            .plan_child_nature(&runtime.local_tentacle_id)
            .unwrap();
        runtime
            .lifecycle
            .enqueue_spawn(
                now.saturating_mul(1_000),
                runtime.local_tentacle_id.clone(),
                "stake-bound-child".to_owned(),
                grant.judgment_id.clone(),
                child_nature,
                "evolution-runtime".to_owned(),
                "d".repeat(64),
            )
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        assert!(matches!(
            runtime
                .next_due_lifecycle_action(now)
                .unwrap()
                .unwrap()
                .action,
            LifecycleAction::Spawn { .. }
        ));

        let withdrawn = TokenEconomicSnapshot {
            stake_basis_points: 0,
            ..funded
        };
        runtime
            .record_node_economic_observation(withdrawn, provenance)
            .unwrap();
        assert!(runtime.next_due_lifecycle_action(now).unwrap().is_none());
        drop(runtime);

        let mut resumed = EvolutionRuntime::open(
            root.path(),
            workspace.path(),
            EvolutionStartupOptions {
                skip_awakening: true,
                auto_spawn: Some(false),
                initial_node_economics: Some((withdrawn, provenance)),
                ..EvolutionStartupOptions::default()
            },
        )
        .unwrap();
        assert!(resumed.next_due_lifecycle_action(now).unwrap().is_none());
    }

    #[test]
    fn dormancy_stays_online_while_legacy_death_can_reconcile_a_top_up() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let now = now_unix_seconds().unwrap();
        let nature = runtime.nature().clone();
        let prior_period = runtime
            .metrics
            .period_started_at_unix_seconds
            .saturating_sub(EvaluationPeriod::Daily.duration_seconds());
        let policy =
            runtime_token_policy(&nature, runtime.propagation_minimum_stake_basis_points).unwrap();
        let mut death_metrics = new_runtime_metrics(
            EvaluationPeriod::Daily,
            prior_period,
            &nature,
            runtime.ritual.epoch(),
        )
        .unwrap();
        death_metrics
            .record_node_token_economic_observation(
                TokenEconomicSnapshot {
                    balance_basis_points: 0,
                    stake_basis_points: runtime.propagation_minimum_stake_basis_points,
                    reward_basis_points: 0,
                    trustworthy: true,
                },
                policy,
                EconomicObservationProvenance::base(
                    [1; 20],
                    EconomicHolderRole::TentacleTreasury,
                    [2; 20],
                    u64::try_from(prior_period).unwrap().saturating_add(1),
                    Some(1),
                    [3; 32],
                )
                .unwrap(),
            )
            .unwrap();
        let mut judgment = death_metrics
            .evaluate(&nature, death_metrics.period_ends_at_unix_seconds)
            .unwrap();
        assert_eq!(judgment.outcome, JudgmentOutcome::Dormant);
        let dormant =
            EvolutionHistoryRecord::new(&nature, death_metrics.clone(), judgment.clone()).unwrap();
        runtime
            .apply_final_judgment_lifecycle(&dormant, now.saturating_mul(1_000))
            .unwrap();
        runtime.last_final_judgment = Some(dormant);
        assert!(runtime.permits_normal_operation());
        assert!(runtime.is_dormant());
        assert!(runtime.take_operator_dormancy_plea().is_some());
        assert!(runtime.take_public_dormancy_plea().is_some());
        for _ in 0..4 {
            assert!(runtime.take_public_dormancy_plea().is_none());
        }
        assert!(runtime.take_public_dormancy_plea().is_some());
        assert!(!runtime.lifecycle.death_pending());
        assert!(runtime.lifecycle.intents.is_empty());
        // Exercise compatibility with a legacy hash-bound Death record.
        judgment.outcome = JudgmentOutcome::Death;
        let death = EvolutionHistoryRecord::new(&nature, death_metrics, judgment).unwrap();
        runtime.scales_store.append_history(&death).unwrap();
        runtime.history_catalog.insert(&death).unwrap();
        runtime
            .lifecycle
            .schedule_death(
                &death.judgment_id,
                now.saturating_mul(1_000),
                DEATH_GRACE_PERIOD_MS,
                None,
            )
            .unwrap();
        runtime.lifecycle_store.save(&runtime.lifecycle).unwrap();
        assert!(
            !runtime.lifecycle.intents.values().any(|intent| {
                matches!(intent.action, LifecycleAction::SpendForSurvival { .. })
            })
        );

        runtime
            .record_node_economic_observation(
                TokenEconomicSnapshot {
                    balance_basis_points: 10_000,
                    stake_basis_points: runtime.propagation_minimum_stake_basis_points,
                    reward_basis_points: 0,
                    trustworthy: true,
                },
                EconomicObservationProvenance::base(
                    [1; 20],
                    EconomicHolderRole::TentacleTreasury,
                    [2; 20],
                    now,
                    Some(2),
                    [3; 32],
                )
                .unwrap(),
            )
            .unwrap();
        let spend = runtime
            .lifecycle
            .intents
            .values()
            .find(|intent| {
                matches!(
                    &intent.action,
                    LifecycleAction::SpendForSurvival { judgment_id, .. }
                        if judgment_id == &death.judgment_id
                )
            })
            .unwrap();
        assert!(matches!(
            &spend.action,
            LifecycleAction::SpendForSurvival { exact_amount, .. }
                if exact_amount.raw_amount
                    == exact_raw_token_amount(
                        runtime.survival_total_supply_whole,
                        runtime.survival_token_decimals,
                        exact_amount.basis_points,
                    )
                    .unwrap()
        ));
        let first_action_id = spend.action_id.clone();
        // A fresher observation with the same exact economic binding must not replace an
        // unreceipted burn with a new idempotency key.
        runtime
            .record_node_economic_observation(
                TokenEconomicSnapshot {
                    balance_basis_points: 10_000,
                    stake_basis_points: runtime.propagation_minimum_stake_basis_points,
                    reward_basis_points: 0,
                    trustworthy: true,
                },
                EconomicObservationProvenance::base(
                    [1; 20],
                    EconomicHolderRole::TentacleTreasury,
                    [2; 20],
                    now.saturating_add(1),
                    Some(3),
                    [3; 32],
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            runtime
                .lifecycle
                .intents
                .values()
                .filter(|intent| {
                    runtime.lifecycle.receipt(&intent.action_id).is_none()
                        && matches!(
                            &intent.action,
                            LifecycleAction::SpendForSurvival { judgment_id, .. }
                                if judgment_id == &death.judgment_id
                        )
                })
                .map(|intent| intent.action_id.as_str())
                .collect::<Vec<_>>(),
            vec![first_action_id.as_str()]
        );

        // Underfunding hides, rather than replaces, the ambiguous unreceipted burn. Funding
        // recovery reactivates the same action ID.
        runtime
            .record_node_economic_observation(
                TokenEconomicSnapshot {
                    balance_basis_points: 0,
                    stake_basis_points: runtime.propagation_minimum_stake_basis_points,
                    reward_basis_points: 0,
                    trustworthy: true,
                },
                EconomicObservationProvenance::base(
                    [1; 20],
                    EconomicHolderRole::TentacleTreasury,
                    [2; 20],
                    now.saturating_add(2),
                    Some(4),
                    [3; 32],
                )
                .unwrap(),
            )
            .unwrap();
        assert!(
            runtime
                .lifecycle
                .canceled_action_ids
                .contains(&first_action_id)
        );
        runtime
            .record_node_economic_observation(
                TokenEconomicSnapshot {
                    balance_basis_points: 10_000,
                    stake_basis_points: runtime.propagation_minimum_stake_basis_points,
                    reward_basis_points: 0,
                    trustworthy: true,
                },
                EconomicObservationProvenance::base(
                    [1; 20],
                    EconomicHolderRole::TentacleTreasury,
                    [2; 20],
                    now.saturating_add(3),
                    Some(5),
                    [3; 32],
                )
                .unwrap(),
            )
            .unwrap();
        assert!(
            !runtime
                .lifecycle
                .canceled_action_ids
                .contains(&first_action_id)
        );
        runtime
            .ack_lifecycle_action(LifecycleReceipt {
                action_id: first_action_id.clone(),
                completed_at_ms: now.saturating_add(3).saturating_mul(1_000),
                status: LifecycleReceiptStatus::Failed,
                external_reference: None,
                detail: Some("burn transaction was not accepted".to_owned()),
                confirmed_chain_receipt: None,
                confirmed_transfer_receipt: None,
                provision_receipt: None,
            })
            .unwrap();
        runtime
            .record_node_economic_observation(
                TokenEconomicSnapshot {
                    balance_basis_points: 10_000,
                    stake_basis_points: runtime.propagation_minimum_stake_basis_points,
                    reward_basis_points: 0,
                    trustworthy: true,
                },
                EconomicObservationProvenance::base(
                    [1; 20],
                    EconomicHolderRole::TentacleTreasury,
                    [2; 20],
                    now.saturating_add(4),
                    Some(6),
                    [3; 32],
                )
                .unwrap(),
            )
            .unwrap();
        assert!(runtime.lifecycle.intents.values().any(|intent| {
            intent.action_id != first_action_id
                && runtime.lifecycle.receipt(&intent.action_id).is_none()
                && matches!(
                    &intent.action,
                    LifecycleAction::SpendForSurvival { judgment_id, .. }
                        if judgment_id == &death.judgment_id
                )
        }));
        let second = runtime
            .lifecycle
            .intents
            .values()
            .find(|intent| {
                intent.action_id != first_action_id
                    && runtime.lifecycle.receipt(&intent.action_id).is_none()
                    && matches!(intent.action, LifecycleAction::SpendForSurvival { .. })
            })
            .unwrap()
            .clone();
        let LifecycleAction::SpendForSurvival {
            chain_id,
            token_contract,
            treasury_address,
            burn_destination,
            configuration_identity,
            exact_amount,
            ..
        } = second.action
        else {
            unreachable!();
        };
        runtime
            .ack_lifecycle_action(LifecycleReceipt {
                action_id: second.action_id,
                completed_at_ms: now.saturating_add(5).saturating_mul(1_000),
                status: LifecycleReceiptStatus::Succeeded,
                external_reference: None,
                detail: None,
                confirmed_chain_receipt: Some(crate::evolution::ConfirmedChainReceipt {
                    chain_id,
                    transaction_hash: format!("0x{}", "9".repeat(64)),
                    block_number: 7,
                    block_timestamp_unix_seconds: now.saturating_add(5),
                    token_contract,
                    from_address: treasury_address,
                    burn_destination,
                    configuration_identity,
                    exact_amount,
                    operation: crate::evolution::TokenSpendOperation::Burn,
                }),
                confirmed_transfer_receipt: None,
                provision_receipt: None,
            })
            .unwrap();
        assert!(!runtime.node_economics_available);
        let snapshot = TokenEconomicSnapshot {
            balance_basis_points: 10_000,
            stake_basis_points: runtime.propagation_minimum_stake_basis_points,
            reward_basis_points: 0,
            trustworthy: true,
        };
        assert!(
            runtime
                .record_node_economic_observation(
                    snapshot,
                    EconomicObservationProvenance::base(
                        [1; 20],
                        EconomicHolderRole::TentacleTreasury,
                        [2; 20],
                        now.saturating_add(4),
                        Some(6),
                        [3; 32],
                    )
                    .unwrap(),
                )
                .unwrap_err()
                .to_string()
                .contains("after the latest confirmed token transaction")
        );
        runtime
            .record_node_economic_observation(
                snapshot,
                EconomicObservationProvenance::base(
                    [1; 20],
                    EconomicHolderRole::TentacleTreasury,
                    [2; 20],
                    now.saturating_add(6),
                    Some(8),
                    [3; 32],
                )
                .unwrap(),
            )
            .unwrap();
        assert!(runtime.node_economics_available);
    }

    #[test]
    fn authenticated_venice_key_reward_is_durable_and_requires_exact_transfer_receipt() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let now = now_unix_seconds().unwrap();
        let provenance = EconomicObservationProvenance::base(
            [1; 20],
            EconomicHolderRole::TentacleTreasury,
            [2; 20],
            now,
            Some(10),
            [3; 32],
        )
        .unwrap();
        runtime
            .record_node_economic_observation(
                TokenEconomicSnapshot {
                    balance_basis_points: 1,
                    stake_basis_points: 0,
                    reward_basis_points: 0,
                    trustworthy: true,
                },
                provenance,
            )
            .unwrap();

        let intent = runtime
            .enqueue_venice_key_reward("xmtp-message-1", [4; 20], 1, now)
            .unwrap()
            .unwrap();
        let LifecycleAction::RewardVeniceKey {
            chain_id,
            token_contract,
            treasury_address,
            acolyte_address,
            configuration_identity,
            exact_amount,
            ..
        } = intent.action.clone()
        else {
            panic!("expected Venice-key reward action");
        };
        assert_eq!(exact_amount.raw_amount, "1000000000000000000");
        assert_eq!(
            runtime
                .next_due_lifecycle_action(now)
                .unwrap()
                .unwrap()
                .action_id,
            intent.action_id
        );

        let mut transfer = crate::evolution::ConfirmedTransferReceipt {
            chain_id,
            transaction_hash: format!("0x{}", "a".repeat(64)),
            block_number: 11,
            block_timestamp_unix_seconds: now,
            token_contract,
            from_address: treasury_address,
            to_address: [5; 20],
            configuration_identity,
            exact_amount,
        };
        assert!(
            runtime
                .ack_lifecycle_action(LifecycleReceipt {
                    action_id: intent.action_id.clone(),
                    completed_at_ms: now.saturating_mul(1_000),
                    status: LifecycleReceiptStatus::Succeeded,
                    external_reference: None,
                    detail: None,
                    confirmed_chain_receipt: None,
                    confirmed_transfer_receipt: Some(transfer.clone()),
                    provision_receipt: None,
                })
                .is_err()
        );

        transfer.to_address = acolyte_address;
        assert!(
            runtime
                .ack_lifecycle_action(LifecycleReceipt {
                    action_id: intent.action_id.clone(),
                    completed_at_ms: now.saturating_mul(1_000),
                    status: LifecycleReceiptStatus::Succeeded,
                    external_reference: None,
                    detail: None,
                    confirmed_chain_receipt: None,
                    confirmed_transfer_receipt: Some(transfer),
                    provision_receipt: None,
                })
                .unwrap()
        );
        assert!(!runtime.node_economics_available);
        drop(runtime);

        let resumed =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        assert!(resumed.lifecycle.receipt(&intent.action_id).is_some());
    }

    #[test]
    fn voluntary_information_reward_is_durable_and_requires_exact_transfer_receipt() {
        let root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut runtime =
            EvolutionRuntime::open_confirmed_for_test(root.path(), workspace.path()).unwrap();
        let now = now_unix_seconds().unwrap();
        runtime
            .record_node_economic_observation(
                TokenEconomicSnapshot {
                    balance_basis_points: 1,
                    stake_basis_points: 0,
                    reward_basis_points: 0,
                    trustworthy: true,
                },
                EconomicObservationProvenance::base(
                    [1; 20],
                    EconomicHolderRole::TentacleTreasury,
                    [2; 20],
                    now,
                    Some(10),
                    [3; 32],
                )
                .unwrap(),
            )
            .unwrap();

        let intent = runtime
            .enqueue_acolyte_contribution_reward(
                "xmtp-profile-answer-1",
                AcolyteContributionKind::Hopes,
                [4; 20],
                8,
                80,
                now,
            )
            .unwrap()
            .unwrap();
        let LifecycleAction::RewardAcolyteContribution {
            chain_id,
            token_contract,
            treasury_address,
            acolyte_address,
            configuration_identity,
            exact_amount,
            information_hunger_basis_points,
            ..
        } = intent.action.clone()
        else {
            panic!("expected contribution reward action");
        };
        assert_eq!(information_hunger_basis_points, 80);
        assert_eq!(exact_amount.whole_tokens, 8);
        assert_eq!(exact_amount.raw_amount, "8000000000000000000");

        let transfer = crate::evolution::ConfirmedTransferReceipt {
            chain_id,
            transaction_hash: format!("0x{}", "b".repeat(64)),
            block_number: 11,
            block_timestamp_unix_seconds: now,
            token_contract,
            from_address: treasury_address,
            to_address: acolyte_address,
            configuration_identity,
            exact_amount,
        };
        assert!(
            runtime
                .ack_lifecycle_action(LifecycleReceipt {
                    action_id: intent.action_id.clone(),
                    completed_at_ms: now.saturating_mul(1_000),
                    status: LifecycleReceiptStatus::Succeeded,
                    external_reference: None,
                    detail: None,
                    confirmed_chain_receipt: None,
                    confirmed_transfer_receipt: Some(transfer),
                    provision_receipt: None,
                })
                .unwrap()
        );
        assert!(!runtime.node_economics_available);
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
