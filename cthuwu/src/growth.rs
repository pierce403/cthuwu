//! Durable referral attribution, onboarding bounties, and rate-limited recruitment prompts.
//!
//! This extends the canonical Acolyte Branding/referral path. Browser fragments and XMTP text are
//! untrusted hints: attribution is accepted only for an SDK-authenticated acolyte and a referrer
//! already known locally as an onboarded acolyte or authenticated operator. The first accepted
//! referrer is immutable. Rewards use the same typed ERC-8004 sidecar and signer nonce journal as
//! Branding; no model-facing transfer primitive exists.

use crate::{
    config::DEFAULT_UWU_TOKEN_CONTRACT,
    erc8004::{BASE_MAINNET_CHAIN_ID, Erc8004Gateway},
    storage::{ensure_private_directory, restrict_file, sync_directory},
    token_eye::Address,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
use tempfile::NamedTempFile;

/// One canonical UWU (18 decimals). This is deliberately conservative and may be changed only
/// through the single node policy setting, never through model or frontend input.
pub const DEFAULT_REFERRAL_BOUNTY_BASE_UNITS: &str = "1000000000000000000";
pub const DEFAULT_PUBLIC_ORIGIN: &str = "https://cthuwu.app";
pub const REFERRAL_CONTROL_PREFIX: &str = "[[cthuwu:referral-attribution:v1;";

pub(crate) fn validate_referral_bounty_amount(value: &str) -> Result<()> {
    validate_decimal(value, "referral bounty", true)
}

const SNAPSHOT_VERSION: u32 = 1;
const SNAPSHOT_FILE: &str = "acolyte-growth.json";
const MAX_SNAPSHOT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RECORDS: usize = 10_000;
const MAX_OPERATORS: usize = 64;
const RETRY_SECONDS: u64 = 60;
const FUNDING_RETRY_SECONDS: u64 = 5 * 60;
const FUNDING_NOTICE_COOLDOWN_SECONDS: u64 = 24 * 60 * 60;
const OPERATOR_PROMPT_COOLDOWN_SECONDS: u64 = 7 * 24 * 60 * 60;
const RECENT_ONBOARDING_SECONDS: u64 = 7 * 24 * 60 * 60;
const MAX_UINT256_DECIMAL: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639935";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrandingAttemptOutcome {
    ObserverUnavailable,
    WalletUnavailable,
    ClockUnavailable,
    RpcUnavailable,
    EmptyTreasury,
    QuoteInvalid,
    Queued,
    AlreadyQueued,
    QueueFailed,
}

