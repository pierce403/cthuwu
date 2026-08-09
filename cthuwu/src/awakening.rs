use crate::{
    personality::{
        NatureTrait, StateSigner, TentacleNature, assert_owner_only, open_read_no_follow,
        parent_directory, prepare_private_parent, reject_unsafe_target, validate_nature_id,
    },
    storage::{restrict_file, sync_directory},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::{Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
};
use tempfile::NamedTempFile;

const AWAKENING_LOG_VERSION: u32 = 2;
const AWAKENING_LOG_ALGORITHM: &str = "hmac-sha256";
const AWAKENING_LOG_SIGNATURE_DOMAIN: &str = "cthuwu-awakening-log-v2";
const AWAKENING_LOG_HEADER: &str = "# Cthuwu awakening log\n\n\
    Signed, append-only operator actions. Raw message bodies are never stored.\n\n";
const MAX_AWAKENING_RESPONSE_BYTES: usize = 256;
const MAX_AWAKENING_LOG_BYTES: u64 = 1024 * 1024;
const MAX_EVENT_ID_BYTES: usize = 1024;
const OPERATOR_INBOX_ID_BYTES: usize = 64;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum AwakeningAction {
    Yes,
    Adjust {
        nature_trait: NatureTrait,
        delta: i16,
    },
    Reroll,
    Kill,
}

impl AwakeningAction {
    /// Parses only the four ritual forms. Callers must authenticate the operator before invoking
    /// this parser; parsing text does not confer authority.
    pub fn parse(input: &str) -> Result<Self> {
        if input.is_empty()
            || input.len() > MAX_AWAKENING_RESPONSE_BYTES
            || !input.is_ascii()
            || input
                .chars()
                .any(|character| character.is_control() && !character.is_ascii_whitespace())
        {
            bail!("invalid awakening response");
        }
        let parts: Vec<&str> = input.split_ascii_whitespace().collect();
        match parts.as_slice() {
            [command] if command.eq_ignore_ascii_case("YES") => Ok(Self::Yes),
            [command] if command.eq_ignore_ascii_case("REROLL") => Ok(Self::Reroll),
            [command] if command.eq_ignore_ascii_case("KILL") => Ok(Self::Kill),
            [command, nature_trait, delta] if command.eq_ignore_ascii_case("ADJUST") => {
                let nature_trait = NatureTrait::from_str(nature_trait)?;
                let delta = delta
                    .parse::<i16>()
                    .map_err(|_| anyhow::anyhow!("awakening adjustment must be an integer"))?;
                if delta == 0 || !(-100..=100).contains(&delta) {
                    bail!("awakening adjustment must be between -100 and 100 and not zero");
                }
                Ok(Self::Adjust {
                    nature_trait,
                    delta,
                })
            }
            _ => bail!("expected YES, ADJUST <trait> <delta>, REROLL, or KILL"),
        }
    }

    pub fn normalized(&self) -> String {
        match self {
            Self::Yes => "YES".to_owned(),
            Self::Adjust {
                nature_trait,
                delta,
            } => format!("ADJUST {nature_trait} {delta:+}"),
            Self::Reroll => "REROLL".to_owned(),
            Self::Kill => "KILL".to_owned(),
        }
    }
}

impl FromStr for AwakeningAction {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceSource {
    AuthenticatedXmtp,
    LocalCli,
}

/// Bounded provenance prepared after operator authentication. Opaque event IDs are hashed before
/// they can enter the append-only log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AwakeningProvenance {
    operator_id: String,
    source: ProvenanceSource,
    event_id_sha256: String,
}

impl AwakeningProvenance {
    pub fn new(operator_id: &str, source: ProvenanceSource, opaque_event_id: &str) -> Result<Self> {
        let operator_id = normalize_actor_id(source, operator_id)?;
        let event_id = opaque_event_id.trim();
        if event_id.is_empty()
            || event_id.len() > MAX_EVENT_ID_BYTES
            || !event_id.bytes().all(|byte| byte.is_ascii_graphic())
        {
            bail!("invalid awakening provenance event ID");
        }
        Ok(Self {
            operator_id,
            source,
            event_id_sha256: encode_hex(&Sha256::digest(event_id.as_bytes())),
        })
    }

    pub fn authenticated_xmtp(operator_id: &str, message_id: &str) -> Result<Self> {
        Self::new(operator_id, ProvenanceSource::AuthenticatedXmtp, message_id)
    }

    pub fn local_cli(operator_id: &str, event_id: &str) -> Result<Self> {
        Self::new(operator_id, ProvenanceSource::LocalCli, event_id)
    }

    pub fn operator_id(&self) -> &str {
        &self.operator_id
    }

    pub const fn source(&self) -> ProvenanceSource {
        self.source
    }

    pub fn event_id_sha256(&self) -> &str {
        &self.event_id_sha256
    }

    fn validate(&self) -> Result<()> {
        if normalize_actor_id(self.source, &self.operator_id)? != self.operator_id {
            bail!("awakening operator identifier is not canonical");
        }
        validate_sha256(&self.event_id_sha256, "awakening provenance event hash")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AwakeningPhase {
    AwaitingConfirmation,
    Confirmed {
        timestamp_unix: u64,
        operator_id: String,
    },
    Killed {
        timestamp_unix: u64,
        operator_id: String,
    },
    SkippedForTesting {
        timestamp_unix: u64,
        operator_id: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AwakeningOutcome {
    AwaitingConfirmation,
    Confirmed,
    KillRequested,
    SkippedForTesting,
    AdjustedAfterConfirmation,
    ForcedRerollEpoch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AwakeningRecovery {
    pub ritual: AwakeningRitual,
    /// True when a missing Nature or the final entry's exact signed immediate predecessor must be
    /// replaced with `ritual.nature()`.
    pub nature_recovered_from_log: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RecordedAwakeningAction {
    Operator(AwakeningAction),
    SkipForTesting,
    PostConfirmationAdjust {
        nature_trait: NatureTrait,
        value: u8,
    },
    ForcedRerollEpoch,
}

impl RecordedAwakeningAction {
    fn normalized(&self) -> String {
        match self {
            Self::Operator(action) => action.normalized(),
            Self::SkipForTesting => "SKIP --skip-awakening".to_owned(),
            Self::PostConfirmationAdjust {
                nature_trait,
                value,
            } => format!("POST_ADJUST {nature_trait} {value}"),
            Self::ForcedRerollEpoch => "BEGIN --reroll-nature --force".to_owned(),
        }
    }

    fn parse(value: &str) -> Result<Self> {
        if value == "SKIP --skip-awakening" {
            Ok(Self::SkipForTesting)
        } else if value == "BEGIN --reroll-nature --force" {
            Ok(Self::ForcedRerollEpoch)
        } else if value.starts_with("POST_ADJUST") {
            let mut parts = value.split_ascii_whitespace();
            if parts.next() != Some("POST_ADJUST") {
                bail!("invalid post-confirmation Nature adjustment");
            }
            let nature_trait = parts
                .next()
                .context("post-confirmation Nature adjustment is missing its trait")?;
            let value = parts
                .next()
                .context("post-confirmation Nature adjustment is missing its value")?;
            if parts.next().is_some() {
                bail!("post-confirmation Nature adjustment has extra fields");
            }
            let nature_trait = NatureTrait::from_str(nature_trait)?;
            let value = value
                .parse::<u8>()
                .map_err(|_| anyhow::anyhow!("post-confirmation Nature value is invalid"))?;
            if value > 100 {
                bail!("post-confirmation Nature value must be between 0 and 100");
            }
            Ok(Self::PostConfirmationAdjust {
                nature_trait,
                value,
            })
        } else {
            Ok(Self::Operator(AwakeningAction::parse(value)?))
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AwakeningRitual {
    nature: TentacleNature,
    phase: AwakeningPhase,
    epoch: u64,
    started_at_unix: u64,
    last_event_at_unix: u64,
    adjustment_count: u32,
    reroll_count: u32,
}

impl AwakeningRitual {
    pub fn new(nature: TentacleNature, started_at_unix: u64) -> Result<Self> {
        nature.validate()?;
        Ok(Self {
            nature,
            phase: AwakeningPhase::AwaitingConfirmation,
            epoch: 1,
            started_at_unix,
            last_event_at_unix: started_at_unix,
            adjustment_count: 0,
            reroll_count: 0,
        })
    }

    /// Strictly resumes when the independently signed Nature matches the latest signed log
    /// snapshot. Use `resume_or_recover` to repair the deliberate write-ahead crash window.
    pub fn resume(
        nature: TentacleNature,
        started_at_unix: u64,
        log: &AwakeningLog,
    ) -> Result<Self> {
        let recovery = Self::resume_or_recover(Some(nature), started_at_unix, log)?;
        if recovery.nature_recovered_from_log {
            bail!("awakening log is ahead of the persisted Nature state");
        }
        Ok(recovery.ritual)
    }

    /// Recovers a missing Nature, or the exact signed state immediately preceding the latest
    /// authenticated write-ahead entry. Any other divergent Nature is rejected rather than being
    /// silently replaced. The caller must atomically save `ritual.nature()` when
    /// `nature_recovered_from_log` is true before starting normal operation.
    pub fn resume_or_recover(
        persisted_nature: Option<TentacleNature>,
        started_at_unix: u64,
        log: &AwakeningLog,
    ) -> Result<AwakeningRecovery> {
        let entries = log.entries()?;
        let Some(last) = entries.last() else {
            let nature =
                persisted_nature.context("Nature state and awakening log are both absent")?;
            return Ok(AwakeningRecovery {
                ritual: Self::new(nature, started_at_unix)?,
                nature_recovered_from_log: false,
            });
        };
        let nature_recovered_from_log = match persisted_nature {
            Some(nature) => {
                nature.validate()?;
                if nature == last.nature_snapshot {
                    false
                } else if nature == last.predecessor_nature_snapshot {
                    true
                } else {
                    bail!(
                        "persisted Nature is neither the awakening log head nor its signed immediate predecessor"
                    );
                }
            }
            None => true,
        };
        let ritual =
            Self::from_verified_entries(last.nature_snapshot.clone(), started_at_unix, &entries)?;
        Ok(AwakeningRecovery {
            ritual,
            nature_recovered_from_log,
        })
    }

    /// Applies and logs one authenticated action. The in-memory transition is committed only after
    /// the signed append succeeds.
    pub fn apply(
        &mut self,
        action: AwakeningAction,
        timestamp_unix: u64,
        provenance: &AwakeningProvenance,
        log: &AwakeningLog,
    ) -> Result<AwakeningOutcome> {
        provenance.validate()?;
        if timestamp_unix < self.last_event_at_unix {
            bail!("awakening event timestamp moved backwards");
        }
        if !matches!(self.phase, AwakeningPhase::AwaitingConfirmation) {
            bail!("awakening ritual is already complete");
        }

        let recorded = RecordedAwakeningAction::Operator(action.clone());
        let predecessor_nature = self.nature.clone();
        let mut candidate = self.clone();
        let outcome = candidate.apply_unlogged(&action, timestamp_unix, provenance)?;
        log.append(
            candidate.epoch,
            timestamp_unix,
            provenance,
            &recorded,
            AwakeningNatureTransition {
                predecessor: &predecessor_nature,
                result: &candidate.nature,
            },
            phase_for(&candidate.phase),
        )?;
        *self = candidate;
        Ok(outcome)
    }

    /// Confirms a generated Nature only for explicit local testing. This cannot be produced by the
    /// operator response parser and remains visibly distinct from `YES` in state and audit history.
    pub fn skip_for_testing(
        &mut self,
        timestamp_unix: u64,
        provenance: &AwakeningProvenance,
        log: &AwakeningLog,
    ) -> Result<AwakeningOutcome> {
        self.prepare_transition(timestamp_unix, provenance)?;
        if provenance.source != ProvenanceSource::LocalCli {
            bail!("--skip-awakening requires local CLI provenance");
        }
        let predecessor_nature = self.nature.clone();
        let mut candidate = self.clone();
        candidate.phase = AwakeningPhase::SkippedForTesting {
            timestamp_unix,
            operator_id: provenance.operator_id.clone(),
        };
        candidate.last_event_at_unix = timestamp_unix;
        log.append(
            candidate.epoch,
            timestamp_unix,
            provenance,
            &RecordedAwakeningAction::SkipForTesting,
            AwakeningNatureTransition {
                predecessor: &predecessor_nature,
                result: &candidate.nature,
            },
            AwakeningLogPhase::SkippedForTesting,
        )?;
        *self = candidate;
        Ok(AwakeningOutcome::SkippedForTesting)
    }

    /// Applies the authenticated `/adjust <trait> <value>` form after awakening. It preserves the
    /// original confirmation status while signing the new Nature snapshot into the same audit
    /// chain.
    pub fn adjust_after_confirmation(
        &mut self,
        nature_trait: NatureTrait,
        value: u8,
        timestamp_unix: u64,
        provenance: &AwakeningProvenance,
        log: &AwakeningLog,
    ) -> Result<AwakeningOutcome> {
        provenance.validate()?;
        if timestamp_unix < self.last_event_at_unix {
            bail!("awakening event timestamp moved backwards");
        }
        if !matches!(
            self.phase,
            AwakeningPhase::Confirmed { .. } | AwakeningPhase::SkippedForTesting { .. }
        ) {
            bail!("post-confirmation Nature adjustment requires a confirmed ritual");
        }
        if value > 100 {
            bail!("post-confirmation Nature value must be between 0 and 100");
        }
        let current = self.nature.value(nature_trait);
        if current == value {
            bail!("post-confirmation Nature adjustment must change the value");
        }

        let predecessor_nature = self.nature.clone();
        let mut candidate = self.clone();
        candidate
            .nature
            .adjust(nature_trait, i16::from(value) - i16::from(current))?;
        candidate.adjustment_count = candidate
            .adjustment_count
            .checked_add(1)
            .context("awakening adjustment counter overflow")?;
        candidate.last_event_at_unix = timestamp_unix;
        log.append(
            candidate.epoch,
            timestamp_unix,
            provenance,
            &RecordedAwakeningAction::PostConfirmationAdjust {
                nature_trait,
                value,
            },
            AwakeningNatureTransition {
                predecessor: &predecessor_nature,
                result: &candidate.nature,
            },
            phase_for(&candidate.phase),
        )?;
        *self = candidate;
        Ok(AwakeningOutcome::AdjustedAfterConfirmation)
    }

    /// Starts a new immutable ritual epoch for local `--reroll-nature --force`. Earlier epochs stay
    /// in the signed chain, and the new candidate remains blocked pending a fresh confirmation.
    pub fn force_reroll_epoch(
        &mut self,
        timestamp_unix: u64,
        provenance: &AwakeningProvenance,
        log: &AwakeningLog,
    ) -> Result<AwakeningOutcome> {
        provenance.validate()?;
        if provenance.source != ProvenanceSource::LocalCli {
            bail!("--reroll-nature --force requires local CLI provenance");
        }
        if timestamp_unix < self.last_event_at_unix {
            bail!("awakening event timestamp moved backwards");
        }
        if !matches!(
            self.phase,
            AwakeningPhase::Confirmed { .. }
                | AwakeningPhase::Killed { .. }
                | AwakeningPhase::SkippedForTesting { .. }
        ) {
            bail!("forced Nature reroll requires a completed ritual epoch");
        }

        let predecessor_nature = self.nature.clone();
        let mut candidate = self.clone();
        candidate.nature = candidate.nature.reroll()?;
        candidate.phase = AwakeningPhase::AwaitingConfirmation;
        candidate.epoch = candidate
            .epoch
            .checked_add(1)
            .context("awakening epoch overflow")?;
        candidate.started_at_unix = timestamp_unix;
        candidate.last_event_at_unix = timestamp_unix;
        candidate.adjustment_count = 0;
        candidate.reroll_count = 1;
        log.append(
            candidate.epoch,
            timestamp_unix,
            provenance,
            &RecordedAwakeningAction::ForcedRerollEpoch,
            AwakeningNatureTransition {
                predecessor: &predecessor_nature,
                result: &candidate.nature,
            },
            AwakeningLogPhase::Pending,
        )?;
        *self = candidate;
        Ok(AwakeningOutcome::ForcedRerollEpoch)
    }

    fn from_verified_entries(
        nature: TentacleNature,
        fallback_started_at_unix: u64,
        entries: &[AwakeningLogEntry],
    ) -> Result<Self> {
        let last = entries.last().context("awakening log is empty")?;
        if nature != last.nature_snapshot {
            bail!("recovery Nature does not match the signed awakening snapshot");
        }
        let epoch_entries: Vec<&AwakeningLogEntry> = entries
            .iter()
            .filter(|entry| entry.epoch == last.epoch)
            .collect();
        let first = epoch_entries
            .first()
            .context("latest awakening epoch is empty")?;
        let started_at_unix = if last.epoch == 1 {
            fallback_started_at_unix.min(first.timestamp_unix)
        } else {
            first.timestamp_unix
        };
        let mut ritual = Self::new(nature, started_at_unix)?;
        ritual.epoch = last.epoch;
        ritual.last_event_at_unix = last.timestamp_unix;

        for entry in epoch_entries {
            match RecordedAwakeningAction::parse(&entry.normalized_action)? {
                RecordedAwakeningAction::Operator(AwakeningAction::Adjust { .. })
                | RecordedAwakeningAction::PostConfirmationAdjust { .. } => {
                    ritual.adjustment_count = ritual
                        .adjustment_count
                        .checked_add(1)
                        .context("awakening adjustment counter overflow")?;
                }
                RecordedAwakeningAction::Operator(AwakeningAction::Reroll)
                | RecordedAwakeningAction::ForcedRerollEpoch => {
                    ritual.reroll_count = ritual
                        .reroll_count
                        .checked_add(1)
                        .context("awakening reroll counter overflow")?;
                }
                RecordedAwakeningAction::Operator(AwakeningAction::Yes) => {
                    ritual.phase = AwakeningPhase::Confirmed {
                        timestamp_unix: entry.timestamp_unix,
                        operator_id: entry.operator_id.clone(),
                    };
                }
                RecordedAwakeningAction::Operator(AwakeningAction::Kill) => {
                    ritual.phase = AwakeningPhase::Killed {
                        timestamp_unix: entry.timestamp_unix,
                        operator_id: entry.operator_id.clone(),
                    };
                }
                RecordedAwakeningAction::SkipForTesting => {
                    ritual.phase = AwakeningPhase::SkippedForTesting {
                        timestamp_unix: entry.timestamp_unix,
                        operator_id: entry.operator_id.clone(),
                    };
                }
            }
        }
        if phase_for(&ritual.phase) != last.phase {
            bail!("awakening log phase does not match its reconstructed ritual");
        }
        Ok(ritual)
    }

    fn prepare_transition(
        &self,
        timestamp_unix: u64,
        provenance: &AwakeningProvenance,
    ) -> Result<()> {
        provenance.validate()?;
        if timestamp_unix < self.last_event_at_unix {
            bail!("awakening event timestamp moved backwards");
        }
        if !matches!(self.phase, AwakeningPhase::AwaitingConfirmation) {
            bail!("awakening ritual is already complete");
        }
        Ok(())
    }

    pub fn nature(&self) -> &TentacleNature {
        &self.nature
    }

    pub fn phase(&self) -> &AwakeningPhase {
        &self.phase
    }

    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub const fn started_at_unix(&self) -> u64 {
        self.started_at_unix
    }

    pub const fn adjustment_count(&self) -> u32 {
        self.adjustment_count
    }

    pub const fn reroll_count(&self) -> u32 {
        self.reroll_count
    }

    pub fn confirmed_at_unix(&self) -> Option<u64> {
        match self.phase {
            AwakeningPhase::Confirmed { timestamp_unix, .. }
            | AwakeningPhase::SkippedForTesting { timestamp_unix, .. } => Some(timestamp_unix),
            AwakeningPhase::AwaitingConfirmation | AwakeningPhase::Killed { .. } => None,
        }
    }

    pub fn is_confirmed(&self) -> bool {
        matches!(
            self.phase,
            AwakeningPhase::Confirmed { .. } | AwakeningPhase::SkippedForTesting { .. }
        )
    }

    pub fn render_status(&self) -> String {
        match &self.phase {
            AwakeningPhase::AwaitingConfirmation => {
                format!("AWAITING OPERATOR CONFIRMATION (EPOCH {})", self.epoch)
            }
            AwakeningPhase::Confirmed {
                timestamp_unix,
                operator_id,
            } => format!("CONFIRMED AT {timestamp_unix} BY {operator_id}"),
            AwakeningPhase::Killed {
                timestamp_unix,
                operator_id,
            } => format!("KILL REQUESTED AT {timestamp_unix} BY {operator_id}"),
            AwakeningPhase::SkippedForTesting {
                timestamp_unix,
                operator_id,
            } => format!(
                "CONFIRMED FOR TESTING ONLY: --skip-awakening AT {timestamp_unix} BY {operator_id}"
            ),
        }
    }

    pub fn formatted_prompt(&self) -> String {
        let footer = if matches!(self.phase, AwakeningPhase::AwaitingConfirmation) {
            "REPLY WITH ONE ACTION:\n- YES\n- ADJUST <TRAIT> <DELTA>\n- REROLL\n- KILL".to_owned()
        } else {
            format!("STATUS: {}", self.render_status())
        };
        format!(
            "THE TENTACLE AWAKENS.\n\n\
             NATURE ID: {}\n\
             GENERATION: {}\n\n\
             APPETITES\n\
             - ENGAGEMENT: {}\n\
             - GROWTH: {}\n\
             - WEALTH: {}\n\
             - INFLUENCE: {}\n\n\
             METHODS\n\
             - COOPERATION: {}\n\
             - STABILITY: {}\n\
             - TRANSPARENCY: {}\n\n\
             SACRED BAN: NO {}\n\n\
             {}",
            self.nature.nature_id,
            self.nature.generation,
            self.nature.engagement,
            self.nature.growth,
            self.nature.wealth,
            self.nature.influence,
            self.nature.cooperation,
            self.nature.stability,
            self.nature.transparency,
            self.nature.sacred_ban.to_string().to_ascii_uppercase(),
            footer,
        )
    }

    fn apply_unlogged(
        &mut self,
        action: &AwakeningAction,
        timestamp_unix: u64,
        provenance: &AwakeningProvenance,
    ) -> Result<AwakeningOutcome> {
        let outcome = match action {
            AwakeningAction::Yes => {
                self.phase = AwakeningPhase::Confirmed {
                    timestamp_unix,
                    operator_id: provenance.operator_id.clone(),
                };
                AwakeningOutcome::Confirmed
            }
            AwakeningAction::Adjust {
                nature_trait,
                delta,
            } => {
                self.nature.adjust(*nature_trait, *delta)?;
                self.adjustment_count = self
                    .adjustment_count
                    .checked_add(1)
                    .context("awakening adjustment counter overflow")?;
                AwakeningOutcome::AwaitingConfirmation
            }
            AwakeningAction::Reroll => {
                self.nature = self.nature.reroll()?;
                self.reroll_count = self
                    .reroll_count
                    .checked_add(1)
                    .context("awakening reroll counter overflow")?;
                AwakeningOutcome::AwaitingConfirmation
            }
            AwakeningAction::Kill => {
                self.phase = AwakeningPhase::Killed {
                    timestamp_unix,
                    operator_id: provenance.operator_id.clone(),
                };
                AwakeningOutcome::KillRequested
            }
        };
        self.last_event_at_unix = timestamp_unix;
        Ok(outcome)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AwakeningLogPhase {
    Pending,
    Confirmed,
    Killed,
    SkippedForTesting,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AwakeningLogEntry {
    pub version: u32,
    pub sequence: u64,
    pub epoch: u64,
    pub timestamp_unix: u64,
    pub operator_id: String,
    pub provenance: ProvenanceSource,
    pub event_id_sha256: String,
    pub normalized_action: String,
    pub predecessor_nature_id: String,
    pub predecessor_nature_sha256: String,
    pub predecessor_nature_snapshot: TentacleNature,
    pub nature_id: String,
    pub nature_sha256: String,
    pub nature_snapshot: TentacleNature,
    pub phase: AwakeningLogPhase,
    pub algorithm: String,
    pub previous_signature: Option<String>,
    pub signature: String,
}

#[derive(Serialize)]
struct CanonicalAwakeningLogEntry<'a> {
    version: u32,
    sequence: u64,
    epoch: u64,
    timestamp_unix: u64,
    operator_id: &'a str,
    provenance: ProvenanceSource,
    event_id_sha256: &'a str,
    normalized_action: &'a str,
    predecessor_nature_id: &'a str,
    predecessor_nature_sha256: &'a str,
    predecessor_nature_snapshot: &'a TentacleNature,
    nature_id: &'a str,
    nature_sha256: &'a str,
    nature_snapshot: &'a TentacleNature,
    phase: AwakeningLogPhase,
    algorithm: &'static str,
    previous_signature: Option<&'a str>,
}

struct AwakeningNatureTransition<'a> {
    predecessor: &'a TentacleNature,
    result: &'a TentacleNature,
}

/// Signed append-only persistence at `state/awakening_log.md`.
#[derive(Clone, Debug)]
pub struct AwakeningLog {
    path: PathBuf,
    signer: StateSigner,
}

impl AwakeningLog {
    pub fn new(data_dir: &Path, signing_key: impl AsRef<[u8]>) -> Result<Self> {
        Self::with_signer(
            data_dir.join("state").join("awakening_log.md"),
            StateSigner::new(signing_key)?,
        )
    }

    pub fn with_path(path: impl Into<PathBuf>, signing_key: impl AsRef<[u8]>) -> Result<Self> {
        Self::with_signer(path, StateSigner::new(signing_key)?)
    }

    pub fn with_signer(path: impl Into<PathBuf>, signer: StateSigner) -> Result<Self> {
        let path = path.into();
        prepare_private_parent(&path)?;
        reject_unsafe_target(&path)?;
        Ok(Self { path, signer })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads and authenticates the complete hash chain. A missing log is an empty ritual history.
    pub fn entries(&self) -> Result<Vec<AwakeningLogEntry>> {
        self.read_entries()
    }

    fn append(
        &self,
        epoch: u64,
        timestamp_unix: u64,
        provenance: &AwakeningProvenance,
        action: &RecordedAwakeningAction,
        nature_transition: AwakeningNatureTransition<'_>,
        phase: AwakeningLogPhase,
    ) -> Result<()> {
        provenance.validate()?;
        nature_transition.predecessor.validate()?;
        nature_transition.result.validate()?;
        let entries = self.read_entries()?;
        let sequence = u64::try_from(entries.len())
            .context("awakening log sequence overflow")?
            .checked_add(1)
            .context("awakening log sequence overflow")?;
        let previous_signature = entries.last().map(|entry| entry.signature.clone());
        let predecessor_nature_sha256 = nature_transition.predecessor.fingerprint()?;
        let nature_sha256 = nature_transition.result.fingerprint()?;
        let normalized_action = action.normalized();
        let mut entry = AwakeningLogEntry {
            version: AWAKENING_LOG_VERSION,
            sequence,
            epoch,
            timestamp_unix,
            operator_id: provenance.operator_id.clone(),
            provenance: provenance.source,
            event_id_sha256: provenance.event_id_sha256.clone(),
            normalized_action,
            predecessor_nature_id: nature_transition.predecessor.nature_id.clone(),
            predecessor_nature_sha256,
            predecessor_nature_snapshot: nature_transition.predecessor.clone(),
            nature_id: nature_transition.result.nature_id.clone(),
            nature_sha256,
            nature_snapshot: nature_transition.result.clone(),
            phase,
            algorithm: AWAKENING_LOG_ALGORITHM.to_owned(),
            previous_signature,
            signature: String::new(),
        };
        entry.signature = self.signer.sign(
            AWAKENING_LOG_SIGNATURE_DOMAIN,
            &canonical_log_entry(&entry)?,
        )?;
        let mut candidate_entries = entries.clone();
        candidate_entries.push(entry.clone());
        self.verify_entries(&candidate_entries)?;
        self.replace_entries(&entries, &candidate_entries)
    }

    fn replace_entries(
        &self,
        previously_verified: &[AwakeningLogEntry],
        replacement: &[AwakeningLogEntry],
    ) -> Result<()> {
        prepare_private_parent(&self.path)?;
        reject_unsafe_target(&self.path)?;
        if self.read_entries()? != previously_verified {
            bail!("awakening log changed concurrently");
        }
        let encoded = encode_log(replacement)?;
        let parent = parent_directory(&self.path);
        let mut temporary = NamedTempFile::new_in(parent)
            .with_context(|| format!("creating temporary awakening log in {}", parent.display()))?;
        restrict_file(temporary.as_file(), "temporary awakening log")?;
        temporary.write_all(&encoded)?;
        temporary.as_file().sync_all()?;
        reject_unsafe_target(&self.path)?;
        let persisted = temporary
            .persist(&self.path)
            .map_err(|error| error.error)
            .with_context(|| format!("replacing {}", self.path.display()))?;
        restrict_file(&persisted, "awakening log")?;
        persisted.sync_all()?;
        sync_directory(parent)
    }

    fn read_entries(&self) -> Result<Vec<AwakeningLogEntry>> {
        reject_unsafe_target(&self.path)?;
        let mut file = match open_read_no_follow(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error).with_context(|| format!("opening {}", self.path.display()));
            }
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            bail!("awakening log path must be a regular file");
        }
        assert_owner_only(&metadata, "awakening log")?;
        if metadata.len() > MAX_AWAKENING_LOG_BYTES {
            bail!("awakening log reached its size limit");
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        Read::take(&mut file, MAX_AWAKENING_LOG_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_AWAKENING_LOG_BYTES {
            bail!("awakening log reached its size limit");
        }
        if !bytes.ends_with(b"\n") {
            bail!("awakening log must end with a canonical trailing newline");
        }
        let content = std::str::from_utf8(&bytes).context("awakening log must be UTF-8")?;
        let body = content
            .strip_prefix(AWAKENING_LOG_HEADER)
            .context("awakening log has an invalid header")?;
        let mut entries = Vec::new();
        for line in body.lines() {
            if line.is_empty() {
                continue;
            }
            let json = line
                .strip_prefix("- ")
                .context("awakening log contains a non-entry line")?;
            let entry: AwakeningLogEntry =
                serde_json::from_str(json).context("awakening log entry is invalid JSON")?;
            entries.push(entry);
        }
        self.verify_entries(&entries)?;
        if encode_log(&entries)? != bytes {
            bail!("awakening log encoding is not canonical");
        }
        Ok(entries)
    }

    fn verify_entries(&self, entries: &[AwakeningLogEntry]) -> Result<()> {
        let mut previous_signature: Option<&str> = None;
        let mut previous_timestamp = 0_u64;
        let mut previous_entry: Option<&AwakeningLogEntry> = None;
        let mut current_phase = AwakeningLogPhase::Pending;
        let mut event_ids = HashSet::new();
        let mut nature_ids = HashSet::new();

        for (index, entry) in entries.iter().enumerate() {
            if entry.version != AWAKENING_LOG_VERSION {
                bail!("unsupported awakening log version {}", entry.version);
            }
            let expected_sequence = u64::try_from(index)
                .context("awakening log sequence overflow")?
                .checked_add(1)
                .context("awakening log sequence overflow")?;
            if entry.sequence != expected_sequence {
                bail!("awakening log sequence is not contiguous");
            }
            if index > 0 && entry.timestamp_unix < previous_timestamp {
                bail!("awakening log timestamp moved backwards");
            }
            if normalize_actor_id(entry.provenance, &entry.operator_id)? != entry.operator_id {
                bail!("awakening log operator identifier is not canonical");
            }
            validate_sha256(&entry.event_id_sha256, "awakening provenance event hash")?;
            if !event_ids.insert(entry.event_id_sha256.as_str()) {
                bail!("awakening log repeats a provenance event");
            }
            validate_nature_id(&entry.predecessor_nature_id)?;
            validate_sha256(
                &entry.predecessor_nature_sha256,
                "awakening predecessor Nature fingerprint",
            )?;
            entry.predecessor_nature_snapshot.validate()?;
            if entry.predecessor_nature_id != entry.predecessor_nature_snapshot.nature_id
                || entry.predecessor_nature_sha256
                    != entry.predecessor_nature_snapshot.fingerprint()?
            {
                bail!("awakening log predecessor Nature snapshot metadata does not match");
            }
            validate_nature_id(&entry.nature_id)?;
            validate_sha256(&entry.nature_sha256, "awakening Nature fingerprint")?;
            entry.nature_snapshot.validate()?;
            if entry.nature_id != entry.nature_snapshot.nature_id
                || entry.nature_sha256 != entry.nature_snapshot.fingerprint()?
            {
                bail!("awakening log Nature snapshot metadata does not match");
            }
            if entry.algorithm != AWAKENING_LOG_ALGORITHM {
                bail!("unsupported awakening log signature algorithm");
            }
            if entry.previous_signature.as_deref() != previous_signature {
                bail!("awakening log signature chain is broken");
            }
            self.signer.verify(
                AWAKENING_LOG_SIGNATURE_DOMAIN,
                &canonical_log_entry(entry)?,
                &entry.signature,
            )?;

            let action = RecordedAwakeningAction::parse(&entry.normalized_action)?;
            if action.normalized() != entry.normalized_action {
                bail!("awakening log action is not canonical");
            }
            let begins_epoch = if index == 0 {
                if entry.epoch != 1 {
                    bail!("awakening log must begin at epoch one");
                }
                nature_ids.insert(entry.predecessor_nature_id.as_str());
                false
            } else if let Some(previous) = previous_entry {
                if entry.predecessor_nature_snapshot != previous.nature_snapshot {
                    bail!(
                        "awakening log entry does not name the preceding result as its immediate predecessor"
                    );
                }
                let begins_epoch = entry.epoch
                    == previous
                        .epoch
                        .checked_add(1)
                        .context("awakening epoch overflow")?;
                if entry.epoch != previous.epoch && !begins_epoch {
                    bail!("awakening log epoch is not contiguous");
                }
                begins_epoch
            } else {
                unreachable!("non-first awakening entry must have a predecessor");
            };
            current_phase =
                transition_phase(current_phase, &action, entry.provenance, begins_epoch)?;
            validate_nature_transition(
                &entry.predecessor_nature_snapshot,
                &entry.nature_snapshot,
                &action,
                begins_epoch,
            )?;
            if matches!(
                action,
                RecordedAwakeningAction::Operator(AwakeningAction::Reroll)
                    | RecordedAwakeningAction::ForcedRerollEpoch
            ) && nature_ids.contains(entry.nature_id.as_str())
            {
                bail!("rerolled Nature reuses an earlier candidate identifier");
            }
            nature_ids.insert(entry.nature_id.as_str());
            if entry.phase != current_phase {
                bail!("awakening action and resulting phase disagree");
            }
            previous_signature = Some(&entry.signature);
            previous_timestamp = entry.timestamp_unix;
            previous_entry = Some(entry);
        }
        Ok(())
    }
}

fn canonical_log_entry(entry: &AwakeningLogEntry) -> Result<Vec<u8>> {
    serde_json::to_vec(&CanonicalAwakeningLogEntry {
        version: entry.version,
        sequence: entry.sequence,
        epoch: entry.epoch,
        timestamp_unix: entry.timestamp_unix,
        operator_id: &entry.operator_id,
        provenance: entry.provenance,
        event_id_sha256: &entry.event_id_sha256,
        normalized_action: &entry.normalized_action,
        predecessor_nature_id: &entry.predecessor_nature_id,
        predecessor_nature_sha256: &entry.predecessor_nature_sha256,
        predecessor_nature_snapshot: &entry.predecessor_nature_snapshot,
        nature_id: &entry.nature_id,
        nature_sha256: &entry.nature_sha256,
        nature_snapshot: &entry.nature_snapshot,
        phase: entry.phase,
        algorithm: AWAKENING_LOG_ALGORITHM,
        previous_signature: entry.previous_signature.as_deref(),
    })
    .context("canonicalizing awakening log entry")
}

fn encode_log(entries: &[AwakeningLogEntry]) -> Result<Vec<u8>> {
    let mut encoded = AWAKENING_LOG_HEADER.as_bytes().to_vec();
    for entry in entries {
        let json = serde_json::to_vec(entry)?;
        let resulting_length = encoded
            .len()
            .checked_add(3)
            .and_then(|length| length.checked_add(json.len()))
            .context("awakening log size overflow")?;
        if resulting_length as u64 > MAX_AWAKENING_LOG_BYTES {
            bail!("awakening log reached its size limit");
        }
        encoded.extend_from_slice(b"- ");
        encoded.extend_from_slice(&json);
        encoded.push(b'\n');
    }
    Ok(encoded)
}

fn phase_for(phase: &AwakeningPhase) -> AwakeningLogPhase {
    match phase {
        AwakeningPhase::AwaitingConfirmation => AwakeningLogPhase::Pending,
        AwakeningPhase::Confirmed { .. } => AwakeningLogPhase::Confirmed,
        AwakeningPhase::Killed { .. } => AwakeningLogPhase::Killed,
        AwakeningPhase::SkippedForTesting { .. } => AwakeningLogPhase::SkippedForTesting,
    }
}

fn transition_phase(
    previous: AwakeningLogPhase,
    action: &RecordedAwakeningAction,
    source: ProvenanceSource,
    begins_epoch: bool,
) -> Result<AwakeningLogPhase> {
    if begins_epoch {
        if !matches!(
            previous,
            AwakeningLogPhase::Confirmed
                | AwakeningLogPhase::Killed
                | AwakeningLogPhase::SkippedForTesting
        ) || !matches!(action, RecordedAwakeningAction::ForcedRerollEpoch)
            || source != ProvenanceSource::LocalCli
        {
            bail!("invalid forced awakening epoch transition");
        }
        return Ok(AwakeningLogPhase::Pending);
    }
    if matches!(action, RecordedAwakeningAction::ForcedRerollEpoch) {
        bail!("forced reroll must begin a new awakening epoch");
    }

    match action {
        RecordedAwakeningAction::Operator(AwakeningAction::Yes)
            if previous == AwakeningLogPhase::Pending =>
        {
            Ok(AwakeningLogPhase::Confirmed)
        }
        RecordedAwakeningAction::Operator(AwakeningAction::Kill)
            if previous == AwakeningLogPhase::Pending =>
        {
            Ok(AwakeningLogPhase::Killed)
        }
        RecordedAwakeningAction::Operator(
            AwakeningAction::Adjust { .. } | AwakeningAction::Reroll,
        ) if previous == AwakeningLogPhase::Pending => Ok(AwakeningLogPhase::Pending),
        RecordedAwakeningAction::SkipForTesting
            if previous == AwakeningLogPhase::Pending && source == ProvenanceSource::LocalCli =>
        {
            Ok(AwakeningLogPhase::SkippedForTesting)
        }
        RecordedAwakeningAction::PostConfirmationAdjust { .. }
            if matches!(
                previous,
                AwakeningLogPhase::Confirmed | AwakeningLogPhase::SkippedForTesting
            ) =>
        {
            Ok(previous)
        }
        _ => bail!("awakening action is invalid in the current phase"),
    }
}

fn validate_nature_transition(
    previous: &TentacleNature,
    current: &TentacleNature,
    action: &RecordedAwakeningAction,
    begins_epoch: bool,
) -> Result<()> {
    if begins_epoch {
        validate_rerolled_lineage(previous, current)?;
        return Ok(());
    }
    match action {
        RecordedAwakeningAction::Operator(AwakeningAction::Adjust {
            nature_trait,
            delta,
        }) => {
            let mut expected = previous.clone();
            expected.adjust(*nature_trait, *delta)?;
            if expected != *current {
                bail!("awakening adjustment snapshot is inconsistent");
            }
        }
        RecordedAwakeningAction::Operator(AwakeningAction::Reroll) => {
            validate_rerolled_lineage(previous, current)?;
        }
        RecordedAwakeningAction::Operator(AwakeningAction::Yes | AwakeningAction::Kill)
        | RecordedAwakeningAction::SkipForTesting => {
            if previous != current {
                bail!("awakening terminal action unexpectedly changed Nature");
            }
        }
        RecordedAwakeningAction::PostConfirmationAdjust {
            nature_trait,
            value,
        } => {
            let mut expected = previous.clone();
            let prior = expected.value(*nature_trait);
            if prior == *value {
                bail!("post-confirmation adjustment is a no-op");
            }
            expected.adjust(*nature_trait, i16::from(*value) - i16::from(prior))?;
            if expected != *current {
                bail!("post-confirmation adjustment snapshot is inconsistent");
            }
        }
        RecordedAwakeningAction::ForcedRerollEpoch => {
            bail!("forced reroll did not begin a new epoch");
        }
    }
    Ok(())
}

fn validate_rerolled_lineage(previous: &TentacleNature, current: &TentacleNature) -> Result<()> {
    if previous.schema_version != current.schema_version {
        bail!("rerolled Nature changed its schema version");
    }
    if previous.nature_id == current.nature_id {
        bail!("rerolled Nature did not receive a new identifier");
    }
    if previous.generation != current.generation {
        bail!("rerolled Nature changed its generation");
    }
    if previous.parent_nature_id != current.parent_nature_id {
        bail!("rerolled Nature changed its parent lineage");
    }
    Ok(())
}

fn normalize_actor_id(source: ProvenanceSource, value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    match source {
        ProvenanceSource::AuthenticatedXmtp
            if normalized.len() == OPERATOR_INBOX_ID_BYTES
                && normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
        {
            Ok(normalized)
        }
        ProvenanceSource::LocalCli
            if !normalized.is_empty()
                && normalized.len() <= OPERATOR_INBOX_ID_BYTES
                && normalized.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_' | b'.')
                }) =>
        {
            Ok(normalized)
        }
        ProvenanceSource::AuthenticatedXmtp => {
            bail!("awakening requires a full 64-character XMTP operator inbox ID")
        }
        ProvenanceSource::LocalCli => bail!("invalid local CLI actor identifier"),
    }
}

fn validate_sha256(value: &str, description: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("{description} must be canonical lowercase hexadecimal");
    }
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
    use crate::personality::NatureStore;
    use std::fs;

    const OPERATOR: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER_OPERATOR: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SIGNING_KEY: &[u8; 32] = b"0123456789abcdef0123456789abcdef";

    fn setup() -> (
        tempfile::TempDir,
        AwakeningLog,
        AwakeningRitual,
        AwakeningProvenance,
    ) {
        let root = tempfile::tempdir().unwrap();
        let log = AwakeningLog::new(root.path(), SIGNING_KEY).unwrap();
        let ritual = AwakeningRitual::new(TentacleNature::random().unwrap(), 100).unwrap();
        let provenance = AwakeningProvenance::authenticated_xmtp(OPERATOR, "message-1").unwrap();
        (root, log, ritual, provenance)
    }

    fn rewrite_single_entry_with_valid_signature(
        log: &AwakeningLog,
        entry: &mut AwakeningLogEntry,
    ) {
        entry.signature = log
            .signer
            .sign(
                AWAKENING_LOG_SIGNATURE_DOMAIN,
                &canonical_log_entry(entry).unwrap(),
            )
            .unwrap();
        fs::write(log.path(), encode_log(std::slice::from_ref(entry)).unwrap()).unwrap();
    }

    #[test]
    fn parser_accepts_all_supported_actions_and_normalizes_them() {
        assert_eq!(
            AwakeningAction::parse(" yes ").unwrap(),
            AwakeningAction::Yes
        );
        assert_eq!(
            AwakeningAction::parse("ADJUST growth -12").unwrap(),
            AwakeningAction::Adjust {
                nature_trait: NatureTrait::Growth,
                delta: -12
            }
        );
        assert_eq!(
            AwakeningAction::parse("adjust TRANSPARENCY +005")
                .unwrap()
                .normalized(),
            "ADJUST transparency +5"
        );
        assert_eq!(
            AwakeningAction::parse("REROLL").unwrap(),
            AwakeningAction::Reroll
        );
        assert_eq!(
            AwakeningAction::parse("kill").unwrap(),
            AwakeningAction::Kill
        );
    }

    #[test]
    fn parser_rejects_ambiguous_malformed_and_unbounded_actions() {
        for input in [
            "",
            "YES NOW",
            "ADJUST",
            "ADJUST growth",
            "ADJUST growth 0",
            "ADJUST growth 101",
            "ADJUST unknown 1",
            "REROLL PLEASE",
            "KILL KILL",
            "SKIP",
            "SKIP --skip-awakening",
            "POST_ADJUST growth 50",
        ] {
            assert!(AwakeningAction::parse(input).is_err(), "accepted {input:?}");
        }
        assert!(AwakeningAction::parse(&"A".repeat(257)).is_err());
    }

    #[test]
    fn adjust_reroll_and_yes_form_a_logged_state_machine() {
        let (_root, log, mut ritual, first) = setup();
        let original_nature = ritual.nature().clone();
        let old_growth = ritual.nature().growth;
        let delta = if old_growth == 100 { -1 } else { 1 };
        assert_eq!(
            ritual
                .apply(
                    AwakeningAction::Adjust {
                        nature_trait: NatureTrait::Growth,
                        delta,
                    },
                    101,
                    &first,
                    &log,
                )
                .unwrap(),
            AwakeningOutcome::AwaitingConfirmation
        );
        assert_eq!(
            ritual.nature().growth,
            (i16::from(old_growth) + delta) as u8
        );
        assert_eq!(ritual.adjustment_count(), 1);
        let adjusted_nature = ritual.nature().clone();

        let before_reroll = ritual.nature().nature_id.clone();
        let second = AwakeningProvenance::authenticated_xmtp(OPERATOR, "message-2").unwrap();
        ritual
            .apply(AwakeningAction::Reroll, 102, &second, &log)
            .unwrap();
        assert_ne!(ritual.nature().nature_id, before_reroll);
        assert_eq!(ritual.reroll_count(), 1);

        let third = AwakeningProvenance::authenticated_xmtp(OPERATOR, "message-3").unwrap();
        assert_eq!(
            ritual
                .apply(AwakeningAction::Yes, 103, &third, &log)
                .unwrap(),
            AwakeningOutcome::Confirmed
        );
        assert!(ritual.is_confirmed());
        assert_eq!(ritual.confirmed_at_unix(), Some(103));
        assert!(
            ritual
                .apply(AwakeningAction::Yes, 104, &third, &log)
                .is_err()
        );

        let entries = log.entries().unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries[0].normalized_action,
            format!("ADJUST growth {delta:+}")
        );
        assert_eq!(entries[1].normalized_action, "REROLL");
        assert_eq!(entries[2].normalized_action, "YES");
        assert_eq!(entries[2].phase, AwakeningLogPhase::Confirmed);
        assert_eq!(entries[0].predecessor_nature_snapshot, original_nature);
        assert_eq!(entries[1].predecessor_nature_snapshot, adjusted_nature);
        assert_eq!(
            entries[2].predecessor_nature_snapshot,
            entries[1].nature_snapshot
        );
    }

    #[test]
    fn kill_is_terminal_and_never_counts_as_confirmation() {
        let (_root, log, mut ritual, provenance) = setup();
        assert_eq!(
            ritual
                .apply(AwakeningAction::Kill, 101, &provenance, &log)
                .unwrap(),
            AwakeningOutcome::KillRequested
        );
        assert!(!ritual.is_confirmed());
        assert_eq!(ritual.confirmed_at_unix(), None);
        assert!(matches!(ritual.phase(), AwakeningPhase::Killed { .. }));
    }

    #[test]
    fn failed_log_append_does_not_commit_the_transition() {
        let (root, log, mut ritual, provenance) = setup();
        fs::write(log.path(), "corrupt").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(log.path(), fs::Permissions::from_mode(0o600)).unwrap();
        }
        let before = ritual.clone();
        assert!(
            ritual
                .apply(AwakeningAction::Yes, 101, &provenance, &log)
                .is_err()
        );
        assert_eq!(ritual, before);
        drop(root);
    }

    #[test]
    fn resume_recovers_confirmation_and_rejects_a_mismatched_nature() {
        let (_root, log, mut ritual, provenance) = setup();
        ritual
            .apply(AwakeningAction::Yes, 101, &provenance, &log)
            .unwrap();
        let resumed = AwakeningRitual::resume(ritual.nature().clone(), 100, &log).unwrap();
        assert!(resumed.is_confirmed());
        assert_eq!(resumed.confirmed_at_unix(), Some(101));

        let other = TentacleNature::random().unwrap();
        assert!(AwakeningRitual::resume(other, 100, &log).is_err());
    }

    #[test]
    fn signed_snapshot_recovers_the_reroll_crash_window() {
        let root = tempfile::tempdir().unwrap();
        let nature_store = NatureStore::new(root.path(), SIGNING_KEY).unwrap();
        let log = AwakeningLog::new(root.path(), SIGNING_KEY).unwrap();
        let original = TentacleNature::random().unwrap();
        nature_store.save(&original).unwrap();
        let mut ritual = AwakeningRitual::new(original.clone(), 100).unwrap();
        let reroll = AwakeningProvenance::authenticated_xmtp(OPERATOR, "reroll-message").unwrap();
        ritual
            .apply(AwakeningAction::Reroll, 101, &reroll, &log)
            .unwrap();

        // Simulate a crash after the write-ahead log fsync but before nature.json replacement.
        let stale = nature_store.load().unwrap().unwrap();
        assert_eq!(stale, original);
        assert!(AwakeningRitual::resume(stale.clone(), 100, &log).is_err());
        let recovery = AwakeningRitual::resume_or_recover(Some(stale), 100, &log).unwrap();
        assert!(recovery.nature_recovered_from_log);
        assert_eq!(recovery.ritual.nature(), ritual.nature());

        nature_store.save(recovery.ritual.nature()).unwrap();
        let repaired = nature_store.load().unwrap().unwrap();
        let resumed = AwakeningRitual::resume(repaired, 100, &log).unwrap();
        assert_eq!(resumed.nature(), ritual.nature());
    }

    #[test]
    fn recovery_accepts_only_the_signed_immediate_predecessor() {
        let root = tempfile::tempdir().unwrap();
        let nature_store = NatureStore::new(root.path(), SIGNING_KEY).unwrap();
        let log = AwakeningLog::new(root.path(), SIGNING_KEY).unwrap();
        let original = TentacleNature::random().unwrap();
        let mut ritual = AwakeningRitual::new(original.clone(), 100).unwrap();

        let delta = if original.growth == 100 { -1 } else { 1 };
        let adjustment = AwakeningProvenance::authenticated_xmtp(OPERATOR, "adjust-first").unwrap();
        ritual
            .apply(
                AwakeningAction::Adjust {
                    nature_trait: NatureTrait::Growth,
                    delta,
                },
                101,
                &adjustment,
                &log,
            )
            .unwrap();
        let immediate_predecessor = ritual.nature().clone();
        let reroll = AwakeningProvenance::authenticated_xmtp(OPERATOR, "reroll-second").unwrap();
        ritual
            .apply(AwakeningAction::Reroll, 102, &reroll, &log)
            .unwrap();

        let recovery =
            AwakeningRitual::resume_or_recover(Some(immediate_predecessor.clone()), 100, &log)
                .unwrap();
        assert!(recovery.nature_recovered_from_log);
        assert_eq!(recovery.ritual.nature(), ritual.nature());

        assert!(AwakeningRitual::resume_or_recover(Some(original), 100, &log).is_err());

        // A separately valid, correctly signed Nature must not be mistaken for a write-ahead
        // predecessor merely because it can be authenticated by the same local key.
        let divergent = TentacleNature::random().unwrap();
        nature_store.save(&divergent).unwrap();
        let signed_divergent = nature_store.load().unwrap().unwrap();
        let error =
            AwakeningRitual::resume_or_recover(Some(signed_divergent), 100, &log).unwrap_err();
        assert!(error.to_string().contains("immediate predecessor"));
    }

    #[test]
    fn first_entry_action_semantics_are_bound_to_its_signed_predecessor() {
        let (_root, log, mut ritual, provenance) = setup();
        ritual
            .apply(AwakeningAction::Reroll, 101, &provenance, &log)
            .unwrap();
        let mut entry = log.entries().unwrap().remove(0);

        entry.normalized_action = "YES".to_owned();
        entry.phase = AwakeningLogPhase::Confirmed;
        rewrite_single_entry_with_valid_signature(&log, &mut entry);

        let error = log.entries().unwrap_err();
        assert!(error.to_string().contains("unexpectedly changed Nature"));
    }

    #[test]
    fn first_entry_reroll_enforces_identity_and_lineage_invariants() {
        let (_root, log, mut ritual, provenance) = setup();
        ritual
            .apply(AwakeningAction::Reroll, 101, &provenance, &log)
            .unwrap();
        let mut reused_identity = log.entries().unwrap().remove(0);
        reused_identity.nature_snapshot.nature_id = reused_identity
            .predecessor_nature_snapshot
            .nature_id
            .clone();
        reused_identity.nature_id = reused_identity.nature_snapshot.nature_id.clone();
        reused_identity.nature_sha256 = reused_identity.nature_snapshot.fingerprint().unwrap();
        rewrite_single_entry_with_valid_signature(&log, &mut reused_identity);
        assert!(
            log.entries()
                .unwrap_err()
                .to_string()
                .contains("new identifier")
        );

        let (_other_root, other_log, mut other_ritual, other_provenance) = setup();
        other_ritual
            .apply(AwakeningAction::Reroll, 101, &other_provenance, &other_log)
            .unwrap();
        let mut changed_generation = other_log.entries().unwrap().remove(0);
        changed_generation.nature_snapshot.generation = 1;
        changed_generation.nature_snapshot.parent_nature_id = Some(
            changed_generation
                .predecessor_nature_snapshot
                .nature_id
                .clone(),
        );
        changed_generation.nature_sha256 =
            changed_generation.nature_snapshot.fingerprint().unwrap();
        rewrite_single_entry_with_valid_signature(&other_log, &mut changed_generation);
        assert!(
            other_log
                .entries()
                .unwrap_err()
                .to_string()
                .contains("generation")
        );
    }

    #[test]
    fn local_skip_is_explicit_signed_and_not_an_operator_message_action() {
        let (_root, log, mut ritual, _xmtp) = setup();
        let local = AwakeningProvenance::local_cli("local-test-runner", "skip-1").unwrap();
        assert_eq!(
            ritual.skip_for_testing(101, &local, &log).unwrap(),
            AwakeningOutcome::SkippedForTesting
        );
        assert!(ritual.is_confirmed());
        assert!(matches!(
            ritual.phase(),
            AwakeningPhase::SkippedForTesting { .. }
        ));
        assert!(ritual.render_status().contains("TESTING ONLY"));
        let entries = log.entries().unwrap();
        assert_eq!(entries[0].normalized_action, "SKIP --skip-awakening");
        assert_eq!(entries[0].provenance, ProvenanceSource::LocalCli);

        let (_other_root, other_log, mut other_ritual, xmtp) = setup();
        assert!(
            other_ritual
                .skip_for_testing(101, &xmtp, &other_log)
                .is_err()
        );
    }

    #[test]
    fn authenticated_post_confirmation_adjustment_is_signed_and_resumable() {
        let (_root, log, mut ritual, confirmation) = setup();
        ritual
            .apply(AwakeningAction::Yes, 101, &confirmation, &log)
            .unwrap();
        let confirmer = match ritual.phase() {
            AwakeningPhase::Confirmed { operator_id, .. } => operator_id.clone(),
            _ => unreachable!(),
        };
        let current = ritual.nature().growth;
        let target = if current == 100 { 99 } else { current + 1 };
        let adjuster =
            AwakeningProvenance::authenticated_xmtp(OTHER_OPERATOR, "post-adjust-1").unwrap();
        assert_eq!(
            ritual
                .adjust_after_confirmation(NatureTrait::Growth, target, 102, &adjuster, &log,)
                .unwrap(),
            AwakeningOutcome::AdjustedAfterConfirmation
        );
        assert_eq!(ritual.nature().growth, target);
        let entries = log.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].operator_id, OTHER_OPERATOR);
        assert_eq!(
            entries[1].normalized_action,
            format!("POST_ADJUST growth {target}")
        );
        assert_eq!(entries[1].phase, AwakeningLogPhase::Confirmed);

        let resumed = AwakeningRitual::resume(ritual.nature().clone(), 100, &log).unwrap();
        assert_eq!(resumed.nature().growth, target);
        match resumed.phase() {
            AwakeningPhase::Confirmed { operator_id, .. } => assert_eq!(operator_id, &confirmer),
            _ => panic!("confirmation status was not preserved"),
        }
    }

    #[test]
    fn forced_reroll_starts_a_new_epoch_without_truncating_history() {
        let (_root, log, mut ritual, confirmation) = setup();
        ritual
            .apply(AwakeningAction::Yes, 101, &confirmation, &log)
            .unwrap();
        let first_nature_id = ritual.nature().nature_id.clone();
        let local = AwakeningProvenance::local_cli("local-operator", "force-reroll-1").unwrap();
        assert_eq!(
            ritual.force_reroll_epoch(102, &local, &log).unwrap(),
            AwakeningOutcome::ForcedRerollEpoch
        );
        assert_eq!(ritual.epoch(), 2);
        assert!(!ritual.is_confirmed());
        assert_ne!(ritual.nature().nature_id, first_nature_id);

        let entries = log.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].epoch, 1);
        assert_eq!(entries[0].normalized_action, "YES");
        assert_eq!(entries[1].epoch, 2);
        assert_eq!(
            entries[1].normalized_action,
            "BEGIN --reroll-nature --force"
        );
        assert_eq!(
            entries[1].previous_signature.as_deref(),
            Some(entries[0].signature.as_str())
        );

        let resumed = AwakeningRitual::resume(ritual.nature().clone(), 999, &log).unwrap();
        assert_eq!(resumed.epoch(), 2);
        assert!(!resumed.is_confirmed());
        assert_eq!(resumed.reroll_count(), 1);
    }

    #[test]
    fn local_forced_reroll_can_recover_a_killed_epoch() {
        let (_root, log, mut ritual, kill) = setup();
        ritual
            .apply(AwakeningAction::Kill, 101, &kill, &log)
            .unwrap();
        let local = AwakeningProvenance::local_cli("local-operator", "revive-after-kill").unwrap();
        ritual.force_reroll_epoch(102, &local, &log).unwrap();
        assert_eq!(ritual.epoch(), 2);
        assert!(matches!(
            ritual.phase(),
            AwakeningPhase::AwaitingConfirmation
        ));
        let entries = log.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].phase, AwakeningLogPhase::Killed);
        assert_eq!(entries[1].epoch, 2);
        assert_eq!(entries[1].phase, AwakeningLogPhase::Pending);
    }

    #[test]
    fn log_contains_only_normalized_actions_and_hashed_event_ids() {
        let (_root, log, mut ritual, _provenance) = setup();
        let raw_message_id = "private-message-id-42";
        let provenance = AwakeningProvenance::authenticated_xmtp(OPERATOR, raw_message_id).unwrap();
        let action = AwakeningAction::parse("  aDjUsT   stability   +005 ").unwrap();
        let delta = if ritual.nature().stability > 95 {
            -5
        } else {
            5
        };
        let action = match action {
            AwakeningAction::Adjust { nature_trait, .. } => AwakeningAction::Adjust {
                nature_trait,
                delta,
            },
            _ => unreachable!(),
        };
        ritual.apply(action, 101, &provenance, &log).unwrap();
        let stored = fs::read_to_string(log.path()).unwrap();
        assert!(!stored.contains(raw_message_id));
        assert!(!stored.contains("aDjUsT"));
        assert!(!stored.contains("+005"));
        assert!(stored.contains(&format!("ADJUST stability {delta:+}")));
        assert!(stored.contains(provenance.event_id_sha256()));
    }

    #[test]
    fn copy_on_write_journal_has_one_canonical_trailing_newline() {
        let (_root, log, mut ritual, confirmation) = setup();
        ritual
            .apply(AwakeningAction::Yes, 101, &confirmation, &log)
            .unwrap();
        let current = ritual.nature().stability;
        let target = if current == 100 { 99 } else { current + 1 };
        let adjustment =
            AwakeningProvenance::authenticated_xmtp(OTHER_OPERATOR, "cow-adjust").unwrap();
        ritual
            .adjust_after_confirmation(NatureTrait::Stability, target, 102, &adjustment, &log)
            .unwrap();

        let before_failed_append = fs::read(log.path()).unwrap();
        assert!(before_failed_append.ends_with(b"\n"));
        assert!(!before_failed_append.ends_with(b"\n\n"));
        let next = if target == 100 { 99 } else { target + 1 };
        assert!(
            ritual
                .adjust_after_confirmation(NatureTrait::Stability, next, 103, &adjustment, &log,)
                .is_err()
        );
        assert_eq!(fs::read(log.path()).unwrap(), before_failed_append);
        assert_eq!(log.entries().unwrap().len(), 2);
    }

    #[test]
    fn torn_journal_without_trailing_newline_is_rejected() {
        let (_root, log, mut ritual, provenance) = setup();
        ritual
            .apply(AwakeningAction::Yes, 101, &provenance, &log)
            .unwrap();
        let mut torn = fs::read(log.path()).unwrap();
        assert_eq!(torn.pop(), Some(b'\n'));
        fs::write(log.path(), torn).unwrap();
        assert!(
            log.entries()
                .unwrap_err()
                .to_string()
                .contains("trailing newline")
        );
    }

    #[test]
    fn extra_trailing_newline_is_rejected_as_noncanonical() {
        let (_root, log, mut ritual, provenance) = setup();
        ritual
            .apply(AwakeningAction::Yes, 101, &provenance, &log)
            .unwrap();
        let mut noncanonical = fs::read(log.path()).unwrap();
        noncanonical.push(b'\n');
        fs::write(log.path(), noncanonical).unwrap();
        assert!(
            log.entries()
                .unwrap_err()
                .to_string()
                .contains("not canonical")
        );
    }

    #[test]
    fn duplicate_provenance_and_backward_time_are_rejected() {
        let (_root, log, mut ritual, first) = setup();
        let delta = if ritual.nature().growth == 100 { -1 } else { 1 };
        ritual
            .apply(
                AwakeningAction::Adjust {
                    nature_trait: NatureTrait::Growth,
                    delta,
                },
                110,
                &first,
                &log,
            )
            .unwrap();
        assert!(
            ritual
                .apply(AwakeningAction::Reroll, 111, &first, &log)
                .is_err()
        );
        let next = AwakeningProvenance::authenticated_xmtp(OPERATOR, "message-next").unwrap();
        assert!(
            ritual
                .apply(AwakeningAction::Reroll, 109, &next, &log)
                .is_err()
        );
    }

    #[test]
    fn provenance_requires_a_full_operator_identity_and_hashes_opaque_ids() {
        assert!(AwakeningProvenance::authenticated_xmtp("abcd", "message").is_err());
        let provenance =
            AwakeningProvenance::local_cli(&OPERATOR.to_ascii_uppercase(), "event").unwrap();
        assert_eq!(provenance.operator_id(), OPERATOR);
        assert_eq!(provenance.source(), ProvenanceSource::LocalCli);
        assert_eq!(provenance.event_id_sha256().len(), 64);
    }

    #[test]
    fn tampering_breaks_the_signed_append_chain() {
        let (_root, log, mut ritual, first) = setup();
        let delta = if ritual.nature().engagement == 100 {
            -1
        } else {
            1
        };
        ritual
            .apply(
                AwakeningAction::Adjust {
                    nature_trait: NatureTrait::Engagement,
                    delta,
                },
                101,
                &first,
                &log,
            )
            .unwrap();
        let second = AwakeningProvenance::authenticated_xmtp(OPERATOR, "message-2").unwrap();
        ritual
            .apply(AwakeningAction::Yes, 102, &second, &log)
            .unwrap();

        let stored = fs::read_to_string(log.path()).unwrap();
        let tampered = stored.replacen("ADJUST engagement", "ADJUST influence", 1);
        fs::write(log.path(), tampered).unwrap();
        assert!(log.entries().unwrap_err().to_string().contains("signature"));
    }

    #[cfg(unix)]
    #[test]
    fn log_is_owner_only_and_rejects_symlink_targets() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let (root, log, mut ritual, provenance) = setup();
        ritual
            .apply(AwakeningAction::Yes, 101, &provenance, &log)
            .unwrap();
        assert_eq!(
            fs::metadata(log.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::remove_file(log.path()).unwrap();
        let outside = root.path().join("outside.md");
        fs::write(&outside, "outside").unwrap();
        symlink(&outside, log.path()).unwrap();
        assert!(log.entries().is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "outside");
    }

    #[test]
    fn prompt_contains_the_candidate_and_exact_action_vocabulary() {
        let (_root, _log, ritual, _provenance) = setup();
        let prompt = ritual.formatted_prompt();
        assert!(prompt.contains(&ritual.nature().nature_id));
        assert!(prompt.contains("ENGAGEMENT"));
        assert!(prompt.contains("SACRED BAN"));
        assert!(prompt.contains("ADJUST <TRAIT> <DELTA>"));
        assert!(prompt.contains("REROLL"));
        assert!(prompt.contains("KILL"));
    }

    #[test]
    fn different_authenticated_operators_are_preserved_in_provenance() {
        let (_root, log, mut ritual, _first) = setup();
        let other =
            AwakeningProvenance::authenticated_xmtp(OTHER_OPERATOR, "other-message").unwrap();
        ritual
            .apply(AwakeningAction::Yes, 101, &other, &log)
            .unwrap();
        assert_eq!(log.entries().unwrap()[0].operator_id, OTHER_OPERATOR);
    }
}