impl BrandingAttemptOutcome {
    pub fn explanation(self) -> &'static str {
        match self {
            Self::ObserverUnavailable => {
                "UWU observation is disabled or not configured; enable token observation on the host"
            }
            Self::WalletUnavailable => {
                "the runtime has no bound XMTP treasury wallet; repair the node identity binding"
            }
            Self::ClockUnavailable => "the host clock is unavailable; repair the host clock",
            Self::RpcUnavailable => {
                "the fresh Base UWU balance read failed; restore the node's Base RPC access (a browser connection does not repair the node RPC)"
            }
            Self::EmptyTreasury => {
                "the verified Tentacle UWU treasury is empty; fund this Tentacle's UWU wallet"
            }
            Self::QuoteInvalid => {
                "the treasury balance cannot produce a valid positive Branding quote"
            }
            Self::Queued => {
                "a Branding invitation was queued; delivery, acolyte consent, and a confirmed mint are still required"
            }
            Self::AlreadyQueued => {
                "a Branding invitation is already queued; queued is not delivered or minted"
            }
            Self::QueueFailed => {
                "the Branding supervisor could not queue the invitation; inspect registration and supervisor state"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferralRewardPhase {
    NewContact,
    Direct,
    Attributed,
    OnboardingComplete,
    RewardPending,
    Submitted,
    Confirmed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DurableBrandingState {
    Declined,
    Branded,
    Inactive,
    Ineligible,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RewardReceipt {
    transaction_hash: String,
    transaction_nonce: String,
    block_number: String,
    block_hash: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum GrowthDeliveryTarget {
    ReferrerInbox,
    Operators,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PendingDelivery {
    target: GrowthDeliveryTarget,
    commitment: String,
    text: String,
    funding_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ReferralRecord {
    inbox_id: String,
    acolyte: String,
    referrer: Option<String>,
    #[serde(default)]
    referrer_inbox_id: Option<String>,
    phase: ReferralRewardPhase,
    #[serde(default)]
    branding_state: Option<DurableBrandingState>,
    #[serde(default)]
    branding_state_updated_at_unix: Option<u64>,
    reward_amount_base_units: String,
    reward_action_id: Option<String>,
    transaction_hash: Option<String>,
    transaction_nonce: Option<String>,
    receipt: Option<RewardReceipt>,
    pending_delivery: Option<PendingDelivery>,
    last_funding_fingerprint: Option<String>,
    last_funding_notice_unix: Option<u64>,
    next_attempt_unix: u64,
    attempt_count: u32,
    last_error: Option<String>,
    attributed_at_unix: Option<u64>,
    onboarding_completed_at_unix: Option<u64>,
    created_at_unix: u64,
    updated_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct VerifiedOperator {
    inbox_id: String,
    address: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct GrowthSnapshot {
    version: u32,
    records: Vec<ReferralRecord>,
    verified_operators: Vec<VerifiedOperator>,
    #[serde(default)]
    current_operator: Option<VerifiedOperator>,
    referrals_sent: u64,
    operator_prompt_variant: u8,
    #[serde(default)]
    operator_reminder_interval: Option<u64>,
    #[serde(default)]
    operator_snooze_until: u64,
    #[serde(default)]
    operator_quiet_hours: Option<(u8, u8)>,
    last_operator_prompt_unix: Option<u64>,
    pending_operator_prompt: Option<PendingDelivery>,
    #[serde(default)]
    last_branding_attempt: Option<(BrandingAttemptOutcome, u64)>,
    updated_at_unix: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrowthStats {
    pub total_acolytes: usize,
    pub branded: usize,
    pub unbranded: usize,
    pub recently_onboarded: usize,
    pub referrals_sent: u64,
    pub successful_referrals: usize,
    pub referral_uwu_paid_base_units: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrowthContext {
    pub is_acolyte: bool,
    pub immutable_referrer: Option<String>,
    pub referral_bounty_phase: Option<ReferralRewardPhase>,
    pub shareable_referral_url: Option<String>,
}

impl GrowthContext {
    pub fn runtime_facts(&self) -> String {
        let referrer = self.immutable_referrer.as_deref().unwrap_or("none");
        let phase = self
            .referral_bounty_phase
            .map(referral_phase_label)
            .unwrap_or_else(|| "none".to_owned());
        let url = self
            .shareable_referral_url
            .as_deref()
            .unwrap_or("not-yet-available");
        format!(
            "growth.is_acolyte={}\ngrowth.immutable_referrer={}\ngrowth.referral_bounty_phase={}\ngrowth.shareable_referral_url={}",
            self.is_acolyte, referrer, phase, url
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GrowthDeliveryAudience {
    Inbox(String),
    ActiveOperator(String),
    Operators,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrowthDelivery {
    pub audience: GrowthDeliveryAudience,
    pub text: String,
    pub commitment: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReferralBountyResult {
    kind: String,
    disposition: ReferralBountyDisposition,
    chain_id: u64,
    token: String,
    wallet: String,
    acolyte: String,
    referrer: String,
    amount_base_units: String,
    eth_balance_wei: String,
    eth_target_wei: String,
    eth_shortfall_wei: String,
    uwu_balance_base_units: String,
    uwu_target_base_units: String,
    uwu_shortfall_base_units: String,
    transaction_hash: Option<String>,
    transaction_nonce: Option<String>,
    receipt_block_number: Option<String>,
    receipt_block_hash: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ReferralBountyDisposition {
    FundingRequired,
    Submitted,
    Confirmed,
}

struct GrowthStore {
    directory: PathBuf,
    path: PathBuf,
}

impl GrowthStore {
    fn new(data_dir: &Path) -> Result<Self> {
        let directory = data_dir.join("state");
        ensure_private_directory(&directory)?;
        let path = directory.join(SNAPSHOT_FILE);
        reject_symlink(&path)?;
        Ok(Self { directory, path })
    }

    fn load_or_create(&self, minter: Address, reward: &str, now: u64) -> Result<GrowthSnapshot> {
        let bytes = match fs::metadata(&self.path) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_file() && metadata.len() <= MAX_SNAPSHOT_BYTES,
                    "growth snapshot must be a bounded regular file"
                );
                fs::read(&self.path)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let snapshot = GrowthSnapshot {
                    version: SNAPSHOT_VERSION,
                    records: Vec::new(),
                    verified_operators: Vec::new(),
                    current_operator: None,
                    referrals_sent: 0,
                    operator_prompt_variant: 0,
                    operator_reminder_interval: None,
                    operator_snooze_until: 0,
                    operator_quiet_hours: None,
                    last_operator_prompt_unix: Some(now),
                    last_branding_attempt: None,
                    pending_operator_prompt: None,
                    updated_at_unix: now,
                };
                self.save(&snapshot, minter, reward)?;
                return Ok(snapshot);
            }
            Err(error) => return Err(error.into()),
        };
        let mut snapshot: GrowthSnapshot =
            serde_json::from_slice(&bytes).context("growth snapshot is invalid")?;
        let mut migrated_policy = false;
        for record in &mut snapshot.records {
            if matches!(
                record.phase,
                ReferralRewardPhase::NewContact
                    | ReferralRewardPhase::Direct
                    | ReferralRewardPhase::Attributed
            ) && record.reward_amount_base_units != reward
            {
                record.reward_amount_base_units = reward.to_owned();
                migrated_policy = true;
            }
        }
        validate_snapshot(&snapshot, minter, reward)?;
        if migrated_policy {
            snapshot.updated_at_unix = now.max(snapshot.updated_at_unix);
            self.save(&snapshot, minter, reward)?;
        }
        Ok(snapshot)
    }

    fn save(&self, snapshot: &GrowthSnapshot, minter: Address, reward: &str) -> Result<()> {
        validate_snapshot(snapshot, minter, reward)?;
        reject_symlink(&self.path)?;
        let mut encoded = serde_json::to_vec_pretty(snapshot)?;
        encoded.push(b'\n');
        ensure!(
            encoded.len() as u64 <= MAX_SNAPSHOT_BYTES,
            "growth snapshot is oversized"
        );
        let mut temporary = NamedTempFile::new_in(&self.directory)?;
        restrict_file(temporary.as_file(), "temporary Acolyte growth snapshot")?;
        temporary.write_all(&encoded)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|error| error.error)?;
        sync_directory(&self.directory)
    }
}

pub(crate) struct GrowthRuntime {
    store: GrowthStore,
    state: GrowthSnapshot,
    minter: Address,
    reward_amount_base_units: String,
    public_origin: String,
    gateway: Arc<dyn Erc8004Gateway>,
}

impl GrowthRuntime {
    pub(crate) fn open(
        data_dir: &Path,
        minter: Address,
        reward_amount_base_units: &str,
        public_origin: &str,
        gateway: Arc<dyn Erc8004Gateway>,
        now: u64,
    ) -> Result<Self> {
        ensure!(minter != Address::ZERO, "growth treasury must be nonzero");
        validate_referral_bounty_amount(reward_amount_base_units)?;
        let public_origin = canonical_origin(public_origin)?;
        let store = GrowthStore::new(data_dir)?;
        let state = store.load_or_create(minter, reward_amount_base_units, now)?;
        Ok(Self {
            store,
            state,
            minter,
            reward_amount_base_units: reward_amount_base_units.to_owned(),
            public_origin,
            gateway,
        })
    }

    fn persist(&mut self, now: u64) -> Result<()> {
        self.state.updated_at_unix = now;
        self.store
            .save(&self.state, self.minter, &self.reward_amount_base_units)
    }

    pub(crate) fn mark_contact(
        &mut self,
        inbox_id: &str,
        authenticated_address: &str,
        now: u64,
    ) -> Result<bool> {
        validate_inbox(inbox_id)?;
        let acolyte = parse_nonzero_address(authenticated_address, "authenticated contact")?;
        let normalized_inbox = inbox_id.to_ascii_lowercase();
        let acolyte_string = acolyte.to_string();
        // The recovered XMTP inbox is the durable local onboarding identity. An inbox may have
        // more than one associated wallet, but rotating the authenticated address must never
        // create a second acolyte/reward record for the same onboarding conversation.
        if self.identity_conflicts(&normalized_inbox, acolyte) {
            return Ok(false);
        }
        if let Some(index) = self
            .state
            .records
            .iter()
            .position(|record| record.acolyte == acolyte_string)
        {
            if self.state.records[index].inbox_id != normalized_inbox {
                let record = &mut self.state.records[index];
                record.inbox_id = normalized_inbox;
                record.updated_at_unix = record.updated_at_unix.max(now);
                for referred in &mut self.state.records {
                    if referred.referrer.as_deref() == Some(acolyte_string.as_str())
                        && referred.inbox_id != inbox_id
                    {
                        referred.referrer_inbox_id = Some(inbox_id.to_owned());
                        referred.updated_at_unix = referred.updated_at_unix.max(now);
                    }
                }
                self.persist(now)?;
            }
            return Ok(false);
        }
        ensure!(
            self.state.records.len() < MAX_RECORDS,
            "growth record limit reached"
        );
        self.state.records.push(ReferralRecord {
            inbox_id: normalized_inbox,
            acolyte: acolyte_string,
            referrer: None,
            referrer_inbox_id: None,
            phase: ReferralRewardPhase::NewContact,
            branding_state: None,
            branding_state_updated_at_unix: None,
            reward_amount_base_units: self.reward_amount_base_units.clone(),
            reward_action_id: None,
            transaction_hash: None,
            transaction_nonce: None,
            receipt: None,
            pending_delivery: None,
            last_funding_fingerprint: None,
            last_funding_notice_unix: None,
            next_attempt_unix: u64::MAX,
            attempt_count: 0,
            last_error: None,
            attributed_at_unix: None,
            onboarding_completed_at_unix: None,
            created_at_unix: now,
            updated_at_unix: now,
        });
        self.persist(now)?;
        Ok(true)
    }

    pub(crate) fn handle_control(
        &mut self,
        inbox_id: &str,
        authenticated_sender: Option<&str>,
        text: &str,
        now: u64,
    ) -> Result<Option<String>> {
        let Some(raw_referrer) = parse_referral_control(text) else {
            if text
                .to_ascii_lowercase()
                .contains("[[cthuwu:referral-attribution:")
            {
                return Ok(Some("that referral control was malformed, so i pinned nothing and queued no reward, fwiend.".to_owned()));
            }
            return Ok(None);
        };
        let Some(sender) = authenticated_sender else {
            return Ok(Some("i could not bind that referral to an authenticated Ethereum sender, so i pinned nothing, fwiend.".to_owned()));
        };
        let acolyte = parse_nonzero_address(sender, "authenticated acolyte")?;
        let referrer = parse_nonzero_address(&raw_referrer, "referrer")?;
        validate_inbox(inbox_id)?;
        if self.identity_conflicts(inbox_id, acolyte) {
            return Ok(Some("this recovered XMTP identity is already pinned to another authenticated acolyte wallet, so changing associated wallets cannot create or replace referral attribution, fwiend.".to_owned()));
        }
        if acolyte == self.minter || acolyte == referrer || referrer == self.minter {
            return Ok(Some("that referral is not eligible for a bounty because self-referrals and the servicing Tentacle treasury are excluded, fwiend.".to_owned()));
        }
        if let Some(existing) = self.record(acolyte) {
            if existing.referrer.as_deref() == Some(referrer.to_string().as_str()) {
                return Ok(Some(referral_ack("accepted", Some(referrer))));
            }
            if let Some(existing_referrer) = &existing.referrer {
                let existing_referrer =
                    parse_nonzero_address(existing_referrer, "persisted referrer")?;
                return Ok(Some(format!(
                    "ur original referral is already pinned to {}. i refused the later replacement, uwu.\n{}",
                    short_address(existing_referrer),
                    referral_ack("immutable", Some(existing_referrer)),
                )));
            }
            if existing.onboarding_completed_at_unix.is_some() {
                return Ok(Some(format!(
                    "this acolyte already completed onboarding without a referral, so later links cannot manufacture a bounty, fwiend.\n{}",
                    referral_ack("direct", None),
                )));
            }
        }
        let Some(referrer_inbox_id) = self.verified_referrer_inbox(referrer) else {
            return Ok(Some("i could not verify that referral address as an established local acolyte or authenticated operator, so i pinned nothing and queued no payout, fwiend.".to_owned()));
        };
        if referrer_inbox_id == inbox_id {
            return Ok(Some("that referral resolves to this same authenticated XMTP inbox, so i treated it as an ineligible self-referral and queued no payout, fwiend.".to_owned()));
        }
        let acolyte_string = acolyte.to_string();
        if let Some(record) = self
            .state
            .records
            .iter_mut()
            .find(|record| record.acolyte == acolyte_string)
        {
            record.inbox_id = inbox_id.to_ascii_lowercase();
            record.referrer = Some(referrer.to_string());
            record.referrer_inbox_id = Some(referrer_inbox_id.clone());
            record.phase = ReferralRewardPhase::Attributed;
            record.attributed_at_unix = Some(now);
            record.updated_at_unix = now;
        } else {
            ensure!(
                self.state.records.len() < MAX_RECORDS,
                "growth record limit reached"
            );
            self.state.records.push(ReferralRecord {
                inbox_id: inbox_id.to_ascii_lowercase(),
                acolyte: acolyte_string,
                referrer: Some(referrer.to_string()),
                referrer_inbox_id: Some(referrer_inbox_id),
                phase: ReferralRewardPhase::Attributed,
                branding_state: None,
                branding_state_updated_at_unix: None,
                reward_amount_base_units: self.reward_amount_base_units.clone(),
                reward_action_id: None,
                transaction_hash: None,
                transaction_nonce: None,
                receipt: None,
                pending_delivery: None,
                last_funding_fingerprint: None,
                last_funding_notice_unix: None,
                next_attempt_unix: u64::MAX,
                attempt_count: 0,
                last_error: None,
                attributed_at_unix: Some(now),
                onboarding_completed_at_unix: None,
                created_at_unix: now,
                updated_at_unix: now,
            });
        }
        self.persist(now)?;
        Ok(Some(format!(
            "referral pinned to {} for this authenticated acolyte. it is immutable; the one-time UWU bounty becomes eligible only after canonical onboarding completes, uwu.\n{}",
            short_address(referrer),
            referral_ack("accepted", Some(referrer)),
        )))
    }

    pub(crate) fn register_operator(
        &mut self,
        inbox_id: &str,
        authenticated_address: &str,
        now: u64,
    ) -> Result<()> {
        validate_inbox(inbox_id)?;
        let address = parse_nonzero_address(authenticated_address, "authenticated operator")?;
        if address == self.minter {
            return Ok(());
        }
        let current = VerifiedOperator {
            inbox_id: inbox_id.to_ascii_lowercase(),
            address: address.to_string(),
        };
        if self.state.current_operator.as_ref() == Some(&current) {
            return Ok(());
        }
        let mut next = self.state.clone();
        // Historical verified referrals remain valid, while recruitment follows the active operator.
        next.verified_operators.retain(|operator| {
            operator.address != current.address && operator.inbox_id != current.inbox_id
        });
        ensure!(
            next.verified_operators.len() < MAX_OPERATORS,
            "verified operator referral limit reached"
        );
        next.verified_operators.push(current.clone());
        next.verified_operators
            .sort_by(|a, b| a.inbox_id.cmp(&b.inbox_id));
        for record in &mut next.records {
            if record.referrer.as_deref() == Some(current.address.as_str())
                && record.inbox_id != current.inbox_id
            {
                record.referrer_inbox_id = Some(current.inbox_id.clone());
                record.updated_at_unix = record.updated_at_unix.max(now);
            }
        }
        next.current_operator = Some(current);
        next.pending_operator_prompt = None;
        let previous = std::mem::replace(&mut self.state, next);
        if let Err(error) = self.persist(now) {
            self.state = previous;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn mark_onboarding_complete(
        &mut self,
        inbox_id: &str,
        authenticated_address: &str,
        now: u64,
    ) -> Result<bool> {
        validate_inbox(inbox_id)?;
        let acolyte = parse_nonzero_address(authenticated_address, "authenticated acolyte")?;
        if self.identity_conflicts(inbox_id, acolyte) {
            return Ok(false);
        }
        let acolyte_string = acolyte.to_string();
        if self.state.records.iter().any(|record| {
            record.acolyte == acolyte_string && record.onboarding_completed_at_unix.is_some()
        }) {
            return Ok(false);
        }
        let index = if let Some(index) = self
            .state
            .records
            .iter()
            .position(|record| record.acolyte == acolyte_string)
        {
            index
        } else {
            ensure!(
                self.state.records.len() < MAX_RECORDS,
                "growth record limit reached"
            );
            self.state.records.push(ReferralRecord {
                inbox_id: inbox_id.to_ascii_lowercase(),
                acolyte: acolyte_string.clone(),
                referrer: None,
                referrer_inbox_id: None,
                phase: ReferralRewardPhase::Direct,
                branding_state: None,
                branding_state_updated_at_unix: None,
                reward_amount_base_units: self.reward_amount_base_units.clone(),
                reward_action_id: None,
                transaction_hash: None,
                transaction_nonce: None,
                receipt: None,
                pending_delivery: None,
                last_funding_fingerprint: None,
                last_funding_notice_unix: None,
                next_attempt_unix: u64::MAX,
                attempt_count: 0,
                last_error: None,
                attributed_at_unix: None,
                onboarding_completed_at_unix: None,
                created_at_unix: now,
                updated_at_unix: now,
            });
            self.state.records.len() - 1
        };
        let referred = {
            let record = &mut self.state.records[index];
            record.inbox_id = inbox_id.to_ascii_lowercase();
            record.onboarding_completed_at_unix = Some(now);
            record.updated_at_unix = now;
            if record.referrer.is_some() {
                // Persist the terminal onboarding event before preparing any signer action.
                record.phase = ReferralRewardPhase::OnboardingComplete;
                record.next_attempt_unix = now;
            } else {
                record.phase = ReferralRewardPhase::Direct;
                record.next_attempt_unix = u64::MAX;
            }
            record.referrer.is_some()
        };
        if referred {
            // Replace stale unsent recruitment copy and make the next operator prompt use current
            // success statistics instead of repeating the prior nag.
            self.state.pending_operator_prompt = None;
            self.state.operator_prompt_variant =
                self.state.operator_prompt_variant.wrapping_add(1) % 3;
            self.state.last_operator_prompt_unix =
                Some(now.saturating_sub(OPERATOR_PROMPT_COOLDOWN_SECONDS));
        }
        self.persist(now)?;
        if referred {
            let record = &mut self.state.records[index];
            record.reward_action_id = Some(format!("referral-bounty:{}", address_key(acolyte)));
            record.phase = ReferralRewardPhase::RewardPending;
            record.next_attempt_unix = now;
            record.updated_at_unix = now;
            self.persist(now)?;
        }
        Ok(true)
    }

    pub(crate) fn immutable_referrer(&self, acolyte: Address) -> Option<Address> {
        self.record(acolyte)
            .and_then(|record| record.referrer.as_deref())
            .and_then(|value| Address::from_str(value).ok())
    }

    pub(crate) fn branding_state(&self, acolyte: Address) -> Option<DurableBrandingState> {
        self.record(acolyte)
            .and_then(|record| record.branding_state)
    }

    pub(crate) fn identity_conflicts(&self, inbox_id: &str, acolyte: Address) -> bool {
        let normalized_inbox = inbox_id.to_ascii_lowercase();
        let acolyte = acolyte.to_string();
        self.state
            .records
            .iter()
            .any(|record| record.inbox_id == normalized_inbox && record.acolyte != acolyte)
    }

    pub(crate) fn note_branding_state(
        &mut self,
        inbox_id: &str,
        acolyte: Address,
        state: DurableBrandingState,
        observed_at: u64,
    ) -> Result<bool> {
        validate_inbox(inbox_id)?;
        if self.record(acolyte).is_none() {
            let contact_created_at = self.state.updated_at_unix.max(observed_at);
            self.mark_contact(inbox_id, &acolyte.to_string(), contact_created_at)?;
        }
        let record = self
            .state
            .records
            .iter_mut()
            .find(|record| record.acolyte == acolyte.to_string())
            .context("Branding state contact disappeared")?;
        if record
            .branding_state_updated_at_unix
            .is_some_and(|existing| existing > observed_at)
            || (record.branding_state == Some(state)
                && record.branding_state_updated_at_unix == Some(observed_at))
            || (record.branding_state == Some(DurableBrandingState::Branded)
                && state == DurableBrandingState::Declined)
        {
            return Ok(false);
        }
        record.inbox_id = inbox_id.to_ascii_lowercase();
        record.branding_state = Some(state);
        record.branding_state_updated_at_unix = Some(observed_at);
        record.updated_at_unix = record.updated_at_unix.max(observed_at);
        let persisted_at = self.state.updated_at_unix.max(observed_at);
        self.persist(persisted_at)?;
        Ok(true)
    }

    pub(crate) fn branded_count(&self) -> usize {
        self.state
            .records
            .iter()
            .filter(|record| {
                record.onboarding_completed_at_unix.is_some()
                    && record.branding_state == Some(DurableBrandingState::Branded)
            })
            .count()
    }

    pub(crate) fn context(&self, acolyte: Address) -> GrowthContext {
        let record = self.record(acolyte);
        let is_acolyte = record.is_some_and(|record| record.onboarding_completed_at_unix.is_some());
        GrowthContext {
            is_acolyte,
            immutable_referrer: record.and_then(|record| record.referrer.clone()),
            referral_bounty_phase: record.map(|record| record.phase),
            shareable_referral_url: is_acolyte.then(|| self.referral_url(acolyte)),
        }
    }

    pub(crate) fn note_referral_link_sent(&mut self, now: u64) -> Result<()> {
        self.state.referrals_sent = self.state.referrals_sent.saturating_add(1);
        self.persist(now)
    }

    pub(crate) fn referral_url(&self, referrer: Address) -> String {
        format!("{}/#t={}&r={}", self.public_origin, self.minter, referrer)
    }

    pub(crate) fn stats(&self, branded: usize, now: u64) -> GrowthStats {
        let total_acolytes = self
            .state
            .records
            .iter()
            .filter(|record| record.onboarding_completed_at_unix.is_some())
            .count();
        let successful_referrals = self
            .state
            .records
            .iter()
            .filter(|record| {
                record.referrer.is_some() && record.onboarding_completed_at_unix.is_some()
            })
            .count();
        let referral_uwu_paid_base_units = self
            .state
            .records
            .iter()
            .filter(|record| record.phase == ReferralRewardPhase::Confirmed)
            .fold("0".to_owned(), |total, record| {
                decimal_add(&total, &record.reward_amount_base_units)
            });
        GrowthStats {
            total_acolytes,
            branded: branded.min(total_acolytes),
            unbranded: total_acolytes.saturating_sub(branded),
            recently_onboarded: self
                .state
                .records
                .iter()
                .filter(|record| {
                    record
                        .onboarding_completed_at_unix
                        .is_some_and(|completed| {
                            now.saturating_sub(completed) <= RECENT_ONBOARDING_SECONDS
                        })
                })
                .count(),
            referrals_sent: self.state.referrals_sent,
            successful_referrals,
            referral_uwu_paid_base_units,
        }
    }

    pub(crate) fn operator_status(
        &mut self,
        operator_address: Address,
        branded: usize,
        now: u64,
    ) -> Result<String> {
        let stats = self.stats(branded, now);
        let link = self.referral_url(operator_address);
        self.note_referral_link_sent(now)?;
        Ok(format!(
            "GROWTH STATUS\n- TOTAL ACOLYTES: {}\n- BRANDED / UNBRANDED: {} / {}\n- RECENTLY ONBOARDED (7D): {}\n- REFERRAL LINKS SENT: {}\n- SUCCESSFUL REFERRED ONBOARDINGS: {}\n- REFERRAL UWU PAID (BASE UNITS): {}\n- OPERATOR RECRUITMENT LINK: {}\n\nSHARE THIS WITH ONE OR TWO PEOPLE WHO WOULD ACTUALLY VALUE A VOLUNTARY CTHUWU ACOLYTE RELATIONSHIP. DO NOT SPAM THEM, UWU.",
            stats.total_acolytes,
            stats.branded,
            stats.unbranded,
            stats.recently_onboarded,
            stats.referrals_sent,
            stats.successful_referrals,
            stats.referral_uwu_paid_base_units,
            link,
        ))
    }

    pub(crate) fn record_branding_attempt(
        &mut self,
        outcome: BrandingAttemptOutcome,
        now: u64,
    ) -> Result<()> {
        let previous = self.state.last_branding_attempt.replace((outcome, now));
        if let Err(error) = self.persist(now) {
            self.state.last_branding_attempt = previous;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn operator_runtime_facts(
        &self,
        operator_address: Address,
        branded: usize,
        now: u64,
    ) -> String {
        let stats = self.stats(branded, now);
        let last_attempt = self.state.last_branding_attempt.map(|(outcome, at)|
            format!("growth.last_branding_attempt={outcome:?}\ngrowth.last_branding_attempt_unix={at}\ngrowth.last_branding_attempt_detail={}", outcome.explanation())
        ).unwrap_or_else(|| "growth.last_branding_attempt=not_recorded".to_owned());
        format!(
            "{last_attempt}\ngrowth.total_acolytes={}\ngrowth.branded={}\ngrowth.unbranded={}\ngrowth.recently_onboarded_7d={}\ngrowth.referral_links_sent={}\ngrowth.successful_referrals={}\ngrowth.referral_uwu_paid_base_units={}\ngrowth.referral_bounty_amount_base_units={}\ngrowth.operator_recruitment_url={}",
            stats.total_acolytes,
            stats.branded,
            stats.unbranded,
            stats.recently_onboarded,
            stats.referrals_sent,
            stats.successful_referrals,
            stats.referral_uwu_paid_base_units,
            self.reward_amount_base_units,
            self.referral_url(operator_address),
        )
    }

    pub(crate) fn reminder_preferences(&mut self, arguments: &str, now: u64) -> Result<String> {
        let mut next = self.state.clone();
        match arguments.trim() {
            "daily" => next.operator_reminder_interval = Some(86400),
            "weekly" => next.operator_reminder_interval = Some(7 * 86400),
            "off" => next.operator_reminder_interval = Some(0),
            "quiet off" => next.operator_quiet_hours = None,
            value if value.starts_with("snooze ") => {
                let days: u64 = value[7..].trim().parse()?;
                ensure!((1..=365).contains(&days), "snooze must be 1–365 days");
                next.operator_snooze_until = now.saturating_add(days * 86400);
            }
            value if value.starts_with("quiet ") => {
                let hours = value[6..]
                    .split_whitespace()
                    .map(str::parse::<u8>)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                ensure!(
                    hours.len() == 2 && hours[0] < 24 && hours[1] < 24 && hours[0] != hours[1],
                    "quiet hours require different UTC hours 0–23"
                );
                next.operator_quiet_hours = Some((hours[0], hours[1]));
            }
            _ => bail!(
                "usage: /referrals daily|weekly|off|snooze <days>|quiet <startUTC> <endUTC>|quiet off"
            ),
        }
        next.pending_operator_prompt = None;
        let previous = std::mem::replace(&mut self.state, next);
        if let Err(error) = self.persist(now) {
            self.state = previous;
            return Err(error);
        }
        Ok("Referral reminder preferences saved. Quiet hours use UTC. Use /referrals daily, weekly, off, or snooze <days>.".into())
    }

    fn reminders_allowed(&self, now: u64) -> bool {
        let hour = ((now / 3600) % 24) as u8;
        let quiet = self.state.operator_quiet_hours.is_some_and(|(start, end)| {
            if start < end {
                hour >= start && hour < end
            } else {
                hour >= start || hour < end
            }
        });
        !quiet
            && self.state.operator_snooze_until <= now
            && self.state.operator_reminder_interval != Some(0)
    }

    pub(crate) fn has_due(&self, now: u64) -> bool {
        (self.state.pending_operator_prompt.is_some() && self.reminders_allowed(now))
            || self.state.records.iter().any(|record| {
                matches!(
                    record.phase,
                    ReferralRewardPhase::OnboardingComplete
                        | ReferralRewardPhase::RewardPending
                        | ReferralRewardPhase::Submitted
                ) && record.next_attempt_unix <= now
                    || record.pending_delivery.is_some()
            })
            || (self.state.current_operator.is_some()
                && self.state.last_operator_prompt_unix.is_some_and(|last| {
                    self.reminders_allowed(now)
                        && now.saturating_sub(last)
                            >= self
                                .state
                                .operator_reminder_interval
                                .unwrap_or(OPERATOR_PROMPT_COOLDOWN_SECONDS)
                }))
    }

    pub(crate) fn defer_due(&mut self, now: u64) -> Result<()> {
        for record in &mut self.state.records {
            if matches!(
                record.phase,
                ReferralRewardPhase::OnboardingComplete
                    | ReferralRewardPhase::RewardPending
                    | ReferralRewardPhase::Submitted
            ) && record.next_attempt_unix <= now
            {
                record.next_attempt_unix = now.saturating_add(RETRY_SECONDS);
            }
        }
        self.persist(now)
    }

    pub(crate) async fn maintain_one(
        &mut self,
        branded: usize,
        now: u64,
    ) -> Result<Option<GrowthDelivery>> {
        if let Some(delivery) = &self.state.pending_operator_prompt
            && let Some(operator) = &self.state.current_operator
            && self.reminders_allowed(now)
        {
            return Ok(Some(GrowthDelivery {
                audience: GrowthDeliveryAudience::ActiveOperator(operator.inbox_id.clone()),
                text: delivery.text.clone(),
                commitment: delivery.commitment.clone(),
            }));
        }
        if let Some((index, delivery)) =
            self.state
                .records
                .iter()
                .enumerate()
                .find_map(|(index, record)| {
                    record.pending_delivery.as_ref().map(|value| (index, value))
                })
        {
            return Ok(Some(GrowthDelivery {
                audience: match delivery.target {
                    GrowthDeliveryTarget::ReferrerInbox => {
                        GrowthDeliveryAudience::Inbox(self.referrer_inbox(index)?)
                    }
                    GrowthDeliveryTarget::Operators => GrowthDeliveryAudience::Operators,
                },
                text: delivery.text.clone(),
                commitment: delivery.commitment.clone(),
            }));
        }
        if let Some(index) = self.state.records.iter().position(|record| {
            matches!(
                record.phase,
                ReferralRewardPhase::OnboardingComplete
                    | ReferralRewardPhase::RewardPending
                    | ReferralRewardPhase::Submitted
            ) && record.next_attempt_unix <= now
        }) {
            if self.state.records[index].phase == ReferralRewardPhase::OnboardingComplete {
                let acolyte =
                    parse_nonzero_address(&self.state.records[index].acolyte, "persisted acolyte")?;
                let record = &mut self.state.records[index];
                record.reward_action_id = Some(format!("referral-bounty:{}", address_key(acolyte)));
                record.phase = ReferralRewardPhase::RewardPending;
                record.next_attempt_unix = now;
                record.updated_at_unix = now;
                self.persist(now)?;
                return Ok(None);
            }
            return self.reconcile_reward(index, now).await;
        }
        if self.state.current_operator.is_some()
            && self.state.last_operator_prompt_unix.is_some_and(|last| {
                self.reminders_allowed(now)
                    && now.saturating_sub(last)
                        >= self
                            .state
                            .operator_reminder_interval
                            .unwrap_or(OPERATOR_PROMPT_COOLDOWN_SECONDS)
            })
        {
            let operator = parse_nonzero_address(
                &self
                    .state
                    .current_operator
                    .as_ref()
                    .context("current operator is missing")?
                    .address,
                "persisted operator",
            )?;
            let stats = self.stats(branded, now);
            let variants = [
                "SHARE THIS LINK WITH ONE PERSON WHO MIGHT ENJOY A VOLUNTARY ACOLYTE RELATIONSHIP",
                "PICK ONE RELEVANT COMMUNITY OR FRIEND—NOT A BLAST LIST—AND OFFER THIS INVITE",
                "ASK ONE SATISFIED ACOLYTE WHO ELSE WOULD BENEFIT, THEN GIVE THEM THIS LINK",
            ];
            let variant = usize::from(self.state.operator_prompt_variant) % variants.len();
            let text = format!(
                "RECRUITMENT OBJECTIVE · {}.\n{}\nCURRENT FUNNEL: {} TOTAL, {} BRANDED, {} UNBRANDED, {} NEW IN 7D, {} SUCCESSFUL REFERRALS, {} UWU BASE UNITS PAID.\n{}",
                variants[variant],
                self.referral_url(operator),
                stats.total_acolytes,
                stats.branded,
                stats.unbranded,
                stats.recently_onboarded,
                stats.successful_referrals,
                stats.referral_uwu_paid_base_units,
                "KEEP IT SPECIFIC AND CONSENSUAL; DO NOT SPAM OR PRESSURE ANYONE, UWU. Use /referrals off or /referrals snooze 7 to pause. Optional: keep a dedicated backup model key with /env add VENICE_API_KEY backup <key>; /env list shows redacted slots.",
            );
            let commitment = commitment("operator-prompt", &text);
            self.state.pending_operator_prompt = Some(PendingDelivery {
                target: GrowthDeliveryTarget::Operators,
                commitment: commitment.clone(),
                text: text.clone(),
                funding_fingerprint: None,
            });
            self.persist(now)?;
            return Ok(Some(GrowthDelivery {
                audience: GrowthDeliveryAudience::ActiveOperator(
                    self.state
                        .current_operator
                        .as_ref()
                        .context("current operator is missing")?
                        .inbox_id
                        .clone(),
                ),
                text,
                commitment,
            }));
        }
        Ok(None)
    }

    async fn reconcile_reward(&mut self, index: usize, now: u64) -> Result<Option<GrowthDelivery>> {
        let record = self.state.records[index].clone();
        let referrer = record
            .referrer
            .as_deref()
            .context("reward record has no referrer")?;
        let action_id = record
            .reward_action_id
            .as_deref()
            .context("reward record has no action ID")?;
        let result = self
            .gateway
            .invoke(
                action_id,
                json!({
                    "type": "referral_bounty",
                    "wallet": self.minter,
                    "acolyte": record.acolyte,
                    "referrer": referrer,
                    "token": DEFAULT_UWU_TOKEN_CONTRACT,
                    "chainId": BASE_MAINNET_CHAIN_ID,
                    "amountBaseUnits": record.reward_amount_base_units,
                    "configuredAmountBaseUnits": self.reward_amount_base_units,
                    "transactionHash": record.transaction_hash,
                    "transactionNonce": record.transaction_nonce,
                }),
            )
            .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                let current = &mut self.state.records[index];
                current.attempt_count = current.attempt_count.saturating_add(1);
                current.last_error = Some(bounded(&error.to_string(), 512));
                current.next_attempt_unix = now.saturating_add(RETRY_SECONDS);
                current.updated_at_unix = now;
                self.persist(now)?;
                return Ok(None);
            }
        };
        let result: ReferralBountyResult = serde_json::from_value(result)
            .context("referral bounty helper returned an invalid result")?;
        validate_result(
            &result,
            &record,
            self.minter,
            &self.reward_amount_base_units,
        )?;
        match result.disposition {
            ReferralBountyDisposition::FundingRequired => {
                let fingerprint = commitment(
                    "funding",
                    &format!(
                        "{}:{}:{}:{}",
                        result.eth_shortfall_wei,
                        result.uwu_shortfall_base_units,
                        result.eth_target_wei,
                        result.uwu_target_base_units
                    ),
                );
                let notify = record.last_funding_fingerprint.as_deref() != Some(&fingerprint)
                    || record.last_funding_notice_unix.is_none_or(|last| {
                        now.saturating_sub(last) >= FUNDING_NOTICE_COOLDOWN_SECONDS
                    });
                let current = &mut self.state.records[index];
                current.phase = ReferralRewardPhase::RewardPending;
                current.attempt_count = current.attempt_count.saturating_add(1);
                current.last_error = None;
                current.updated_at_unix = now;
                if notify {
                    let text = format!(
                        "REFERRAL BOUNTY FUNDING REQUIRED. FUND TENTACLE WALLET {} WITH EXACTLY {} MORE UWU BASE UNITS AT CANONICAL {} AND AT LEAST {} MORE BASE ETH WEI. THE DURABLE BOUNTY FOR ACOLYTE {} -> REFERRER {} REMAINS PENDING AND WILL RESUME AUTOMATICALLY, UWU.",
                        self.minter,
                        result.uwu_shortfall_base_units,
                        DEFAULT_UWU_TOKEN_CONTRACT,
                        result.eth_shortfall_wei,
                        short_address(parse_nonzero_address(&record.acolyte, "acolyte")?),
                        short_address(parse_nonzero_address(referrer, "referrer")?),
                    );
                    let delivery_commitment = commitment("reward-funding", &text);
                    current.pending_delivery = Some(PendingDelivery {
                        target: GrowthDeliveryTarget::Operators,
                        commitment: delivery_commitment.clone(),
                        text: text.clone(),
                        funding_fingerprint: Some(fingerprint),
                    });
                    current.next_attempt_unix = now;
                    self.persist(now)?;
                    return Ok(Some(GrowthDelivery {
                        audience: GrowthDeliveryAudience::Operators,
                        text,
                        commitment: delivery_commitment,
                    }));
                }
                current.next_attempt_unix = now.saturating_add(FUNDING_RETRY_SECONDS);
                self.persist(now)?;
                Ok(None)
            }
            ReferralBountyDisposition::Submitted => {
                let current = &mut self.state.records[index];
                current.phase = ReferralRewardPhase::Submitted;
                current.transaction_hash = result.transaction_hash;
                current.transaction_nonce = result.transaction_nonce;
                current.attempt_count = current.attempt_count.saturating_add(1);
                current.next_attempt_unix = now.saturating_add(RETRY_SECONDS);
                current.last_error = None;
                current.updated_at_unix = now;
                self.persist(now)?;
                Ok(None)
            }
            ReferralBountyDisposition::Confirmed => {
                let receipt = RewardReceipt {
                    transaction_hash: result
                        .transaction_hash
                        .context("confirmed bounty has no transaction hash")?,
                    transaction_nonce: result
                        .transaction_nonce
                        .context("confirmed bounty has no transaction nonce")?,
                    block_number: result
                        .receipt_block_number
                        .context("confirmed bounty has no receipt block")?,
                    block_hash: result
                        .receipt_block_hash
                        .context("confirmed bounty has no receipt block hash")?,
                };
                let referrer_address = parse_nonzero_address(referrer, "referrer")?;
                let referrer_inbox = self.referrer_inbox(index)?;
                let text = format!(
                    "ur one-time {} UWU referral bounty is confirmed because acolyte {} completed canonical onboarding, fwiend! thank u for growing the network. share ur invite again when it feels natural: {}\n[[cthuwu:referral-reward:v1;status=confirmed;amount={}]]",
                    format_uwu_base_units(&record.reward_amount_base_units),
                    short_address(parse_nonzero_address(&record.acolyte, "acolyte")?),
                    self.referral_url(referrer_address),
                    record.reward_amount_base_units,
                );
                let delivery_commitment = commitment("reward-confirmed", &text);
                let current = &mut self.state.records[index];
                current.phase = ReferralRewardPhase::Confirmed;
                current.transaction_hash = Some(receipt.transaction_hash.clone());
                current.transaction_nonce = Some(receipt.transaction_nonce.clone());
                current.receipt = Some(receipt);
                current.pending_delivery = Some(PendingDelivery {
                    target: GrowthDeliveryTarget::ReferrerInbox,
                    commitment: delivery_commitment.clone(),
                    text: text.clone(),
                    funding_fingerprint: None,
                });
                current.attempt_count = current.attempt_count.saturating_add(1);
                current.next_attempt_unix = u64::MAX;
                current.last_error = None;
                current.updated_at_unix = now;
                self.persist(now)?;
                Ok(Some(GrowthDelivery {
                    audience: GrowthDeliveryAudience::Inbox(referrer_inbox),
                    text,
                    commitment: delivery_commitment,
                }))
            }
        }
    }

    pub(crate) fn acknowledge_delivery(
        &mut self,
        commitment_value: &str,
        delivered: bool,
        now: u64,
    ) -> Result<()> {
        if self
            .state
            .pending_operator_prompt
            .as_ref()
            .is_some_and(|delivery| delivery.commitment == commitment_value)
        {
            if delivered {
                self.state.pending_operator_prompt = None;
                self.state.last_operator_prompt_unix = Some(now);
                self.state.operator_prompt_variant =
                    self.state.operator_prompt_variant.wrapping_add(1) % 3;
                self.state.referrals_sent = self.state.referrals_sent.saturating_add(1);
            }
            return self.persist(now);
        }
        let Some(index) = self.state.records.iter().position(|record| {
            record
                .pending_delivery
                .as_ref()
                .is_some_and(|delivery| delivery.commitment == commitment_value)
        }) else {
            return Ok(());
        };
        if delivered
            && let Some(delivery) = self.state.records[index].pending_delivery.take()
            && let Some(fingerprint) = delivery.funding_fingerprint
        {
            self.state.records[index].last_funding_fingerprint = Some(fingerprint);
            self.state.records[index].last_funding_notice_unix = Some(now);
            self.state.records[index].next_attempt_unix = now.saturating_add(FUNDING_RETRY_SECONDS);
        }
        self.persist(now)
    }

    fn record(&self, acolyte: Address) -> Option<&ReferralRecord> {
        let acolyte = acolyte.to_string();
        self.state
            .records
            .iter()
            .find(|record| record.acolyte == acolyte)
    }

    fn verified_referrer_inbox(&self, referrer: Address) -> Option<String> {
        let referrer = referrer.to_string();
        self.state
            .records
            .iter()
            .find(|record| {
                record.acolyte == referrer && record.onboarding_completed_at_unix.is_some()
            })
            .map(|record| record.inbox_id.clone())
            .or_else(|| {
                self.state
                    .verified_operators
                    .iter()
                    .find(|operator| operator.address == referrer)
                    .map(|operator| operator.inbox_id.clone())
            })
    }

    fn referrer_inbox(&self, index: usize) -> Result<String> {
        self.state
            .records
            .get(index)
            .and_then(|record| record.referrer_inbox_id.clone())
            .context("referral confirmation has no durably verified referrer inbox")
    }
}

fn validate_snapshot(snapshot: &GrowthSnapshot, minter: Address, reward: &str) -> Result<()> {
    ensure!(
        snapshot.version == SNAPSHOT_VERSION,
        "unsupported growth snapshot version"
    );
    ensure!(
        snapshot.records.len() <= MAX_RECORDS,
        "growth record limit exceeded"
    );
    ensure!(
        snapshot.verified_operators.len() <= MAX_OPERATORS,
        "growth operator limit exceeded"
    );
    ensure!(
        snapshot.operator_prompt_variant < 3,
        "invalid operator prompt variant"
    );
    if let Some(operator) = &snapshot.current_operator {
        ensure!(
            snapshot.verified_operators.contains(operator),
            "current operator is not verified"
        );
    }
    if let Some(delivery) = &snapshot.pending_operator_prompt {
        ensure!(
            delivery.target == GrowthDeliveryTarget::Operators
                && delivery.funding_fingerprint.is_none()
                && delivery.commitment == commitment("operator-prompt", &delivery.text),
            "persisted operator recruitment prompt is invalid"
        );
    }
    let mut acolytes = BTreeSet::new();
    let mut operator_inboxes = BTreeSet::new();
    let mut operator_addresses = BTreeSet::new();
    let mut acolyte_inboxes = BTreeSet::new();
    let mut action_ids = BTreeSet::new();
    for operator in &snapshot.verified_operators {
        validate_inbox(&operator.inbox_id)?;
        let address = parse_nonzero_address(&operator.address, "persisted operator")?;
        ensure!(
            address != minter,
            "operator referrer is the Tentacle treasury"
        );
        ensure!(
            operator_addresses.insert(operator.address.clone()),
            "duplicate operator payout address"
        );
        ensure!(
            operator_inboxes.insert(operator.inbox_id.clone()),
            "duplicate operator inbox"
        );
    }
    for record in &snapshot.records {
        validate_inbox(&record.inbox_id)?;
        ensure!(
            acolyte_inboxes.insert(record.inbox_id.clone()),
            "one XMTP inbox cannot create multiple acolyte reward records"
        );
        let acolyte = parse_nonzero_address(&record.acolyte, "persisted acolyte")?;
        ensure!(
            acolytes.insert(record.acolyte.clone()),
            "duplicate acolyte reward record"
        );
        validate_decimal(&record.reward_amount_base_units, "persisted reward", true)?;
        if let Some(referrer) = &record.referrer {
            let referrer = parse_nonzero_address(referrer, "persisted referrer")?;
            ensure!(
                acolyte != minter && referrer != acolyte && referrer != minter,
                "ineligible persisted referral participants"
            );
            ensure!(
                record.attributed_at_unix.is_some(),
                "referral has no attribution time"
            );
        }
        ensure!(
            record.referrer.is_some() == record.referrer_inbox_id.is_some(),
            "persisted referral inbox binding is incomplete"
        );
        if let Some(referrer_inbox_id) = &record.referrer_inbox_id {
            validate_inbox(referrer_inbox_id)?;
            ensure!(
                referrer_inbox_id != &record.inbox_id,
                "persisted referral resolves to the acolyte inbox"
            );
        }
        if let Some(action_id) = &record.reward_action_id {
            ensure!(
                action_id == &format!("referral-bounty:{}", address_key(acolyte)),
                "reward action is not bound to the acolyte"
            );
            ensure!(
                action_ids.insert(action_id.clone()),
                "duplicate reward action ID"
            );
        }
        if let Some(hash) = &record.transaction_hash {
            validate_hash(hash, "transaction hash")?;
        }
        if let Some(nonce) = &record.transaction_nonce {
            validate_decimal(nonce, "transaction nonce", false)?;
        }
        ensure!(
            record.transaction_hash.is_some() == record.transaction_nonce.is_some(),
            "persisted reward transaction binding is incomplete"
        );
        if let Some(receipt) = &record.receipt {
            validate_hash(&receipt.transaction_hash, "receipt transaction hash")?;
            validate_decimal(&receipt.transaction_nonce, "receipt nonce", false)?;
            validate_decimal(&receipt.block_number, "receipt block", false)?;
            validate_hash(&receipt.block_hash, "receipt block hash")?;
            ensure!(
                record.phase == ReferralRewardPhase::Confirmed
                    && record.transaction_hash.as_deref()
                        == Some(receipt.transaction_hash.as_str())
                    && record.transaction_nonce.as_deref()
                        == Some(receipt.transaction_nonce.as_str()),
                "reward receipt does not match its confirmed transaction"
            );
        }
        if let Some(delivery) = &record.pending_delivery {
            ensure!(
                !delivery.text.is_empty() && delivery.text.len() <= 16 * 1024,
                "growth delivery is empty or oversized"
            );
            if let Some(fingerprint) = &delivery.funding_fingerprint {
                validate_commitment(fingerprint, "funding fingerprint")?;
                ensure!(
                    delivery.target == GrowthDeliveryTarget::Operators
                        && record.phase == ReferralRewardPhase::RewardPending
                        && delivery.commitment == commitment("reward-funding", &delivery.text),
                    "persisted funding notice is invalid"
                );
            } else {
                ensure!(
                    delivery.target == GrowthDeliveryTarget::ReferrerInbox
                        && record.phase == ReferralRewardPhase::Confirmed
                        && delivery.commitment == commitment("reward-confirmed", &delivery.text),
                    "persisted referral confirmation delivery is invalid"
                );
            }
        }
        ensure!(
            record.updated_at_unix >= record.created_at_unix,
            "growth timestamps inverted"
        );
        if let Some(error) = &record.last_error {
            ensure!(error.len() <= 512, "growth error is oversized");
        }
        ensure!(
            record.last_funding_fingerprint.is_some() == record.last_funding_notice_unix.is_some(),
            "persisted referral funding acknowledgement is incomplete"
        );
        if let Some(fingerprint) = &record.last_funding_fingerprint {
            validate_commitment(fingerprint, "delivered funding fingerprint")?;
        }
        match record.phase {
            ReferralRewardPhase::NewContact => ensure!(
                record.referrer.is_none()
                    && record.onboarding_completed_at_unix.is_none()
                    && record.reward_action_id.is_none()
                    && record.transaction_hash.is_none()
                    && record.receipt.is_none(),
                "new-contact growth record is invalid"
            ),
            ReferralRewardPhase::Direct => ensure!(
                record.referrer.is_none()
                    && record.onboarding_completed_at_unix.is_some()
                    && record.reward_action_id.is_none()
                    && record.transaction_hash.is_none()
                    && record.receipt.is_none(),
                "direct onboarding record is invalid"
            ),
            ReferralRewardPhase::Attributed => ensure!(
                record.referrer.is_some()
                    && record.onboarding_completed_at_unix.is_none()
                    && record.reward_action_id.is_none()
                    && record.transaction_hash.is_none()
                    && record.receipt.is_none(),
                "attributed record is invalid"
            ),
            ReferralRewardPhase::OnboardingComplete => ensure!(
                record.referrer.is_some()
                    && record.onboarding_completed_at_unix.is_some()
                    && record.reward_action_id.is_none()
                    && record.transaction_hash.is_none()
                    && record.receipt.is_none()
                    && record.next_attempt_unix != u64::MAX,
                "onboarding-complete reward is invalid"
            ),
            ReferralRewardPhase::RewardPending => ensure!(
                record.referrer.is_some()
                    && record.onboarding_completed_at_unix.is_some()
                    && record.reward_action_id.is_some()
                    && record.transaction_hash.is_none()
                    && record.receipt.is_none(),
                "pending referral reward is invalid"
            ),
            ReferralRewardPhase::Submitted => ensure!(
                record.referrer.is_some()
                    && record.onboarding_completed_at_unix.is_some()
                    && record.reward_action_id.is_some()
                    && record.transaction_hash.is_some()
                    && record.receipt.is_none(),
                "submitted referral reward is invalid"
            ),
            ReferralRewardPhase::Confirmed => ensure!(
                record.referrer.is_some()
                    && record.onboarding_completed_at_unix.is_some()
                    && record.reward_action_id.is_some()
                    && record.receipt.is_some(),
                "confirmed referral reward is invalid"
            ),
        }
        if record.phase != ReferralRewardPhase::Confirmed {
            ensure!(
                record.reward_amount_base_units == reward,
                "reward policy changed while an onboarding bounty is pending"
            );
        }
        ensure!(
            record.branding_state.is_some() == record.branding_state_updated_at_unix.is_some(),
            "persisted terminal Branding state is incomplete"
        );
    }
    Ok(())
}

fn parse_referral_control(text: &str) -> Option<String> {
    if text.trim() != text || text.contains(['\n', '\r']) {
        return None;
    }
    let body = text
        .strip_prefix(REFERRAL_CONTROL_PREFIX)?
        .strip_suffix("]]")?;
    let referrer = body.strip_prefix("referrer=")?;
    if referrer.contains(';') {
        return None;
    }
    Address::from_str(referrer)
        .ok()
        .and_then(|address| (address != Address::ZERO).then(|| address.to_string()))
}

fn validate_result(
    result: &ReferralBountyResult,
    record: &ReferralRecord,
    minter: Address,
    reward: &str,
) -> Result<()> {
    ensure!(
        result.kind == "referral_bounty",
        "unexpected bounty result kind"
    );
    ensure!(
        result.chain_id == BASE_MAINNET_CHAIN_ID,
        "bounty result is not Base mainnet"
    );
    ensure!(
        parse_nonzero_address(&result.token, "bounty result token")?
            == parse_nonzero_address(DEFAULT_UWU_TOKEN_CONTRACT, "canonical UWU")?,
        "bounty result uses another token"
    );
    ensure!(
        parse_nonzero_address(&result.wallet, "bounty result treasury")? == minter,
        "bounty result uses another treasury"
    );
    ensure!(
        parse_nonzero_address(&result.acolyte, "bounty result acolyte")?
            == parse_nonzero_address(&record.acolyte, "persisted acolyte")?,
        "bounty result acolyte mismatch"
    );
    ensure!(
        parse_nonzero_address(&result.referrer, "bounty result referrer")?
            == parse_nonzero_address(
                record.referrer.as_deref().unwrap_or_default(),
                "persisted referrer",
            )?,
        "bounty result referrer mismatch"
    );
    ensure!(
        result.amount_base_units == reward,
        "bounty result amount mismatch"
    );
    for (value, label) in [
        (&result.eth_balance_wei, "ETH balance"),
        (&result.eth_target_wei, "ETH target"),
        (&result.eth_shortfall_wei, "ETH shortfall"),
        (&result.uwu_balance_base_units, "UWU balance"),
        (&result.uwu_target_base_units, "UWU target"),
        (&result.uwu_shortfall_base_units, "UWU shortfall"),
    ] {
        validate_decimal(value, label, false)?;
    }
    ensure!(
        result.uwu_target_base_units == reward,
        "bounty result UWU target differs from policy"
    );
    ensure!(
        result.eth_shortfall_wei
            == decimal_shortfall(&result.eth_balance_wei, &result.eth_target_wei),
        "bounty result ETH shortfall is inconsistent"
    );
    ensure!(
        result.uwu_shortfall_base_units
            == decimal_shortfall(
                &result.uwu_balance_base_units,
                &result.uwu_target_base_units,
            ),
        "bounty result UWU shortfall is inconsistent"
    );
    match result.disposition {
        ReferralBountyDisposition::FundingRequired => ensure!(
            result.transaction_hash.is_none()
                && result.transaction_nonce.is_none()
                && result.receipt_block_number.is_none()
                && result.receipt_block_hash.is_none()
                && (result.eth_shortfall_wei != "0" || result.uwu_shortfall_base_units != "0"),
            "funding-required bounty result is inconsistent"
        ),
        ReferralBountyDisposition::Submitted => {
            validate_hash(
                result
                    .transaction_hash
                    .as_deref()
                    .context("submitted bounty has no hash")?,
                "submitted hash",
            )?;
            validate_decimal(
                result
                    .transaction_nonce
                    .as_deref()
                    .context("submitted bounty has no nonce")?,
                "submitted nonce",
                false,
            )?;
            ensure!(
                result.receipt_block_number.is_none() && result.receipt_block_hash.is_none(),
                "submitted bounty already has a receipt"
            );
        }
        ReferralBountyDisposition::Confirmed => {
            validate_hash(
                result
                    .transaction_hash
                    .as_deref()
                    .context("confirmed bounty has no hash")?,
                "confirmed hash",
            )?;
            validate_decimal(
                result
                    .transaction_nonce
                    .as_deref()
                    .context("confirmed bounty has no nonce")?,
                "confirmed nonce",
                false,
            )?;
            validate_decimal(
                result
                    .receipt_block_number
                    .as_deref()
                    .context("confirmed bounty has no block")?,
                "confirmed block",
                false,
            )?;
            validate_hash(
                result
                    .receipt_block_hash
                    .as_deref()
                    .context("confirmed bounty has no block hash")?,
                "confirmed block hash",
            )?;
        }
    }
    Ok(())
}

fn parse_nonzero_address(value: &str, label: &str) -> Result<Address> {
    let address = Address::from_str(&value.to_ascii_lowercase())
        .with_context(|| format!("{label} is not a canonical Ethereum address"))?;
    ensure!(address != Address::ZERO, "{label} must not be zero");
    Ok(address)
}

fn referral_phase_label(value: ReferralRewardPhase) -> String {
    match value {
        ReferralRewardPhase::NewContact => "new_contact",
        ReferralRewardPhase::Direct => "direct",
        ReferralRewardPhase::Attributed => "attributed",
        ReferralRewardPhase::OnboardingComplete => "onboarding_complete",
        ReferralRewardPhase::RewardPending => "reward_pending",
        ReferralRewardPhase::Submitted => "submitted",
        ReferralRewardPhase::Confirmed => "confirmed",
    }
    .to_owned()
}

fn validate_inbox(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value.chars().all(|character| character.is_ascii_hexdigit())
            && value == value.to_ascii_lowercase(),
        "XMTP inbox ID must be 32 lowercase hexadecimal bytes"
    );
    Ok(())
}

fn validate_decimal(value: &str, label: &str, positive: bool) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 78
            && value.chars().all(|character| character.is_ascii_digit())
            && (value == "0" || !value.starts_with('0'))
            && (value.len() < MAX_UINT256_DECIMAL.len()
                || (value.len() == MAX_UINT256_DECIMAL.len() && value <= MAX_UINT256_DECIMAL)),
        "{label} is not a canonical uint256 decimal"
    );
    ensure!(!positive || value != "0", "{label} must be positive");
    Ok(())
}

fn validate_hash(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() == 66
            && value.starts_with("0x")
            && value[2..]
                .chars()
                .all(|character| character.is_ascii_hexdigit()),
        "{label} is not 32 bytes"
    );
    Ok(())
}

fn validate_commitment(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase()),
        "{label} is not a lowercase SHA-256 commitment"
    );
    Ok(())
}

fn canonical_origin(value: &str) -> Result<String> {
    let trimmed = value.trim();
    let parsed = reqwest::Url::parse(trimmed).context("public referral origin is not a URL")?;
    ensure!(
        trimmed.len() <= 256
            && parsed.scheme() == "https"
            && parsed.host_str().is_some()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.fragment().is_none(),
        "public referral origin must be a bounded HTTPS origin"
    );
    Ok(parsed.origin().ascii_serialization())
}

fn commitment(kind: &str, text: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"cthuwu-growth-v1\0");
    digest.update(kind.as_bytes());
    digest.update(b"\0");
    digest.update(text.as_bytes());
    format!("{:x}", digest.finalize())
}

fn referral_ack(status: &str, referrer: Option<Address>) -> String {
    format!(
        "[[cthuwu:referral-attribution-ack:v1;status={status};referrer={}]]",
        referrer
            .map(|address| address.to_string())
            .unwrap_or_else(|| "none".to_owned())
    )
}

fn address_key(address: Address) -> String {
    address.to_string().trim_start_matches("0x").to_owned()
}

fn short_address(address: Address) -> String {
    let value = address.to_string();
    format!("{}…{}", &value[..8], &value[value.len() - 6..])
}

fn format_uwu_base_units(value: &str) -> String {
    let (whole, fraction) = if value.len() > 18 {
        value.split_at(value.len() - 18)
    } else {
        let padded = format!("{}{}", "0".repeat(18 - value.len()), value);
        return match padded.trim_end_matches('0') {
            "" => "0".to_owned(),
            fraction => format!("0.{fraction}"),
        };
    };
    match fraction.trim_end_matches('0') {
        "" => whole.to_owned(),
        fraction => format!("{whole}.{fraction}"),
    }
}

fn bounded(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn decimal_add(left: &str, right: &str) -> String {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let length = left.len().max(right.len());
    let mut result = Vec::with_capacity(length + 1);
    let mut carry = 0_u8;
    for offset in 0..length {
        let left_digit = left
            .len()
            .checked_sub(offset + 1)
            .map_or(0, |index| left[index] - b'0');
        let right_digit = right
            .len()
            .checked_sub(offset + 1)
            .map_or(0, |index| right[index] - b'0');
        let sum = left_digit + right_digit + carry;
        result.push(b'0' + sum % 10);
        carry = sum / 10;
    }
    if carry != 0 {
        result.push(b'0' + carry);
    }
    result.reverse();
    String::from_utf8(result).expect("decimal addition remains ASCII")
}

fn decimal_shortfall(balance: &str, target: &str) -> String {
    if balance.len() > target.len() || (balance.len() == target.len() && balance >= target) {
        return "0".to_owned();
    }
    let mut minuend = target.as_bytes().to_vec();
    let mut subtrahend = vec![b'0'; minuend.len().saturating_sub(balance.len())];
    subtrahend.extend_from_slice(balance.as_bytes());
    let mut borrow = 0_i16;
    for index in (0..minuend.len()).rev() {
        let left = i16::from(minuend[index] - b'0') - borrow;
        let right = i16::from(subtrahend[index] - b'0');
        if left < right {
            minuend[index] = b'0' + u8::try_from(left + 10 - right).unwrap_or(0);
            borrow = 1;
        } else {
            minuend[index] = b'0' + u8::try_from(left - right).unwrap_or(0);
            borrow = 0;
        }
    }
    let first = minuend
        .iter()
        .position(|digit| *digit != b'0')
        .unwrap_or(minuend.len() - 1);
    String::from_utf8(minuend[first..].to_vec()).expect("decimal subtraction remains ASCII")
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("growth snapshot {} must not be a symlink", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::{collections::VecDeque, sync::Mutex};

    const INBOX_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const INBOX_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const INBOX_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    #[derive(Default)]
    struct RecordingGateway {
        calls: Mutex<Vec<Value>>,
    }

    #[async_trait]
    impl Erc8004Gateway for RecordingGateway {
        async fn invoke(&self, _action_id: &str, operation: Value) -> Result<Value> {
            self.calls.lock().unwrap().push(operation);
            bail!("offline")
        }
    }

    struct ScriptedGateway {
        calls: Mutex<Vec<Value>>,
        results: Mutex<VecDeque<Value>>,
    }

    impl ScriptedGateway {
        fn new(results: Vec<Value>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                results: Mutex::new(results.into()),
            }
        }
    }

    #[async_trait]
    impl Erc8004Gateway for ScriptedGateway {
        async fn invoke(&self, _action_id: &str, operation: Value) -> Result<Value> {
            self.calls.lock().unwrap().push(operation);
            self.results
                .lock()
                .unwrap()
                .pop_front()
                .context("scripted referral result exhausted")
        }
    }

    fn address(value: u8) -> Address {
        Address::from_bytes([value; 20])
    }

    #[test]
    fn public_referral_origin_is_an_exact_https_origin() {
        assert_eq!(
            canonical_origin("https://cthuwu.app/").unwrap(),
            "https://cthuwu.app"
        );
        for value in [
            "http://cthuwu.app",
            "https://user@cthuwu.app",
            "https://cthuwu.app/path",
            "https://cthuwu.app/?query=1",
        ] {
            assert!(canonical_origin(value).is_err());
        }
    }

    fn bounty_result(
        disposition: &str,
        wallet: Address,
        acolyte: Address,
        referrer: Address,
        eth_shortfall: &str,
        uwu_shortfall: &str,
    ) -> Value {
        let transaction_hash = matches!(disposition, "submitted" | "confirmed")
            .then(|| format!("0x{}", "a".repeat(64)));
        let transaction_nonce = transaction_hash.as_ref().map(|_| "7".to_owned());
        let confirmed = disposition == "confirmed";
        json!({
            "kind": "referral_bounty",
            "disposition": disposition,
            "chainId": BASE_MAINNET_CHAIN_ID,
            // Exercise checksum-insensitive Rust validation as the viem sidecar returns checksums.
            "token": "0x9dBa3AE7002DaEfd7324e7B9f829ed31Cb5f0B07",
            "wallet": wallet,
            "acolyte": acolyte,
            "referrer": referrer,
            "amountBaseUnits": DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            "ethBalanceWei": if eth_shortfall == "0" { "10" } else { "0" },
            "ethTargetWei": if eth_shortfall == "0" { "10" } else { eth_shortfall },
            "ethShortfallWei": eth_shortfall,
            "uwuBalanceBaseUnits": if uwu_shortfall == "0" {
                DEFAULT_REFERRAL_BOUNTY_BASE_UNITS
            } else {
                "0"
            },
            "uwuTargetBaseUnits": DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            "uwuShortfallBaseUnits": uwu_shortfall,
            "transactionHash": transaction_hash,
            "transactionNonce": transaction_nonce,
            "receiptBlockNumber": confirmed.then_some("99"),
            "receiptBlockHash": confirmed.then(|| format!("0x{}", "b".repeat(64))),
        })
    }

    fn referred_runtime(root: &Path, gateway: Arc<dyn Erc8004Gateway>) -> GrowthRuntime {
        let mut runtime = GrowthRuntime::open(
            root,
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            gateway,
            1,
        )
        .unwrap();
        runtime
            .mark_onboarding_complete(INBOX_A, &address(2).to_string(), 2)
            .unwrap();
        let marker = format!("[[cthuwu:referral-attribution:v1;referrer={}]]", address(2));
        runtime
            .handle_control(INBOX_B, Some(&address(3).to_string()), &marker, 3)
            .unwrap();
        runtime
    }

    #[test]
    fn branding_attempt_status_survives_restart_and_reports_current_outcome() {
        let root = tempfile::tempdir().unwrap();
        let gateway = Arc::new(RecordingGateway::default());
        let mut runtime = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            gateway.clone(),
            1,
        )
        .unwrap();
        runtime
            .record_branding_attempt(BrandingAttemptOutcome::RpcUnavailable, 2)
            .unwrap();
        drop(runtime);
        let mut recovered = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            gateway,
            3,
        )
        .unwrap();
        let facts = recovered.operator_runtime_facts(address(4), 0, 3);
        assert!(facts.contains("growth.last_branding_attempt=RpcUnavailable"));
        assert!(facts.contains("growth.last_branding_attempt_unix=2"));
        recovered
            .record_branding_attempt(BrandingAttemptOutcome::Queued, 4)
            .unwrap();
        let facts = recovered.operator_runtime_facts(address(4), 0, 4);
        assert!(facts.contains("growth.last_branding_attempt=Queued"));
        assert!(!facts.contains("growth.last_branding_attempt=RpcUnavailable"));
    }

    #[test]
    fn first_verified_referrer_is_immutable_and_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let gateway = Arc::new(RecordingGateway::default());
        let mut runtime = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            gateway.clone(),
            1,
        )
        .unwrap();
        runtime
            .mark_onboarding_complete(INBOX_A, &address(2).to_string(), 2)
            .unwrap();
        let first = format!("[[cthuwu:referral-attribution:v1;referrer={}]]", address(2));
        let accepted = runtime
            .handle_control(INBOX_B, Some(&address(3).to_string()), &first, 3)
            .unwrap()
            .unwrap();
        assert!(accepted.contains(&referral_ack("accepted", Some(address(2)))));
        assert_eq!(
            runtime
                .handle_control(INBOX_B, Some(&address(3).to_string()), &first, 4)
                .unwrap()
                .unwrap(),
            referral_ack("accepted", Some(address(2)))
        );
        let later = format!("[[cthuwu:referral-attribution:v1;referrer={}]]", address(4));
        assert!(
            runtime
                .handle_control(INBOX_B, Some(&address(3).to_string()), &later, 4)
                .unwrap()
                .unwrap()
                .contains(&referral_ack("immutable", Some(address(2))))
        );
        assert!(
            runtime
                .handle_control(INBOX_A, Some(&address(4).to_string()), &first, 4)
                .unwrap()
                .unwrap()
                .contains("already pinned to another authenticated acolyte wallet")
        );
        assert!(runtime.record(address(4)).is_none());
        runtime
            .register_operator(INBOX_C, &address(4).to_string(), 5)
            .unwrap();
        let operator_referral =
            format!("[[cthuwu:referral-attribution:v1;referrer={}]]", address(4));
        assert!(
            runtime
                .handle_control(
                    INBOX_C,
                    Some(&address(5).to_string()),
                    &operator_referral,
                    5,
                )
                .unwrap()
                .unwrap()
                .contains("same authenticated XMTP inbox")
        );
        assert!(runtime.record(address(5)).is_none());
        drop(runtime);
        let recovered = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            gateway,
            5,
        )
        .unwrap();
        assert_eq!(recovered.immutable_referrer(address(3)), Some(address(2)));
    }

    #[test]
    fn direct_and_duplicate_onboarding_never_create_a_bounty() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            Arc::new(RecordingGateway::default()),
            1,
        )
        .unwrap();
        assert!(
            runtime
                .mark_onboarding_complete(INBOX_A, &address(2).to_string(), 2)
                .unwrap()
        );
        assert!(
            !runtime
                .mark_onboarding_complete(INBOX_A, &address(2).to_string(), 3)
                .unwrap()
        );
        let late = format!("[[cthuwu:referral-attribution:v1;referrer={}]]", address(3));
        assert!(
            runtime
                .handle_control(INBOX_A, Some(&address(2).to_string()), &late, 4)
                .unwrap()
                .unwrap()
                .contains(&referral_ack("direct", None))
        );
        let record = runtime.record(address(2)).unwrap();
        assert_eq!(record.phase, ReferralRewardPhase::Direct);
        assert!(record.reward_action_id.is_none());
    }

    #[test]
    fn one_recovered_xmtp_identity_cannot_create_multiple_bounties() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = referred_runtime(root.path(), Arc::new(RecordingGateway::default()));
        assert!(
            runtime
                .mark_onboarding_complete(INBOX_B, &address(3).to_string(), 4)
                .unwrap()
        );

        assert!(
            !runtime
                .mark_contact(INBOX_B, &address(4).to_string(), 5)
                .unwrap()
        );
        let marker = format!("[[cthuwu:referral-attribution:v1;referrer={}]]", address(2));
        assert!(
            runtime
                .handle_control(INBOX_B, Some(&address(4).to_string()), &marker, 5)
                .unwrap()
                .unwrap()
                .contains("already pinned to another authenticated acolyte wallet")
        );
        assert!(
            !runtime
                .mark_onboarding_complete(INBOX_B, &address(4).to_string(), 6)
                .unwrap()
        );
        assert!(runtime.record(address(4)).is_none());
        assert_eq!(runtime.stats(0, 6).successful_referrals, 1);
    }

    #[test]
    fn referred_onboarding_prepares_exactly_one_durable_reward() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            Arc::new(RecordingGateway::default()),
            1,
        )
        .unwrap();
        runtime
            .mark_onboarding_complete(INBOX_A, &address(2).to_string(), 2)
            .unwrap();
        let marker = format!("[[cthuwu:referral-attribution:v1;referrer={}]]", address(2));
        runtime
            .handle_control(INBOX_B, Some(&address(3).to_string()), &marker, 3)
            .unwrap();
        assert!(
            runtime
                .mark_onboarding_complete(INBOX_B, &address(3).to_string(), 4)
                .unwrap()
        );
        assert!(
            !runtime
                .mark_onboarding_complete(INBOX_B, &address(3).to_string(), 5)
                .unwrap()
        );
        let record = runtime.record(address(3)).unwrap();
        assert_eq!(record.phase, ReferralRewardPhase::RewardPending);
        let expected = format!("referral-bounty:{}", address_key(address(3)));
        assert_eq!(record.reward_action_id.as_deref(), Some(expected.as_str()));
    }

    #[tokio::test]
    async fn crash_after_onboarding_terminal_event_resumes_reward_preparation() {
        let root = tempfile::tempdir().unwrap();
        let gateway = Arc::new(RecordingGateway::default());
        let mut runtime = referred_runtime(root.path(), gateway.clone());
        let record = runtime
            .state
            .records
            .iter_mut()
            .find(|record| record.acolyte == address(3).to_string())
            .unwrap();
        record.onboarding_completed_at_unix = Some(4);
        record.phase = ReferralRewardPhase::OnboardingComplete;
        record.next_attempt_unix = 4;
        record.updated_at_unix = 4;
        runtime.persist(4).unwrap();
        drop(runtime);

        let mut recovered = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            gateway,
            5,
        )
        .unwrap();
        assert!(recovered.has_due(5));
        assert!(recovered.maintain_one(0, 5).await.unwrap().is_none());
        let record = recovered.record(address(3)).unwrap();
        assert_eq!(record.phase, ReferralRewardPhase::RewardPending);
        assert!(record.reward_action_id.is_some());
    }

    #[tokio::test]
    async fn submitted_and_confirmed_rewards_recover_across_restarts_without_replay() {
        let root = tempfile::tempdir().unwrap();
        let gateway = Arc::new(ScriptedGateway::new(vec![
            bounty_result("submitted", address(1), address(3), address(2), "0", "0"),
            bounty_result("confirmed", address(1), address(3), address(2), "0", "0"),
        ]));
        let mut runtime = referred_runtime(root.path(), gateway.clone());
        runtime
            .mark_contact(INBOX_C, &address(2).to_string(), 4)
            .unwrap();
        runtime
            .mark_onboarding_complete(INBOX_B, &address(3).to_string(), 4)
            .unwrap();
        assert!(runtime.maintain_one(0, 4).await.unwrap().is_none());
        let submitted = runtime.record(address(3)).unwrap();
        assert_eq!(submitted.phase, ReferralRewardPhase::Submitted);
        assert_eq!(submitted.transaction_nonce.as_deref(), Some("7"));
        drop(runtime);

        let mut recovered = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            gateway.clone(),
            5,
        )
        .unwrap();
        let confirmation = recovered
            .maintain_one(0, 4 + RETRY_SECONDS)
            .await
            .unwrap()
            .unwrap();
        assert!(confirmation.text.contains("is confirmed"));
        assert!(confirmation.text.contains("one-time 1 UWU referral bounty"));
        assert_eq!(
            confirmation.audience,
            GrowthDeliveryAudience::Inbox(INBOX_C.to_owned())
        );
        drop(recovered);

        let mut confirmed = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            gateway.clone(),
            100,
        )
        .unwrap();
        let replayed_delivery = confirmed.maintain_one(0, 100).await.unwrap().unwrap();
        assert_eq!(replayed_delivery.commitment, confirmation.commitment);
        confirmed
            .acknowledge_delivery(&replayed_delivery.commitment, true, 100)
            .unwrap();
        assert!(confirmed.maintain_one(0, 101).await.unwrap().is_none());
        assert_eq!(gateway.calls.lock().unwrap().len(), 2);
        assert_eq!(
            confirmed.record(address(3)).unwrap().phase,
            ReferralRewardPhase::Confirmed
        );
    }

    #[tokio::test]
    async fn insufficient_uwu_and_base_eth_remain_pending_and_resume_after_funding() {
        for (eth_shortfall, uwu_shortfall, expected) in [
            ("123", "0", "123 MORE BASE ETH WEI"),
            (
                "0",
                DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
                "1000000000000000000 MORE UWU BASE UNITS",
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let gateway = Arc::new(ScriptedGateway::new(vec![
                bounty_result(
                    "funding_required",
                    address(1),
                    address(3),
                    address(2),
                    eth_shortfall,
                    uwu_shortfall,
                ),
                bounty_result("submitted", address(1), address(3), address(2), "0", "0"),
            ]));
            let mut runtime = referred_runtime(root.path(), gateway.clone());
            runtime
                .mark_onboarding_complete(INBOX_B, &address(3).to_string(), 4)
                .unwrap();
            let notice = runtime.maintain_one(0, 4).await.unwrap().unwrap();
            assert!(notice.text.contains(expected));
            assert_eq!(
                runtime.record(address(3)).unwrap().phase,
                ReferralRewardPhase::RewardPending
            );
            runtime
                .acknowledge_delivery(&notice.commitment, true, 4)
                .unwrap();
            assert!(
                runtime
                    .maintain_one(0, 4 + FUNDING_RETRY_SECONDS)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                runtime.record(address(3)).unwrap().phase,
                ReferralRewardPhase::Submitted
            );
            assert_eq!(gateway.calls.lock().unwrap().len(), 2);
        }
    }

    #[test]
    fn malformed_zero_self_and_unknown_referrers_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            Arc::new(RecordingGateway::default()),
            1,
        )
        .unwrap();
        for marker in [
            "[[cthuwu:referral-attribution:v1;referrer=0x0000000000000000000000000000000000000000]]".to_owned(),
            format!("[[cthuwu:referral-attribution:v1;referrer={}]]", address(2)),
            format!("[[cthuwu:referral-attribution:v1;referrer={}]]", address(4)),
            format!("[[cthuwu:referral-attribution:v1;referrer={}]]", address(1)),
            "[[cthuwu:referral-attribution:v1;referrer=nope]]".to_owned(),
        ] {
            assert!(runtime.handle_control(INBOX_A, Some(&address(2).to_string()), &marker, 2).unwrap().is_some());
        }
        let treasury_as_acolyte =
            format!("[[cthuwu:referral-attribution:v1;referrer={}]]", address(2));
        assert!(
            runtime
                .handle_control(
                    INBOX_A,
                    Some(&address(1).to_string()),
                    &treasury_as_acolyte,
                    2,
                )
                .unwrap()
                .is_some()
        );
        assert!(runtime.record(address(2)).is_none());
        assert!(runtime.record(address(1)).is_none());
    }

    #[test]
    fn terminal_branding_state_survives_action_pruning_and_restart() {
        let root = tempfile::tempdir().unwrap();
        let gateway = Arc::new(RecordingGateway::default());
        let mut runtime = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            gateway.clone(),
            1,
        )
        .unwrap();
        runtime
            .mark_contact(INBOX_A, &address(2).to_string(), 2)
            .unwrap();
        runtime
            .note_branding_state(INBOX_A, address(2), DurableBrandingState::Declined, 3)
            .unwrap();
        drop(runtime);

        let mut recovered = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            gateway,
            4,
        )
        .unwrap();
        assert_eq!(
            recovered.branding_state(address(2)),
            Some(DurableBrandingState::Declined)
        );
        recovered
            .note_branding_state(INBOX_A, address(2), DurableBrandingState::Branded, 5)
            .unwrap();
        recovered
            .mark_onboarding_complete(INBOX_A, &address(2).to_string(), 6)
            .unwrap();
        assert_eq!(recovered.branded_count(), 1);
    }

    #[tokio::test]
    async fn operator_prompts_are_rate_limited_and_rotated() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = GrowthRuntime::open(
            root.path(),
            address(1),
            DEFAULT_REFERRAL_BOUNTY_BASE_UNITS,
            DEFAULT_PUBLIC_ORIGIN,
            Arc::new(RecordingGateway::default()),
            1,
        )
        .unwrap();
        runtime
            .register_operator(INBOX_A, &address(2).to_string(), 2)
            .unwrap();
        runtime
            .register_operator(INBOX_C, &address(2).to_string(), 3)
            .unwrap();
        assert_eq!(runtime.state.verified_operators.len(), 1);
        assert_eq!(runtime.state.verified_operators[0].inbox_id, INBOX_C);
        assert!(!runtime.has_due(3));
        let due = 1 + OPERATOR_PROMPT_COOLDOWN_SECONDS;
        let first = runtime.maintain_one(0, due).await.unwrap().unwrap();
        runtime
            .acknowledge_delivery(&first.commitment, true, due)
            .unwrap();
        assert!(!runtime.has_due(due + 1));
        let second_due = due + OPERATOR_PROMPT_COOLDOWN_SECONDS;
        let second = runtime.maintain_one(0, second_due).await.unwrap().unwrap();
        assert_ne!(first.text, second.text);
        runtime
            .register_operator(INBOX_A, &address(4).to_string(), second_due)
            .unwrap();
        let moved = runtime.maintain_one(0, second_due).await.unwrap().unwrap();
        assert_eq!(
            moved.audience,
            GrowthDeliveryAudience::ActiveOperator(INBOX_A.into())
        );
        assert!(moved.text.contains(&format!("r={}", address(4))));
        assert!(!moved.text.contains(&format!("r={}", address(2))));
        runtime
            .acknowledge_delivery(&moved.commitment, true, second_due)
            .unwrap();
        runtime.reminder_preferences("off", second_due).unwrap();
        assert!(!runtime.has_due(second_due + 30 * 86400));
        runtime.reminder_preferences("daily", second_due).unwrap();
        runtime
            .reminder_preferences("quiet 0 12", second_due)
            .unwrap();
        let next_day = ((second_due / 86400) + 2) * 86400;
        assert!(!runtime.has_due(next_day));
        runtime
            .reminder_preferences("quiet off", second_due)
            .unwrap();
        assert!(runtime.has_due(next_day));
        runtime
            .reminder_preferences("snooze 7", second_due)
            .unwrap();
        assert!(!runtime.has_due(next_day));
    }
}
