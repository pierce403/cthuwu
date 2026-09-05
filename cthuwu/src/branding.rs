//! Durable Acolyte Branding offers and the narrow mint/repair supervisor.
//!
//! The browser signs the deployed contract's exact `MintConsent`; Rust never accepts a prose
//! approximation of that consent and never exposes a generic transaction interface. Every signer
//! call crosses the same typed sidecar boundary used by registration, while the shared
//! registration mutex prevents the two supervisors from racing the Tentacle wallet nonce.

use crate::{
    erc8004::{
        BASE_MAINNET_CHAIN_ID, Erc8004Gateway, IDENTITY_REGISTRY, RegistrationPhase,
        TentacleRegistration,
    },
    growth::{DurableBrandingState, GrowthContext, GrowthDeliveryAudience, GrowthRuntime},
    storage::{ensure_private_directory, restrict_file, sync_directory},
    token_eye::{Address, U256},
};
use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
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
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;
use tokio::sync::{Mutex, Notify};

pub const DEFAULT_INITIAL_PRICE_BASIS_POINTS: u16 = 1_000;
pub const MIN_INITIAL_PRICE_BASIS_POINTS: u16 = 500;
pub const MAX_INITIAL_PRICE_BASIS_POINTS: u16 = 2_000;
pub const WEEKLY_UPKEEP_BASIS_POINTS: u16 = 10;
pub const ACOLYTE_NAME_SCHEME: &str = "acolyte-v1";
pub const ACOLYTE_NAME_TRAIT: &str = "Acolyte Name";
pub const BRANDING_CONTRACT: &str = "0xd8c36f13d79a505c7fbdc5f6467ea3cd75e896da";
pub const BRANDING_RUNTIME_CODE_HASH: &str =
    "0x3a22b742a570dc2d030edf2bd82dceda1e068a297e2c36883f97b7b66ed4ef2d";
pub const UWU_CONTRACT: &str = "0x9dba3ae7002daefd7324e7b9f829ed31cb5f0b07";

const SNAPSHOT_VERSION: u32 = 1;
const SNAPSHOT_FILE: &str = "acolyte-branding-actions.json";
const MAX_SNAPSHOT_BYTES: u64 = 512 * 1024;
const MAX_ACTIONS: usize = 128;
const MAX_SIGNATURE_BYTES: usize = 8 * 1024;
const MAX_NAME_BYTES: usize = 128;
const OFFER_LIFETIME_SECONDS: u64 = 30 * 60;
const MIN_SIGNING_WINDOW_SECONDS: u64 = 120;
const MAX_INSPECTION_BLOCK_AGE_SECONDS: u64 = 5 * 60;
const MAX_FUTURE_BLOCK_SKEW_SECONDS: u64 = 2 * 60;
const RETRY_SECONDS: u64 = 60;
const FUNDING_RETRY_SECONDS: u64 = 5 * 60;
const FUNDING_NOTICE_COOLDOWN_SECONDS: u64 = 24 * 60 * 60;
const TERMINAL_RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(15);
const BRANDING_FOLLOWUP_COOLDOWN_SECONDS: u64 = 24 * 60 * 60;

const FIRST: &[&str] = &[
    "Ainsworth",
    "Ashcombe",
    "Bellingham",
    "Blackwood",
    "Cavendish",
    "Cholmondeley",
    "Davenport",
    "Devereux",
    "Eversleigh",
    "Fairfax",
    "Featherstone",
    "Fitzwilliam",
    "Fortescue",
    "Gainsborough",
    "Harrington",
    "Hawthorne",
    "Kensington",
    "Langford",
    "Marlborough",
    "Montague",
    "Pemberton",
    "Ravenscroft",
    "Sinclair",
    "Somerset",
    "Stanhope",
    "Thackeray",
    "Wainwright",
    "Weatherby",
    "Wellington",
    "Westcott",
    "Whitcombe",
    "Winchester",
    "Abberley",
    "Adderley",
    "Alvingham",
    "Bancroft",
    "Barrington",
    "Beauchamp",
    "Beresford",
    "Brabazon",
    "Broughton",
    "Buckhurst",
    "Cadogan",
    "Chatterton",
    "Chetwynd",
    "Coleridge",
    "Digby",
    "Edgeworth",
    "Frobisher",
    "Granville",
    "Hardwick",
    "Hesketh",
    "Lascelles",
    "Mandeville",
    "Mortimer",
    "Neville",
    "Paget",
    "Rawdon",
    "Rockingham",
    "Sherborne",
    "Trelawney",
    "Waldegrave",
    "Wentworth",
    "Wyndham",
];
const SECOND: &[&str] = &[
    "Arbuthnot",
    "Bramwell",
    "Carrington",
    "Chadwick",
    "Clavering",
    "Cumberland",
    "Darlington",
    "Ellsworth",
    "Farnsworth",
    "Fetherstonhaugh",
    "Godolphin",
    "Grantham",
    "Hargreaves",
    "Kingsley",
    "Loxley",
    "Marchbanks",
    "Molesworth",
    "Northcott",
    "Ormsby",
    "Ponsonby",
    "Radcliffe",
    "Sackville",
    "Smythe",
    "Tavistock",
    "Templeton",
    "Uxbridge",
    "Vane",
    "Walsingham",
    "Wetherell",
    "Whittington",
    "Wickham",
    "Worthing",
    "Acton",
    "Blandford",
    "Boswell",
    "Bridgeman",
    "Bulwer",
    "Calthorpe",
    "Chichester",
    "Coningsby",
    "Delamere",
    "Denham",
    "Dorrington",
    "Eddington",
    "Fane",
    "Fitzalan",
    "Grafton",
    "Grosvenor",
    "Harcourt",
    "Ingleby",
    "Jermyn",
    "Kettering",
    "Lowther",
    "Marwood",
    "Painswick",
    "Quenby",
    "Rivington",
    "SaintJohn",
    "Strathmore",
    "Tichborne",
    "Underhill",
    "Vernon",
    "Wrottesley",
    "Yelverton",
];
const ESTATE_PREFIX: &[&str] = &[
    "Alder", "Amber", "Apple", "Ash", "Barrow", "Beech", "Bel", "Birch", "Black", "Blen", "Blythe",
    "Bracken", "Bram", "Briar", "Bright", "Broad", "Buck", "Cedar", "Charn", "Clear", "Cold",
    "Crow", "Deep", "Dun", "East", "Elder", "Elm", "Ever", "Fair", "Fern", "Fleet", "Fox", "Glen",
    "Gold", "Grand", "Green", "Grey", "Hart", "Hazel", "High", "Holly", "Honey", "Ivy", "Kings",
    "Lang", "Little", "Long", "Low", "Maple", "Marsh", "Mere", "Mill", "Nether", "North", "Oak",
    "Pen", "Pine", "Raven", "Red", "Rose", "Silver", "South", "Stan", "Wych",
];
const ESTATE_SUFFIX: &[&str] = &[
    "abbey", "bank", "borough", "bourne", "bridge", "brook", "bury", "castle", "chester", "cliff",
    "combe", "court", "croft", "dale", "den", "field", "ford", "gate", "grove", "hall", "ham",
    "haven", "heath", "hill", "holm", "hurst", "ington", "land", "leigh", "manor", "marsh",
    "meadow", "mere", "mill", "minster", "moor", "mount", "park", "pool", "port", "ridge", "rose",
    "stead", "stoke", "stone", "thorp", "ton", "vale", "view", "ville", "wall", "water", "way",
    "well", "wick", "wood", "worth", "yard", "end", "fen", "green", "lodge", "priory", "quay",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrandingQuote {
    pub treasury_balance: U256,
    pub price_basis_points: u16,
    pub initial_declared_price: U256,
    pub first_week_upkeep: U256,
}

pub fn quote_initial_branding(
    treasury_balance: U256,
    price_basis_points: u16,
) -> Result<BrandingQuote> {
    if !(MIN_INITIAL_PRICE_BASIS_POINTS..=MAX_INITIAL_PRICE_BASIS_POINTS)
        .contains(&price_basis_points)
    {
        bail!("initial Branding price adjustment must remain between 5% and 20%");
    }
    let initial_declared_price = treasury_balance
        .checked_mul_basis_points(price_basis_points)
        .ok_or_else(|| anyhow::anyhow!("initial Branding price calculation overflowed"))?;
    if initial_declared_price.is_zero() {
        bail!("initial Branding price must be positive");
    }
    let floor_upkeep = initial_declared_price
        .checked_mul_basis_points(WEEKLY_UPKEEP_BASIS_POINTS)
        .ok_or_else(|| anyhow::anyhow!("Branding upkeep calculation overflowed"))?;
    let recomposed = floor_upkeep
        .checked_mul_u64(1_000)
        .ok_or_else(|| anyhow::anyhow!("Branding upkeep comparison overflowed"))?;
    let first_week_upkeep = if recomposed < initial_declared_price {
        add_one(floor_upkeep)
            .ok_or_else(|| anyhow::anyhow!("Branding upkeep rounded past uint256"))?
    } else {
        floor_upkeep
    };
    Ok(BrandingQuote {
        treasury_balance,
        price_basis_points,
        initial_declared_price,
        first_week_upkeep,
    })
}

fn add_one(value: U256) -> Option<U256> {
    let mut bytes = value.to_be_bytes();
    for byte in bytes.iter_mut().rev() {
        let (next, carry) = byte.overflowing_add(1);
        *byte = next;
        if !carry {
            return Some(U256::from_be_bytes(bytes));
        }
    }
    None
}

pub fn acolyte_name(acolyte: Address) -> String {
    let digest = keccak256(acolyte.as_bytes());
    format!(
        "{}-{} of {}{}",
        FIRST[name_index(digest[0], digest[1], FIRST.len())],
        SECOND[name_index(digest[2], digest[3], SECOND.len())],
        ESTATE_PREFIX[name_index(digest[4], digest[5], ESTATE_PREFIX.len())],
        ESTATE_SUFFIX[name_index(digest[6], digest[7], ESTATE_SUFFIX.len())]
    )
}

fn name_index(high: u8, low: u8, length: usize) -> usize {
    usize::from(u16::from_be_bytes([high, low])) % length
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum BrandingPhase {
    Requested,
    OfferPendingDelivery,
    Offered,
    Consented,
    FundingRequired,
    ReceiptPendingDelivery,
    Completed,
    Declined,
    Expired,
    Superseded,
    FailedPermanent,
}

impl BrandingPhase {
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::Declined
                | Self::Expired
                | Self::Superseded
                | Self::FailedPermanent
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct InspectionBinding {
    consent_nonce: String,
    deadline: String,
    block_number: String,
    block_hash: String,
    block_timestamp: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ConsentBinding {
    signature: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ReceiptBinding {
    token_id: String,
    declared_price: String,
    block_number: String,
    block_hash: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PendingDeliveryKind {
    Offer,
    Funding,
    Receipt,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PendingDelivery {
    kind: PendingDeliveryKind,
    commitment: String,
    text: String,
    fingerprint: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct BrandingAction {
    offer_id: String,
    inbox_id: String,
    acolyte: String,
    minter: String,
    controller_agent_id: String,
    referrer: String,
    treasury_balance: String,
    price_basis_points: u16,
    initial_declared_price: String,
    first_week_upkeep: String,
    acolyte_name: String,
    phase: BrandingPhase,
    #[serde(default)]
    observed_branding_status: Option<HelperBrandingStatus>,
    inspection: Option<InspectionBinding>,
    consent: Option<ConsentBinding>,
    receipt: Option<ReceiptBinding>,
    completion_action_id: Option<String>,
    pending_delivery: Option<PendingDelivery>,
    last_funding_fingerprint: Option<String>,
    last_funding_notice_unix: Option<u64>,
    attempt_count: u32,
    next_attempt_unix: u64,
    last_error: Option<String>,
    created_at_unix: u64,
    updated_at_unix: u64,
}

impl BrandingAction {
    fn validate(&self, expected_minter: Address) -> Result<()> {
        validate_offer_id(&self.offer_id)?;
        validate_inbox_id(&self.inbox_id)?;
        let acolyte = parse_nonzero_address(&self.acolyte, "persisted acolyte")?;
        ensure!(
            parse_nonzero_address(&self.minter, "persisted minter")? == expected_minter,
            "persisted Branding action belongs to another Tentacle wallet"
        );
        parse_nonzero_address(&self.referrer, "persisted referrer")?;
        validate_uint256(&self.controller_agent_id, "persisted controller agent ID")?;
        validate_uint256(&self.treasury_balance, "persisted treasury balance")?;
        validate_uint256(
            &self.initial_declared_price,
            "persisted initial declared price",
        )?;
        ensure!(
            self.initial_declared_price != "0",
            "persisted price must be positive"
        );
        validate_uint256(&self.first_week_upkeep, "persisted first upkeep")?;
        ensure!(
            self.first_week_upkeep != "0",
            "persisted upkeep must be positive"
        );
        ensure!(
            (MIN_INITIAL_PRICE_BASIS_POINTS..=MAX_INITIAL_PRICE_BASIS_POINTS)
                .contains(&self.price_basis_points),
            "persisted initial price policy is outside its bound"
        );
        let persisted_quote = quote_initial_branding(
            decimal_u256(&self.treasury_balance)?,
            self.price_basis_points,
        )?;
        ensure!(
            self.initial_declared_price == u256_decimal(persisted_quote.initial_declared_price)
                && self.first_week_upkeep == u256_decimal(persisted_quote.first_week_upkeep),
            "persisted Branding quote does not match its treasury and basis-point policy"
        );
        ensure!(
            self.acolyte_name == acolyte_name(acolyte),
            "persisted Acolyte name does not match {ACOLYTE_NAME_SCHEME}"
        );
        validate_public_name(&self.acolyte_name)?;
        if let Some(inspection) = &self.inspection {
            validate_uint256(&inspection.consent_nonce, "persisted consent nonce")?;
            validate_uint256(&inspection.deadline, "persisted consent deadline")?;
            validate_uint256(&inspection.block_number, "persisted offer block")?;
            validate_hash(&inspection.block_hash, "persisted offer block hash")?;
            let block_timestamp = decimal_u64(
                &inspection.block_timestamp,
                "persisted offer block timestamp",
            )?;
            ensure!(
                inspection.deadline
                    == block_timestamp
                        .checked_add(OFFER_LIFETIME_SECONDS)
                        .context("persisted Branding deadline overflowed")?
                        .to_string(),
                "persisted Branding deadline is not bound to its Base block timestamp"
            );
        }
        if let Some(consent) = &self.consent {
            validate_signature(&consent.signature)?;
        }
        if let Some(receipt) = &self.receipt {
            validate_uint256(&receipt.token_id, "persisted Branding token ID")?;
            ensure!(
                receipt.token_id == address_token_id(acolyte),
                "persisted Branding token ID is not uint160(acolyte) in decimal"
            );
            validate_uint256(&receipt.declared_price, "persisted receipt price")?;
            ensure!(
                receipt.declared_price == self.initial_declared_price,
                "persisted receipt does not echo the signed offer price"
            );
            validate_uint256(&receipt.block_number, "persisted receipt block")?;
            validate_hash(&receipt.block_hash, "persisted receipt block hash")?;
        }
        if let Some(action_id) = &self.completion_action_id {
            validate_action_id(action_id)?;
            ensure!(
                action_id == &format!("branding-complete:{}", self.offer_id),
                "persisted completion action ID is not bound to its offer"
            );
        }
        if let Some(delivery) = &self.pending_delivery {
            validate_action_id(&delivery.commitment)?;
            ensure!(
                !delivery.text.is_empty() && delivery.text.len() <= 16 * 1024,
                "persisted Branding delivery is empty or oversized"
            );
            if let Some(fingerprint) = &delivery.fingerprint {
                validate_hash(fingerprint, "persisted funding fingerprint")?;
            }
            let kind = match delivery.kind {
                PendingDeliveryKind::Offer => "offer",
                PendingDeliveryKind::Funding => "funding",
                PendingDeliveryKind::Receipt => "receipt",
            };
            ensure!(
                delivery.commitment == delivery_commitment(kind, &self.offer_id, &delivery.text),
                "persisted Branding delivery commitment does not match its exact text"
            );
            match delivery.kind {
                PendingDeliveryKind::Offer => {
                    ensure!(
                        delivery.fingerprint.is_none()
                            && self.inspection.as_ref().is_some_and(|inspection| {
                                delivery.text == offer_text(self, inspection)
                            }),
                        "persisted Branding offer delivery does not match its action"
                    );
                }
                PendingDeliveryKind::Receipt => {
                    ensure!(
                        delivery.fingerprint.is_none()
                            && self.inspection.is_some()
                            && self.receipt.is_some()
                            && delivery.text == receipt_text(self),
                        "persisted Branding receipt delivery does not match its action"
                    );
                }
                PendingDeliveryKind::Funding => ensure!(
                    delivery.fingerprint.is_some(),
                    "persisted Branding funding delivery has no fingerprint"
                ),
            }
        }
        if let Some(fingerprint) = &self.last_funding_fingerprint {
            validate_hash(fingerprint, "persisted delivered funding fingerprint")?;
        }
        if let Some(error) = &self.last_error {
            ensure!(error.len() <= 512, "persisted Branding error is oversized");
        }
        ensure!(
            self.updated_at_unix >= self.created_at_unix,
            "persisted Branding timestamps are inverted"
        );
        ensure!(
            self.last_funding_fingerprint.is_some() == self.last_funding_notice_unix.is_some(),
            "persisted Branding funding acknowledgement is incomplete"
        );
        match self.phase {
            BrandingPhase::Requested => ensure!(
                self.inspection.is_none()
                    && self.consent.is_none()
                    && self.receipt.is_none()
                    && self.completion_action_id.is_none()
                    && self.pending_delivery.is_none(),
                "requested Branding action contains later-phase state"
            ),
            BrandingPhase::OfferPendingDelivery => ensure!(
                self.inspection.is_some()
                    && self.consent.is_none()
                    && self.receipt.is_none()
                    && self.completion_action_id.is_none()
                    && self
                        .pending_delivery
                        .as_ref()
                        .is_some_and(|delivery| delivery.kind == PendingDeliveryKind::Offer),
                "pending Branding offer has an invalid persisted shape"
            ),
            BrandingPhase::Offered => ensure!(
                self.inspection.is_some()
                    && self.consent.is_none()
                    && self.receipt.is_none()
                    && self.completion_action_id.is_none()
                    && self.pending_delivery.is_none(),
                "delivered Branding offer has an invalid persisted shape"
            ),
            BrandingPhase::Consented => ensure!(
                self.inspection.is_some()
                    && self.consent.is_some()
                    && self.receipt.is_none()
                    && self.completion_action_id.is_some()
                    && self.pending_delivery.is_none(),
                "consented Branding action has an invalid persisted shape"
            ),
            BrandingPhase::FundingRequired => ensure!(
                self.inspection.is_some()
                    && self.consent.is_some()
                    && self.receipt.is_none()
                    && self.completion_action_id.is_some()
                    && self
                        .pending_delivery
                        .as_ref()
                        .is_none_or(|delivery| { delivery.kind == PendingDeliveryKind::Funding }),
                "resource-blocked Branding action has an invalid persisted shape"
            ),
            BrandingPhase::ReceiptPendingDelivery => ensure!(
                self.inspection.is_some()
                    && self.consent.is_some()
                    && self.receipt.is_some()
                    && self.completion_action_id.is_some()
                    && self
                        .pending_delivery
                        .as_ref()
                        .is_some_and(|delivery| delivery.kind == PendingDeliveryKind::Receipt),
                "completed Branding action has an invalid pending-receipt shape"
            ),
            BrandingPhase::Completed => ensure!(
                self.inspection.is_some()
                    && self.consent.is_none()
                    && self.receipt.is_some()
                    && self.completion_action_id.is_some()
                    && self.pending_delivery.is_none(),
                "terminal Branding receipt did not scrub its consent signature"
            ),
            BrandingPhase::Declined
            | BrandingPhase::Expired
            | BrandingPhase::Superseded
            | BrandingPhase::FailedPermanent => ensure!(
                self.pending_delivery.is_none(),
                "terminal Branding action retains a pending delivery"
            ),
        }
        match self.pending_delivery.as_ref().map(|delivery| delivery.kind) {
            Some(PendingDeliveryKind::Offer) => ensure!(
                self.phase == BrandingPhase::OfferPendingDelivery,
                "offer delivery is attached to another phase"
            ),
            Some(PendingDeliveryKind::Funding) => ensure!(
                self.phase == BrandingPhase::FundingRequired,
                "funding delivery is attached to another phase"
            ),
            Some(PendingDeliveryKind::Receipt) => ensure!(
                self.phase == BrandingPhase::ReceiptPendingDelivery,
                "receipt delivery is attached to another phase"
            ),
            None => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct BrandingSnapshot {
    version: u32,
    cursor: usize,
    actions: Vec<BrandingAction>,
    updated_at_unix: u64,
}

impl BrandingSnapshot {
    fn validate(&self, minter: Address) -> Result<()> {
        ensure!(
            self.version == SNAPSHOT_VERSION,
            "unsupported Branding snapshot version"
        );
        ensure!(
            self.actions.len() <= MAX_ACTIONS,
            "persisted Branding queue is unbounded"
        );
        ensure!(
            self.actions.is_empty() && self.cursor == 0 || self.cursor < self.actions.len(),
            "persisted Branding fair-queue cursor is outside the queue"
        );
        let mut offers = BTreeSet::new();
        let mut action_ids = BTreeSet::new();
        let mut active_conversations = BTreeSet::new();
        for action in &self.actions {
            action.validate(minter)?;
            ensure!(
                offers.insert(action.offer_id.clone()),
                "duplicate Branding offer ID"
            );
            if !action.phase.terminal() {
                ensure!(
                    active_conversations.insert((action.inbox_id.clone(), action.acolyte.clone())),
                    "multiple live Branding actions exist for one authenticated conversation"
                );
            }
            if let Some(action_id) = &action.completion_action_id {
                ensure!(
                    action_ids.insert(action_id.clone()),
                    "duplicate completion action ID"
                );
            }
        }
        Ok(())
    }
}

struct BrandingStore {
    directory: PathBuf,
    path: PathBuf,
}

impl BrandingStore {
    fn new(data_dir: &Path) -> Result<Self> {
        let directory = data_dir.join("state");
        ensure_private_directory(&directory)?;
        let path = directory.join(SNAPSHOT_FILE);
        reject_symlink(&path)?;
        Ok(Self { directory, path })
    }

    fn load_or_create(&self, minter: Address, now: u64) -> Result<BrandingSnapshot> {
        let bytes = match fs::metadata(&self.path) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_file() && metadata.len() <= MAX_SNAPSHOT_BYTES,
                    "Branding snapshot must be a bounded regular file"
                );
                fs::read(&self.path).with_context(|| format!("reading {}", self.path.display()))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let snapshot = BrandingSnapshot {
                    version: SNAPSHOT_VERSION,
                    cursor: 0,
                    actions: Vec::new(),
                    updated_at_unix: now,
                };
                self.save(&snapshot, minter)?;
                return Ok(snapshot);
            }
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", self.path.display()));
            }
        };
        let snapshot: BrandingSnapshot =
            serde_json::from_slice(&bytes).context("Branding snapshot is invalid")?;
        snapshot.validate(minter)?;
        Ok(snapshot)
    }

    fn save(&self, snapshot: &BrandingSnapshot, minter: Address) -> Result<()> {
        snapshot.validate(minter)?;
        reject_symlink(&self.path)?;
        let mut encoded = serde_json::to_vec_pretty(snapshot)?;
        encoded.push(b'\n');
        ensure!(
            encoded.len() as u64 <= MAX_SNAPSHOT_BYTES,
            "Branding snapshot is oversized"
        );
        let mut temporary = NamedTempFile::new_in(&self.directory)?;
        restrict_file(temporary.as_file(), "temporary Acolyte Branding snapshot")?;
        temporary.write_all(&encoded)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&self.path)
            .map_err(|error| error.error)
            .with_context(|| format!("replacing {}", self.path.display()))?;
        sync_directory(&self.directory)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrandingDeliveryTarget {
    ActiveOperator(String),
    Inbox(String),
    Operators,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrandingDelivery {
    pub target: BrandingDeliveryTarget,
    pub text: String,
    commitment: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum HelperDisposition {
    Ready,
    FundingRequired,
    Complete,
    RepairRequired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum HelperBrandingStatus {
    Unminted,
    Active,
    Expired,
    Ineligible,
    RegistryUnavailable,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrandingInspectionResult {
    kind: String,
    disposition: HelperDisposition,
    chain_id: u64,
    contract: String,
    runtime_code_hash: String,
    identity_registry: String,
    uwu: String,
    observed_block_number: String,
    observed_block_hash: String,
    observed_block_timestamp: String,
    minter: String,
    acolyte: String,
    token_id: String,
    controller_agent_id: String,
    referrer: String,
    initial_declared_price: String,
    first_week_upkeep: String,
    acolyte_name: String,
    consent_nonce: String,
    branding_status: HelperBrandingStatus,
    owner: String,
    onchain_controller_agent_id: String,
    onchain_referrer: String,
    onchain_declared_price: String,
    paid_through: String,
    name_trait: Option<String>,
    eth_balance_wei: String,
    eth_target_wei: String,
    eth_shortfall_wei: String,
    uwu_balance: String,
    uwu_target: String,
    uwu_shortfall_wei: String,
    allowance: String,
    estimated_cost_wei: String,
    execution_gas: String,
    l1_data_fee_wei: String,
    l1_data_fee_exact: bool,
    max_fee_per_gas_wei: String,
    max_priority_fee_per_gas_wei: String,
    safety_bps: String,
    reserve_wei: String,
    exact_operations: Vec<String>,
    conservative_operations: Vec<String>,
    pending_operations: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrandingTransactionHash {
    operation: String,
    transaction_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrandingCompletionResult {
    kind: String,
    disposition: HelperDisposition,
    chain_id: u64,
    contract: String,
    runtime_code_hash: String,
    identity_registry: String,
    uwu: String,
    observed_block_number: String,
    observed_block_hash: String,
    observed_block_timestamp: String,
    minter: String,
    acolyte: String,
    token_id: String,
    controller_agent_id: String,
    referrer: String,
    initial_declared_price: String,
    first_week_upkeep: String,
    acolyte_name: String,
    consent_nonce: String,
    current_consent_nonce: String,
    branding_status: HelperBrandingStatus,
    owner: String,
    onchain_controller_agent_id: String,
    onchain_referrer: String,
    onchain_declared_price: String,
    paid_through: String,
    name_trait: Option<String>,
    eth_balance_wei: String,
    eth_target_wei: String,
    eth_shortfall_wei: String,
    uwu_balance: String,
    uwu_target: String,
    uwu_shortfall_wei: String,
    allowance: String,
    estimated_cost_wei: String,
    execution_gas: String,
    l1_data_fee_wei: String,
    l1_data_fee_exact: bool,
    max_fee_per_gas_wei: String,
    max_priority_fee_per_gas_wei: String,
    safety_bps: String,
    reserve_wei: String,
    exact_operations: Vec<String>,
    conservative_operations: Vec<String>,
    pending_operations: Vec<String>,
    transaction_hashes: Vec<BrandingTransactionHash>,
}

struct BrandingRuntime {
    store: BrandingStore,
    state: BrandingSnapshot,
    minter: Address,
    gateway: Arc<dyn Erc8004Gateway>,
}

impl BrandingRuntime {
    fn open(data_dir: &Path, minter: Address, gateway: Arc<dyn Erc8004Gateway>) -> Result<Self> {
        ensure!(minter != Address::ZERO, "Branding minter must not be zero");
        let store = BrandingStore::new(data_dir)?;
        let state = store.load_or_create(minter, unix_seconds()?)?;
        Ok(Self {
            store,
            state,
            minter,
            gateway,
        })
    }

    fn persist(&mut self, now: u64) -> Result<()> {
        self.state.updated_at_unix = now;
        if self.state.actions.is_empty() {
            self.state.cursor = 0;
        } else if self.state.cursor >= self.state.actions.len() {
            self.state.cursor %= self.state.actions.len();
        }
        self.store.save(&self.state, self.minter)
    }

    fn prune_terminal(&mut self, now: u64) {
        self.state.actions.retain(|action| {
            !action.phase.terminal()
                || now.saturating_sub(action.updated_at_unix) <= TERMINAL_RETENTION_SECONDS
        });
        if self.state.actions.is_empty() {
            self.state.cursor = 0;
        } else {
            self.state.cursor %= self.state.actions.len();
        }
    }

    fn durable_branding_updates(
        &self,
    ) -> Result<Vec<(String, Address, DurableBrandingState, u64)>> {
        let mut seen = BTreeSet::new();
        let mut updates = Vec::new();
        for action in self.state.actions.iter().rev() {
            if !seen.insert(action.acolyte.clone()) {
                continue;
            }
            let state = match action.phase {
                BrandingPhase::Declined => Some(DurableBrandingState::Declined),
                BrandingPhase::ReceiptPendingDelivery | BrandingPhase::Completed => {
                    Some(DurableBrandingState::Branded)
                }
                BrandingPhase::FailedPermanent => match action.observed_branding_status {
                    Some(HelperBrandingStatus::Active) => Some(DurableBrandingState::Branded),
                    Some(HelperBrandingStatus::Expired) => Some(DurableBrandingState::Inactive),
                    Some(HelperBrandingStatus::Ineligible) => {
                        Some(DurableBrandingState::Ineligible)
                    }
                    Some(HelperBrandingStatus::RegistryUnavailable)
                    | Some(HelperBrandingStatus::Unminted)
                    | None => None,
                },
                BrandingPhase::Requested
                | BrandingPhase::OfferPendingDelivery
                | BrandingPhase::Offered
                | BrandingPhase::Consented
                | BrandingPhase::FundingRequired
                | BrandingPhase::Expired
                | BrandingPhase::Superseded => None,
            };
            if let Some(state) = state {
                updates.push((
                    action.inbox_id.clone(),
                    parse_nonzero_address(&action.acolyte, "persisted Branding acolyte")?,
                    state,
                    action.updated_at_unix,
                ));
            }
        }
        Ok(updates)
    }

    fn state_for(&self, inbox_id: &str, acolyte: Address, now: u64) -> (&'static str, bool) {
        let acolyte = acolyte.to_string();
        let latest = self
            .state
            .actions
            .iter()
            .rfind(|action| action.inbox_id == inbox_id && action.acolyte == acolyte);
        match latest {
            None => ("not_offered", true),
            Some(action) => match action.phase {
                BrandingPhase::Requested | BrandingPhase::OfferPendingDelivery => {
                    ("branding_offered", false)
                }
                BrandingPhase::Offered => ("consent_pending", false),
                BrandingPhase::Consented
                | BrandingPhase::FundingRequired
                | BrandingPhase::ReceiptPendingDelivery => ("completion_pending", false),
                BrandingPhase::Completed => ("branded", false),
                BrandingPhase::Declined => ("declined", false),
                BrandingPhase::FailedPermanent => match action.observed_branding_status {
                    Some(HelperBrandingStatus::Active) => ("branded", false),
                    Some(HelperBrandingStatus::Expired) => ("branding_expired", false),
                    Some(HelperBrandingStatus::Ineligible) => ("branding_ineligible", false),
                    Some(HelperBrandingStatus::RegistryUnavailable)
                    | Some(HelperBrandingStatus::Unminted)
                    | None => (
                        "follow_up_due",
                        now.saturating_sub(action.updated_at_unix)
                            >= BRANDING_FOLLOWUP_COOLDOWN_SECONDS,
                    ),
                },
                BrandingPhase::Expired | BrandingPhase::Superseded => (
                    "follow_up_due",
                    now.saturating_sub(action.updated_at_unix)
                        >= BRANDING_FOLLOWUP_COOLDOWN_SECONDS,
                ),
            },
        }
    }

    fn enqueue_offer(
        &mut self,
        inbox_id: &str,
        acolyte: Address,
        controller_agent_id: &str,
        referrer: Address,
        quote: BrandingQuote,
        now: u64,
    ) -> Result<bool> {
        let previous_state = self.state.clone();
        match self.enqueue_offer_inner(inbox_id, acolyte, controller_agent_id, referrer, quote, now)
        {
            Ok(queued) => {
                // Duplicate suppression can still retire an unrelated elapsed offer or prune
                // terminal history. Those maintenance changes must be just as durable as a new
                // action, even though the requested action itself was not queued.
                if !queued
                    && self.state != previous_state
                    && let Err(error) = self.persist(now)
                {
                    self.state = previous_state;
                    return Err(error);
                }
                Ok(queued)
            }
            Err(error) => {
                // Never retain an in-memory queue transition that failed to become durable.
                self.state = previous_state;
                Err(error)
            }
        }
    }

    fn enqueue_offer_inner(
        &mut self,
        inbox_id: &str,
        acolyte: Address,
        controller_agent_id: &str,
        referrer: Address,
        quote: BrandingQuote,
        now: u64,
    ) -> Result<bool> {
        validate_inbox_id(inbox_id)?;
        validate_uint256(controller_agent_id, "controller agent ID")?;
        ensure!(
            referrer != Address::ZERO,
            "Branding referrer must not be zero"
        );
        // A browser may ask for a refreshed offer immediately after its displayed deadline,
        // before the periodic supervisor has selected the old action to expire it. Retire that
        // exact unsigned invitation here so its duplicate check cannot suppress the renewal.
        for action in &mut self.state.actions {
            let expired = matches!(
                action.phase,
                BrandingPhase::OfferPendingDelivery | BrandingPhase::Offered
            ) && action
                .inspection
                .as_ref()
                .and_then(|inspection| inspection.deadline.parse::<u64>().ok())
                .is_some_and(|deadline| deadline <= now);
            if expired {
                action.phase = BrandingPhase::Expired;
                action.pending_delivery = None;
                action.updated_at_unix = now;
            }
        }
        self.prune_terminal(now);
        let acolyte_string = acolyte.to_string();
        let referrer_string = referrer.to_string();
        let treasury_balance = u256_decimal(quote.treasury_balance);
        let initial_declared_price = u256_decimal(quote.initial_declared_price);
        let first_week_upkeep = u256_decimal(quote.first_week_upkeep);
        // Automatic admission from a later profile contribution must never cancel execution that
        // already carries authority. Only the explicit, authenticated fresh-offer request may
        // retire consent after its deadline; a verified receipt may not be retired at all.
        if self.state.actions.iter().any(|action| {
            let failed_consent_still_live = action.phase == BrandingPhase::FailedPermanent
                && action.completion_action_id.is_some()
                && action
                    .inspection
                    .as_ref()
                    .and_then(|inspection| inspection.deadline.parse::<u64>().ok())
                    .is_none_or(|deadline| deadline >= now);
            (matches!(
                action.phase,
                BrandingPhase::Consented
                    | BrandingPhase::FundingRequired
                    | BrandingPhase::ReceiptPendingDelivery
            ) || failed_consent_still_live)
                && action.inbox_id == inbox_id
                && action.acolyte == acolyte_string
        }) {
            return Ok(false);
        }
        if self.state.actions.iter().any(|action| {
            !action.phase.terminal()
                && action.inbox_id == inbox_id
                && action.acolyte == acolyte_string
                && action.referrer == referrer_string
                && action.controller_agent_id == controller_agent_id
                && action.treasury_balance == treasury_balance
                && action.price_basis_points == quote.price_basis_points
                && action.initial_declared_price == initial_declared_price
                && action.first_week_upkeep == first_week_upkeep
        }) {
            return Ok(false);
        }
        if self.state.actions.len() >= MAX_ACTIONS
            && let Some(remove_index) = self
                .state
                .actions
                .iter()
                .position(|action| action.phase.terminal())
        {
            self.state.actions.remove(remove_index);
            if remove_index < self.state.cursor {
                self.state.cursor -= 1;
            } else if self.state.cursor >= self.state.actions.len() {
                self.state.cursor = 0;
            }
        }
        for action in &mut self.state.actions {
            if !action.phase.terminal()
                && action.inbox_id == inbox_id
                && action.acolyte == acolyte_string
            {
                action.phase = BrandingPhase::Superseded;
                action.consent = None;
                action.pending_delivery = None;
                action.updated_at_unix = now;
            }
        }
        ensure!(
            self.state.actions.len() < MAX_ACTIONS,
            "Branding action queue is full"
        );
        self.state.actions.push(BrandingAction {
            offer_id: random_id()?,
            inbox_id: inbox_id.to_owned(),
            acolyte: acolyte_string,
            minter: self.minter.to_string(),
            controller_agent_id: controller_agent_id.to_owned(),
            referrer: referrer_string,
            treasury_balance,
            price_basis_points: quote.price_basis_points,
            initial_declared_price,
            first_week_upkeep,
            acolyte_name: acolyte_name(acolyte),
            phase: BrandingPhase::Requested,
            observed_branding_status: None,
            inspection: None,
            consent: None,
            receipt: None,
            completion_action_id: None,
            pending_delivery: None,
            last_funding_fingerprint: None,
            last_funding_notice_unix: None,
            attempt_count: 0,
            next_attempt_unix: now,
            last_error: None,
            created_at_unix: now,
            updated_at_unix: now,
        });
        self.persist(now)?;
        Ok(true)
    }

    fn replace_referrer_from_request(
        &mut self,
        inbox_id: &str,
        acolyte: Address,
        controller_agent_id: &str,
        referrer: Address,
        name: &str,
        now: u64,
    ) -> Result<bool> {
        validate_uint256(controller_agent_id, "active controller agent ID")?;
        ensure!(name == acolyte_name(acolyte), "Acolyte name mismatch");
        let acolyte_string = acolyte.to_string();
        let source_index = self
            .state
            .actions
            .iter()
            .rposition(|action| action.inbox_id == inbox_id && action.acolyte == acolyte_string)
            .context("no current Branding invitation is bound to this conversation")?;
        let source = self.state.actions[source_index].clone();
        ensure!(
            !matches!(
                source.phase,
                BrandingPhase::ReceiptPendingDelivery
                    | BrandingPhase::Completed
                    | BrandingPhase::Declined
            ),
            "the latest Branding action already has a verified receipt or is completed/declined"
        );
        let has_consumed_or_failed_consent = matches!(
            source.phase,
            BrandingPhase::Consented | BrandingPhase::FundingRequired
        ) || (source.phase == BrandingPhase::FailedPermanent
            && source.completion_action_id.is_some());
        if has_consumed_or_failed_consent {
            let deadline = source
                .inspection
                .as_ref()
                .context("the prior Branding consent has no deadline")?
                .deadline
                .parse::<u64>()
                .context("the prior Branding deadline exceeds the local clock range")?;
            ensure!(
                now > deadline,
                "a consented Branding action cannot be replaced before its deadline"
            );
        }
        let quote = BrandingQuote {
            treasury_balance: decimal_u256(&source.treasury_balance)?,
            price_basis_points: source.price_basis_points,
            initial_declared_price: decimal_u256(&source.initial_declared_price)?,
            first_week_upkeep: decimal_u256(&source.first_week_upkeep)?,
        };
        let previous_state = self.state.clone();
        if source.phase.terminal()
            || matches!(
                source.phase,
                BrandingPhase::Consented | BrandingPhase::FundingRequired
            )
        {
            let retired = &mut self.state.actions[source_index];
            retired.phase = BrandingPhase::Superseded;
            retired.consent = None;
            retired.receipt = None;
            retired.completion_action_id = None;
            retired.pending_delivery = None;
            retired.last_funding_fingerprint = None;
            retired.last_funding_notice_unix = None;
            retired.next_attempt_unix = u64::MAX;
            retired.last_error =
                Some("superseded by an authenticated fresh-offer request".to_owned());
            retired.updated_at_unix = now;
        }
        match self.enqueue_offer(inbox_id, acolyte, controller_agent_id, referrer, quote, now) {
            Ok(true) => Ok(true),
            Ok(false) => {
                self.state = previous_state;
                Ok(false)
            }
            Err(error) => {
                self.state = previous_state;
                Err(error)
            }
        }
    }

    fn refresh_requested_quote(
        &mut self,
        index: usize,
        treasury_balance: &str,
        now: u64,
    ) -> Result<()> {
        let action = self
            .state
            .actions
            .get(index)
            .context("Branding quote refresh selected no action")?;
        ensure!(
            action.phase == BrandingPhase::Requested,
            "only an unsigned Requested Branding action may be re-quoted"
        );
        let quote =
            quote_initial_branding(decimal_u256(treasury_balance)?, action.price_basis_points)?;
        let current = &mut self.state.actions[index];
        current.treasury_balance = u256_decimal(quote.treasury_balance);
        current.initial_declared_price = u256_decimal(quote.initial_declared_price);
        current.first_week_upkeep = u256_decimal(quote.first_week_upkeep);
        current.inspection = None;
        current.consent = None;
        current.receipt = None;
        current.completion_action_id = None;
        current.pending_delivery = None;
        current.phase = BrandingPhase::Requested;
        current.attempt_count = current.attempt_count.saturating_add(1);
        current.next_attempt_unix = now;
        current.last_error =
            Some("canonical UWU treasury changed before delivery; quote refreshed".to_owned());
        current.updated_at_unix = now;
        self.persist(now)
    }

    fn accept_consent(
        &mut self,
        inbox_id: &str,
        sender: Address,
        consent: ParsedConsent,
        now: u64,
    ) -> Result<String> {
        let previous_state = self.state.clone();
        let index = self
            .state
            .actions
            .iter()
            .position(|action| action.offer_id == consent.offer_id)
            .context("the Branding offer is unknown or no longer retained")?;
        let action = &mut self.state.actions[index];
        ensure!(
            action.inbox_id == inbox_id,
            "Branding offer conversation mismatch"
        );
        ensure!(
            action.acolyte == sender.to_string(),
            "Branding acolyte mismatch"
        );
        validate_consent_matches_action(&consent, action)?;
        if action.phase == BrandingPhase::Completed {
            return Ok(receipt_text(action));
        }
        if matches!(
            action.phase,
            BrandingPhase::Consented
                | BrandingPhase::FundingRequired
                | BrandingPhase::ReceiptPendingDelivery
        ) {
            ensure!(
                action
                    .consent
                    .as_ref()
                    .is_some_and(|saved| saved.signature == consent.signature),
                "a different signature cannot replace persisted Branding consent"
            );
            return Ok("that exact Branding consent is already bound and pending, fwiend. i'll keep reconciling it without issuing a second mint, uwu.".to_owned());
        }
        ensure!(
            matches!(
                action.phase,
                BrandingPhase::OfferPendingDelivery | BrandingPhase::Offered
            ),
            "Branding offer is not accepting consent"
        );
        let deadline = action
            .inspection
            .as_ref()
            .context("Branding offer has no inspection binding")?
            .deadline
            .parse::<u64>()
            .context("Branding deadline exceeds local clock range")?;
        if now > deadline {
            action.phase = BrandingPhase::Expired;
            action.pending_delivery = None;
            action.updated_at_unix = now;
            if let Err(error) = self.persist(now) {
                self.state = previous_state;
                return Err(error);
            }
            bail!("Branding offer expired before consent was received");
        }
        ensure!(
            deadline.saturating_sub(now) >= MIN_SIGNING_WINDOW_SECONDS,
            "Branding consent arrived without the required execution window"
        );
        action.consent = Some(ConsentBinding {
            signature: consent.signature,
        });
        action.completion_action_id = Some(format!("branding-complete:{}", action.offer_id));
        action.pending_delivery = None;
        action.phase = BrandingPhase::Consented;
        action.next_attempt_unix = now;
        action.updated_at_unix = now;
        if let Err(error) = self.persist(now) {
            // Never retain an in-memory authority transition that failed to become durable.
            self.state = previous_state;
            return Err(error);
        }
        Ok("i bound that exact Base consent to this durable offer. minting and the Acolyte Name repair will continue through the narrow executor, and i'll send a verified receipt when both are canonical, uwu.".to_owned())
    }

    fn decline(
        &mut self,
        inbox_id: &str,
        sender: Address,
        offer_id: &str,
        now: u64,
    ) -> Result<String> {
        validate_offer_id(offer_id)?;
        let action = self
            .state
            .actions
            .iter_mut()
            .find(|action| action.offer_id == offer_id)
            .context("the Branding offer is unknown or no longer retained")?;
        ensure!(
            action.inbox_id == inbox_id,
            "Branding offer conversation mismatch"
        );
        ensure!(
            action.acolyte == sender.to_string(),
            "Branding acolyte mismatch"
        );
        ensure!(
            matches!(
                action.phase,
                BrandingPhase::Requested
                    | BrandingPhase::OfferPendingDelivery
                    | BrandingPhase::Offered
                    | BrandingPhase::Expired
                    | BrandingPhase::Declined
            ),
            "a consented Branding action cannot be declined after execution began"
        );
        action.phase = BrandingPhase::Declined;
        action.consent = None;
        action.pending_delivery = None;
        action.updated_at_unix = now;
        self.persist(now)?;
        Ok(
            "no worries, fwiend—the exact Branding offer is declined and i won't mint from it :3"
                .to_owned(),
        )
    }

    fn has_due(&self, now: u64) -> bool {
        self.state
            .actions
            .iter()
            .any(|action| !action.phase.terminal() && action.next_attempt_unix <= now)
    }

    fn defer_due(&mut self, now: u64) -> Result<()> {
        for action in &mut self.state.actions {
            if !action.phase.terminal() && action.next_attempt_unix <= now {
                action.next_attempt_unix = now.saturating_add(RETRY_SECONDS);
            }
        }
        self.persist(now)
    }

    async fn maintain_one(
        &mut self,
        active_controller_agent_id: &str,
        now: u64,
    ) -> Result<Option<BrandingDelivery>> {
        let Some(index) = self.select_due_fairly(now)? else {
            return Ok(None);
        };
        if self.state.actions[index].controller_agent_id != active_controller_agent_id {
            self.state.actions[index].last_error = Some(
                "the offer controller no longer matches this Tentacle's active registration"
                    .to_owned(),
            );
            self.state.actions[index].next_attempt_unix = now.saturating_add(RETRY_SECONDS);
            self.state.actions[index].updated_at_unix = now;
            self.persist(now)?;
            return Ok(None);
        }
        if self.state.actions[index].phase == BrandingPhase::OfferPendingDelivery
            && self.state.actions[index]
                .inspection
                .as_ref()
                .and_then(|inspection| inspection.deadline.parse::<u64>().ok())
                .is_some_and(|deadline| deadline <= now)
        {
            self.state.actions[index].phase = BrandingPhase::Expired;
            self.state.actions[index].pending_delivery = None;
            self.state.actions[index].updated_at_unix = now;
            self.persist(now)?;
            return Ok(None);
        }
        if let Some(delivery) = self.delivery_for(index) {
            return Ok(Some(delivery));
        }
        let phase = self.state.actions[index].phase;
        let result = match phase {
            BrandingPhase::Requested => self.inspect_offer(index, now).await,
            BrandingPhase::Consented | BrandingPhase::FundingRequired => {
                self.complete_branding(index, now).await
            }
            BrandingPhase::OfferPendingDelivery | BrandingPhase::ReceiptPendingDelivery => {
                unreachable!("pending delivery was returned above")
            }
            BrandingPhase::Offered => {
                let action = &mut self.state.actions[index];
                action.phase = BrandingPhase::Expired;
                action.updated_at_unix = now;
                self.persist(now)?;
                Ok(None)
            }
            BrandingPhase::Completed
            | BrandingPhase::Declined
            | BrandingPhase::Expired
            | BrandingPhase::Superseded
            | BrandingPhase::FailedPermanent => Ok(None),
        };
        match result {
            Ok(delivery) => Ok(delivery),
            Err(error) => {
                self.record_attempt_error(index, &error, now)?;
                Ok(None)
            }
        }
    }

    fn select_due_fairly(&mut self, now: u64) -> Result<Option<usize>> {
        let length = self.state.actions.len();
        if length == 0 {
            return Ok(None);
        }
        let start = self.state.cursor % length;
        let selected = (0..length)
            .map(|offset| (start + offset) % length)
            .find(|index| {
                let action = &self.state.actions[*index];
                !action.phase.terminal() && action.next_attempt_unix <= now
            });
        if let Some(index) = selected {
            // Persist before I/O: one failing action can never starve the rest after a crash.
            self.state.cursor = (index + 1) % length;
            self.persist(now)?;
        }
        Ok(selected)
    }

    fn delivery_for(&self, index: usize) -> Option<BrandingDelivery> {
        let action = self.state.actions.get(index)?;
        let pending = action.pending_delivery.as_ref()?;
        Some(BrandingDelivery {
            target: match pending.kind {
                PendingDeliveryKind::Offer | PendingDeliveryKind::Receipt => {
                    BrandingDeliveryTarget::Inbox(action.inbox_id.clone())
                }
                PendingDeliveryKind::Funding => BrandingDeliveryTarget::Operators,
            },
            text: pending.text.clone(),
            commitment: pending.commitment.clone(),
        })
    }

    async fn inspect_offer(&mut self, index: usize, now: u64) -> Result<Option<BrandingDelivery>> {
        let action = self.state.actions[index].clone();
        let result = self
            .gateway
            .invoke(
                &format!("branding-inspect:{}", action.offer_id),
                json!({
                    "type": "branding_inspect",
                    "acolyte": action.acolyte,
                    "controllerAgentId": action.controller_agent_id,
                    "referrer": action.referrer,
                    "treasuryBalance": action.treasury_balance,
                    "priceBasisPoints": action.price_basis_points,
                    "initialDeclaredPrice": action.initial_declared_price,
                    "acolyteName": action.acolyte_name,
                }),
            )
            .await?;
        let inspection: BrandingInspectionResult = serde_json::from_value(result)
            .context("Branding inspector returned an invalid result shape")?;
        validate_inspection_result(&inspection, &action)?;
        let block_timestamp = decimal_u64(
            &inspection.observed_block_timestamp,
            "Branding observation block timestamp",
        )?;
        ensure!(
            block_timestamp <= now.saturating_add(MAX_FUTURE_BLOCK_SKEW_SECONDS),
            "Branding inspection block timestamp is implausibly far in the future"
        );
        ensure!(
            now.saturating_sub(block_timestamp) <= MAX_INSPECTION_BLOCK_AGE_SECONDS,
            "Branding inspection is too old to anchor a fresh offer"
        );
        if inspection.branding_status == HelperBrandingStatus::RegistryUnavailable {
            bail!("canonical Branding registry was unavailable during inspection");
        }
        if inspection.branding_status != HelperBrandingStatus::Unminted
            || !matches!(
                inspection.disposition,
                HelperDisposition::Ready | HelperDisposition::FundingRequired
            )
        {
            let current = &mut self.state.actions[index];
            current.phase = BrandingPhase::FailedPermanent;
            current.observed_branding_status = Some(inspection.branding_status);
            current.last_error = Some(format!(
                "canonical Branding status is {:?}; offer retired",
                inspection.branding_status
            ));
            current.updated_at_unix = now;
            self.persist(now)?;
            return Ok(None);
        }
        if inspection.uwu_balance != action.treasury_balance {
            // The quote is an unsigned invitation until it is delivered. If the canonical
            // treasury moved while this Requested action waited for its inspection, bind a fresh
            // quote durably and inspect it again rather than retrying an impossible stale tuple.
            self.refresh_requested_quote(index, &inspection.uwu_balance, now)?;
            return Ok(None);
        }
        let binding = InspectionBinding {
            consent_nonce: inspection.consent_nonce,
            deadline: block_timestamp
                .checked_add(OFFER_LIFETIME_SECONDS)
                .context("Branding offer deadline overflowed")?
                .to_string(),
            block_number: inspection.observed_block_number,
            block_hash: inspection.observed_block_hash.to_ascii_lowercase(),
            block_timestamp: inspection.observed_block_timestamp,
        };
        let text = offer_text(&action, &binding);
        let commitment = delivery_commitment("offer", &action.offer_id, &text);
        let current = &mut self.state.actions[index];
        current.observed_branding_status = Some(HelperBrandingStatus::Unminted);
        current.inspection = Some(binding);
        current.phase = BrandingPhase::OfferPendingDelivery;
        current.pending_delivery = Some(PendingDelivery {
            kind: PendingDeliveryKind::Offer,
            commitment: commitment.clone(),
            text: text.clone(),
            fingerprint: None,
        });
        current.attempt_count = current.attempt_count.saturating_add(1);
        current.next_attempt_unix = now;
        current.last_error = None;
        current.updated_at_unix = now;
        self.persist(now)?;
        Ok(Some(BrandingDelivery {
            target: BrandingDeliveryTarget::Inbox(action.inbox_id),
            text,
            commitment,
        }))
    }

    async fn complete_branding(
        &mut self,
        index: usize,
        now: u64,
    ) -> Result<Option<BrandingDelivery>> {
        let action = self.state.actions[index].clone();
        let inspection = action
            .inspection
            .as_ref()
            .context("consented Branding action has no inspection")?;
        let consent = action
            .consent
            .as_ref()
            .context("consented Branding action has no signature")?;
        let action_id = action
            .completion_action_id
            .as_deref()
            .context("consented Branding action has no durable helper action ID")?;
        let result = self
            .gateway
            .invoke(
                action_id,
                json!({
                    "type": "complete_branding",
                    "acolyte": action.acolyte,
                    "minter": action.minter,
                    "controllerAgentId": action.controller_agent_id,
                    "referrer": action.referrer,
                    "treasuryBalance": action.treasury_balance,
                    "priceBasisPoints": action.price_basis_points,
                    "initialDeclaredPrice": action.initial_declared_price,
                    "nonce": inspection.consent_nonce,
                    "deadline": inspection.deadline,
                    "signature": consent.signature,
                    "acolyteName": action.acolyte_name,
                    "offerBlockNumber": inspection.block_number,
                    "offerBlockHash": inspection.block_hash,
                }),
            )
            .await?;
        let completion: BrandingCompletionResult = serde_json::from_value(result)
            .context("Branding executor returned an invalid result shape")?;
        validate_completion_result(&completion, &action)?;
        match completion.disposition {
            HelperDisposition::FundingRequired => {
                let fingerprint = funding_fingerprint(&completion);
                let should_notify = action.last_funding_fingerprint.as_deref()
                    != Some(fingerprint.as_str())
                    || action.last_funding_notice_unix.is_none_or(|last| {
                        now.saturating_sub(last) >= FUNDING_NOTICE_COOLDOWN_SECONDS
                    });
                let current = &mut self.state.actions[index];
                current.phase = BrandingPhase::FundingRequired;
                current.attempt_count = current.attempt_count.saturating_add(1);
                current.last_error = None;
                current.updated_at_unix = now;
                if should_notify {
                    let text = funding_text(&completion);
                    let commitment = delivery_commitment("funding", &action.offer_id, &text);
                    current.pending_delivery = Some(PendingDelivery {
                        kind: PendingDeliveryKind::Funding,
                        commitment: commitment.clone(),
                        text: text.clone(),
                        fingerprint: Some(fingerprint),
                    });
                    current.next_attempt_unix = now;
                    self.persist(now)?;
                    return Ok(Some(BrandingDelivery {
                        target: BrandingDeliveryTarget::Operators,
                        text,
                        commitment,
                    }));
                }
                current.pending_delivery = None;
                current.next_attempt_unix = now.saturating_add(FUNDING_RETRY_SECONDS);
                self.persist(now)?;
                Ok(None)
            }
            HelperDisposition::Complete => {
                // A lost mint response may be repaired after upkeep expiry or registry trouble;
                // owner-only name repair remains valid in those states. The exact active state is
                // required only when this completion actually includes the mint operation.
                let newly_minted = completion
                    .transaction_hashes
                    .iter()
                    .any(|transaction| transaction.operation == "mint");
                if newly_minted {
                    ensure!(
                        completion.branding_status == HelperBrandingStatus::Active,
                        "newly minted Branding is not active"
                    );
                }
                ensure!(
                    completion.name_trait.as_deref() == Some(action.acolyte_name.as_str()),
                    "completed Branding has not repaired the canonical {ACOLYTE_NAME_TRAIT} trait"
                );
                ensure!(
                    completion.current_consent_nonce
                        == decimal_increment(
                            &action
                                .inspection
                                .as_ref()
                                .context("completed action has no consent nonce")?
                                .consent_nonce
                        )?,
                    "completed Branding did not consume exactly one consent nonce"
                );
                let receipt = ReceiptBinding {
                    token_id: completion.token_id,
                    // The receipt echoes the exact signed offer. A delayed owner-only name repair
                    // may observe a legitimately changed mutable on-chain price, which is not a
                    // new mint authority and is deliberately not substituted into this field.
                    declared_price: action.initial_declared_price.clone(),
                    block_number: completion.observed_block_number,
                    block_hash: completion.observed_block_hash.to_ascii_lowercase(),
                };
                let mut receipt_action = action.clone();
                receipt_action.receipt = Some(receipt.clone());
                let text = receipt_text(&receipt_action);
                let commitment = delivery_commitment("receipt", &action.offer_id, &text);
                let current = &mut self.state.actions[index];
                current.receipt = Some(receipt);
                current.phase = BrandingPhase::ReceiptPendingDelivery;
                current.pending_delivery = Some(PendingDelivery {
                    kind: PendingDeliveryKind::Receipt,
                    commitment: commitment.clone(),
                    text: text.clone(),
                    fingerprint: None,
                });
                current.attempt_count = current.attempt_count.saturating_add(1);
                current.next_attempt_unix = now;
                current.last_error = None;
                current.updated_at_unix = now;
                self.persist(now)?;
                Ok(Some(BrandingDelivery {
                    target: BrandingDeliveryTarget::Inbox(action.inbox_id),
                    text,
                    commitment,
                }))
            }
            HelperDisposition::Ready | HelperDisposition::RepairRequired => {
                bail!("Branding executor returned a nonterminal completion disposition")
            }
        }
    }

    fn record_attempt_error(
        &mut self,
        index: usize,
        error: &anyhow::Error,
        now: u64,
    ) -> Result<()> {
        let current = &mut self.state.actions[index];
        current.attempt_count = current.attempt_count.saturating_add(1);
        current.last_error = Some(bounded_diagnostic(&error.to_string(), 512));
        current.pending_delivery = None;
        current.updated_at_unix = now;
        if error
            .to_string()
            .contains("permanent ERC-8004 helper error")
        {
            current.phase = BrandingPhase::FailedPermanent;
            current.consent = None;
        } else {
            current.next_attempt_unix = now.saturating_add(RETRY_SECONDS);
        }
        self.persist(now)
    }

    fn acknowledge_delivery(&mut self, commitment: &str, delivered: bool, now: u64) -> Result<()> {
        validate_action_id(commitment)?;
        let Some(index) = self.state.actions.iter().position(|action| {
            action
                .pending_delivery
                .as_ref()
                .is_some_and(|pending| pending.commitment == commitment)
        }) else {
            return Ok(());
        };
        let action = &mut self.state.actions[index];
        if !delivered {
            action.next_attempt_unix = now.saturating_add(RETRY_SECONDS);
            action.updated_at_unix = now;
            return self.persist(now);
        }
        let pending = action
            .pending_delivery
            .take()
            .context("delivery disappeared while acknowledging it")?;
        match pending.kind {
            PendingDeliveryKind::Offer => {
                action.phase = BrandingPhase::Offered;
                action.next_attempt_unix = action
                    .inspection
                    .as_ref()
                    .and_then(|binding| binding.deadline.parse::<u64>().ok())
                    .unwrap_or(now);
            }
            PendingDeliveryKind::Funding => {
                action.phase = BrandingPhase::FundingRequired;
                action.last_funding_fingerprint = pending.fingerprint;
                action.last_funding_notice_unix = Some(now);
                action.next_attempt_unix = now.saturating_add(FUNDING_RETRY_SECONDS);
            }
            PendingDeliveryKind::Receipt => {
                action.phase = BrandingPhase::Completed;
                action.consent = None;
                action.next_attempt_unix = u64::MAX;
            }
        }
        action.updated_at_unix = now;
        self.persist(now)
    }
}

/// Shared bot/supervisor facade. Branding helper calls retain the registration mutex so registry
/// repair and Branding execution cannot race the same persistent signer nonce.
pub struct SharedBrandingControl {
    runtime: Mutex<BrandingRuntime>,
    growth: Mutex<GrowthRuntime>,
    registration: Arc<Mutex<TentacleRegistration>>,
    wake: Notify,
}

impl SharedBrandingControl {
    pub fn open(
        data_dir: &Path,
        minter: Address,
        gateway: Arc<dyn Erc8004Gateway>,
        registration: Arc<Mutex<TentacleRegistration>>,
        referral_bounty_base_units: &str,
        public_origin: &str,
    ) -> Result<Self> {
        let now = unix_seconds()?;
        let runtime = BrandingRuntime::open(data_dir, minter, gateway.clone())?;
        let mut growth = GrowthRuntime::open(
            data_dir,
            minter,
            referral_bounty_base_units,
            public_origin,
            gateway,
            now,
        )?;
        for (inbox_id, acolyte, state, observed_at) in runtime.durable_branding_updates()? {
            growth.note_branding_state(&inbox_id, acolyte, state, observed_at)?;
        }
        Ok(Self {
            runtime: Mutex::new(runtime),
            growth: Mutex::new(growth),
            registration,
            wake: Notify::new(),
        })
    }
    pub fn maintenance_interval(&self) -> Duration {
        MAINTENANCE_INTERVAL
    }

    pub async fn wait_for_work(&self) {
        self.wake.notified().await;
    }

    pub async fn maintain_once(&self) -> Result<Option<BrandingDelivery>> {
        let now = unix_seconds()?;
        let branding_due = self.runtime.lock().await.has_due(now);
        let growth_due = self.growth.lock().await.has_due(now);
        if !branding_due && !growth_due {
            return Ok(None);
        }
        let mut registration = self.registration.lock().await;
        let _ = registration.maintain(false).await?;
        let active_agent_id = if registration.snapshot().phase == RegistrationPhase::Active {
            registration.snapshot().confirmed_agent_id.clone()
        } else {
            None
        };
        let Some(active_agent_id) = active_agent_id else {
            self.runtime.lock().await.defer_due(now)?;
            self.growth.lock().await.defer_due(now)?;
            return Ok(None);
        };
        if growth_due {
            let branded = self.growth.lock().await.branded_count();
            if let Some(delivery) = self.growth.lock().await.maintain_one(branded, now).await? {
                drop(registration);
                return Ok(Some(BrandingDelivery {
                    target: match delivery.audience {
                        GrowthDeliveryAudience::Inbox(inbox_id) => {
                            BrandingDeliveryTarget::Inbox(inbox_id)
                        }
                        GrowthDeliveryAudience::Operators => BrandingDeliveryTarget::Operators,
                        GrowthDeliveryAudience::ActiveOperator(inbox) => {
                            BrandingDeliveryTarget::ActiveOperator(inbox)
                        }
                    },
                    text: delivery.text,
                    commitment: format!("growth:{}", delivery.commitment),
                }));
            }
        }
        let result = self
            .runtime
            .lock()
            .await
            .maintain_one(&active_agent_id, now)
            .await;
        if result.is_ok() {
            let updates = self.runtime.lock().await.durable_branding_updates()?;
            let mut growth = self.growth.lock().await;
            for (inbox_id, acolyte, state, observed_at) in updates {
                growth.note_branding_state(&inbox_id, acolyte, state, observed_at)?;
            }
        }
        drop(registration);
        result
    }

    pub async fn acknowledge_delivery(&self, delivery: &BrandingDelivery, delivered: bool) {
        match unix_seconds() {
            Ok(now) => {
                let result = if let Some(commitment) = delivery.commitment.strip_prefix("growth:") {
                    self.growth
                        .lock()
                        .await
                        .acknowledge_delivery(commitment, delivered, now)
                } else {
                    self.runtime.lock().await.acknowledge_delivery(
                        &delivery.commitment,
                        delivered,
                        now,
                    )
                };
                if let Err(error) = result {
                    tracing::warn!(%error, "could not persist Branding delivery acknowledgement");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "could not read clock for Branding acknowledgement")
            }
        }
        self.wake.notify_one();
    }

    async fn active_agent_id(&self) -> Option<String> {
        let registration = self.registration.lock().await;
        (registration.snapshot().phase == RegistrationPhase::Active)
            .then(|| registration.snapshot().confirmed_agent_id.clone())
            .flatten()
    }

    async fn enqueue_offer(
        &self,
        inbox_id: &str,
        acolyte: Address,
        referrer: Address,
        quote: BrandingQuote,
    ) -> Result<bool> {
        let agent_id = self
            .active_agent_id()
            .await
            .context("Tentacle must repair its ERC-8004 registration before Branding")?;
        let queued = self.runtime.lock().await.enqueue_offer(
            inbox_id,
            acolyte,
            &agent_id,
            referrer,
            quote,
            unix_seconds()?,
        )?;
        if queued {
            self.wake.notify_one();
        }
        Ok(queued)
    }
}

#[async_trait]
pub trait AcolyteBrandingControl: Send + Sync {
    async fn referral_preferences(&self, _arguments: &str) -> Result<String> {
        bail!("referral preferences unavailable")
    }
    async fn enqueue_default_offer(
        &self,
        inbox_id: &str,
        authenticated_sender_address: &str,
        quote: BrandingQuote,
    ) -> Result<bool>;

    /// Any Branding-looking control is consumed before contact state or inference, including
    /// malformed controls, so a signature can never enter a model prompt.
    async fn handle_public_message(
        &self,
        inbox_id: &str,
        authenticated_sender_address: Option<&str>,
        text: &str,
    ) -> Result<Option<String>>;

    async fn mark_onboarding_complete(
        &self,
        _inbox_id: &str,
        _authenticated_sender_address: &str,
    ) -> Result<bool> {
        Ok(false)
    }

    async fn mark_contact(
        &self,
        _inbox_id: &str,
        _authenticated_sender_address: &str,
    ) -> Result<bool> {
        Ok(false)
    }

    async fn growth_context(
        &self,
        _inbox_id: &str,
        _authenticated_sender_address: &str,
    ) -> Result<(GrowthContext, String, bool)> {
        Ok((
            GrowthContext {
                is_acolyte: false,
                immutable_referrer: None,
                referral_bounty_phase: None,
                shareable_referral_url: None,
            },
            "unknown".to_owned(),
            false,
        ))
    }

    async fn referral_link(&self, _authenticated_sender_address: &str) -> Result<Option<String>> {
        Ok(None)
    }

    async fn register_operator(
        &self,
        _inbox_id: &str,
        _authenticated_sender_address: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn operator_growth_status(&self, _authenticated_sender_address: &str) -> Result<String> {
        Ok("GROWTH STATUS IS UNAVAILABLE IN THIS RUNTIME.".to_owned())
    }

    async fn record_branding_attempt(
        &self,
        _outcome: crate::growth::BrandingAttemptOutcome,
    ) -> Result<()> {
        Ok(())
    }

    async fn operator_growth_facts(&self, _authenticated_sender_address: &str) -> Result<String> {
        Ok("growth.runtime=unavailable".to_owned())
    }
}

#[async_trait]
impl AcolyteBrandingControl for SharedBrandingControl {
    async fn enqueue_default_offer(
        &self,
        inbox_id: &str,
        authenticated_sender_address: &str,
        quote: BrandingQuote,
    ) -> Result<bool> {
        let acolyte = parse_nonzero_address(authenticated_sender_address, "authenticated acolyte")?;
        let referrer = {
            let growth = self.growth.lock().await;
            if growth.identity_conflicts(inbox_id, acolyte)
                || growth.branding_state(acolyte).is_some()
            {
                return Ok(false);
            }
            growth.immutable_referrer(acolyte).unwrap_or(acolyte)
        };
        let (_, should_offer) =
            self.runtime
                .lock()
                .await
                .state_for(inbox_id, acolyte, unix_seconds()?);
        if !should_offer {
            return Ok(false);
        }
        self.enqueue_offer(inbox_id, acolyte, referrer, quote).await
    }

    async fn handle_public_message(
        &self,
        inbox_id: &str,
        authenticated_sender_address: Option<&str>,
        text: &str,
    ) -> Result<Option<String>> {
        if let Some(response) = self.growth.lock().await.handle_control(
            inbox_id,
            authenticated_sender_address,
            text,
            unix_seconds()?,
        )? {
            self.wake.notify_one();
            return Ok(Some(response));
        }
        let parsed = match parse_branding_control(text) {
            BrandingControlMessage::NotBranding => return Ok(None),
            BrandingControlMessage::Invalid => {
                return Ok(Some("that Branding control was malformed or stale, so i treated it as no authority and sent nothing to a signer, fwiend.".to_owned()));
            }
            parsed => parsed,
        };
        let sender = authenticated_sender_address
            .context("Branding controls require an authenticated EVM sender")
            .and_then(|value| parse_nonzero_address(value, "authenticated acolyte"));
        let sender = match sender {
            Ok(sender) => sender,
            Err(_) => {
                return Ok(Some("i could not bind that Branding control to an authenticated Ethereum address, so it authorized nothing, fwiend.".to_owned()));
            }
        };
        if self
            .growth
            .lock()
            .await
            .identity_conflicts(inbox_id, sender)
        {
            return Ok(Some("this recovered XMTP identity is already pinned to another authenticated acolyte wallet, so changing associated wallets cannot authorize Branding or referral actions, fwiend.".to_owned()));
        }
        let now = unix_seconds()?;
        let declined = matches!(&parsed, BrandingControlMessage::Decline { .. });
        let response = match parsed {
            BrandingControlMessage::Request { referrer, name } => {
                let referrer = parse_nonzero_address(&referrer, "requested Branding referrer")?;
                let canonical = self
                    .growth
                    .lock()
                    .await
                    .immutable_referrer(sender)
                    .unwrap_or(sender);
                ensure!(
                    referrer == canonical,
                    "Branding request referrer differs from immutable onboarding attribution"
                );
                let controller_agent_id = self
                    .active_agent_id()
                    .await
                    .context("Tentacle must repair its ERC-8004 registration before Branding")?;
                let queued = self.runtime.lock().await.replace_referrer_from_request(
                    inbox_id,
                    sender,
                    &controller_agent_id,
                    referrer,
                    &name,
                    now,
                )?;
                if queued {
                    self.wake.notify_one();
                    "i'm rebuilding the exact Branding offer with that pinned referrer. i'll send the review after a fresh canonical Base inspection, uwu.".to_owned()
                } else {
                    "that exact Branding offer is already queued, fwiend.".to_owned()
                }
            }
            BrandingControlMessage::Consent(consent) => {
                let response = self
                    .runtime
                    .lock()
                    .await
                    .accept_consent(inbox_id, sender, *consent, now)?;
                self.wake.notify_one();
                response
            }
            BrandingControlMessage::Decline { offer_id } => self
                .runtime
                .lock()
                .await
                .decline(inbox_id, sender, &offer_id, now)?,
            BrandingControlMessage::NotBranding | BrandingControlMessage::Invalid => unreachable!(),
        };
        if declined {
            self.growth.lock().await.note_branding_state(
                inbox_id,
                sender,
                DurableBrandingState::Declined,
                now,
            )?;
        }
        Ok(Some(response))
    }

    async fn mark_contact(
        &self,
        inbox_id: &str,
        authenticated_sender_address: &str,
    ) -> Result<bool> {
        self.growth.lock().await.mark_contact(
            inbox_id,
            authenticated_sender_address,
            unix_seconds()?,
        )
    }

    async fn mark_onboarding_complete(
        &self,
        inbox_id: &str,
        authenticated_sender_address: &str,
    ) -> Result<bool> {
        let completed = self.growth.lock().await.mark_onboarding_complete(
            inbox_id,
            authenticated_sender_address,
            unix_seconds()?,
        )?;
        if completed {
            self.wake.notify_one();
        }
        Ok(completed)
    }

    async fn growth_context(
        &self,
        inbox_id: &str,
        authenticated_sender_address: &str,
    ) -> Result<(GrowthContext, String, bool)> {
        let acolyte = parse_nonzero_address(authenticated_sender_address, "authenticated acolyte")?;
        let (context, durable_branding_state) = {
            let growth = self.growth.lock().await;
            (growth.context(acolyte), growth.branding_state(acolyte))
        };
        let (branding_state, should_offer) = match durable_branding_state {
            Some(DurableBrandingState::Declined) => ("declined", false),
            Some(DurableBrandingState::Branded) => ("branded", false),
            Some(DurableBrandingState::Inactive) => ("branding_expired", false),
            Some(DurableBrandingState::Ineligible) => ("branding_ineligible", false),
            None => self
                .runtime
                .lock()
                .await
                .state_for(inbox_id, acolyte, unix_seconds()?),
        };
        Ok((context, branding_state.to_owned(), should_offer))
    }

    async fn referral_link(&self, authenticated_sender_address: &str) -> Result<Option<String>> {
        let address = parse_nonzero_address(authenticated_sender_address, "authenticated acolyte")?;
        let now = unix_seconds()?;
        let mut growth = self.growth.lock().await;
        let context = growth.context(address);
        if !context.is_acolyte {
            return Ok(None);
        }
        growth.note_referral_link_sent(now)?;
        Ok(context.shareable_referral_url)
    }

    async fn register_operator(
        &self,
        inbox_id: &str,
        authenticated_sender_address: &str,
    ) -> Result<()> {
        self.growth.lock().await.register_operator(
            inbox_id,
            authenticated_sender_address,
            unix_seconds()?,
        )?;
        self.wake.notify_one();
        Ok(())
    }

    async fn referral_preferences(&self, arguments: &str) -> Result<String> {
        self.growth
            .lock()
            .await
            .reminder_preferences(arguments, unix_seconds()?)
    }

    async fn operator_growth_status(&self, authenticated_sender_address: &str) -> Result<String> {
        let address =
            parse_nonzero_address(authenticated_sender_address, "authenticated operator")?;
        let branded = self.growth.lock().await.branded_count();
        self.growth
            .lock()
            .await
            .operator_status(address, branded, unix_seconds()?)
    }

    async fn record_branding_attempt(
        &self,
        outcome: crate::growth::BrandingAttemptOutcome,
    ) -> Result<()> {
        self.growth
            .lock()
            .await
            .record_branding_attempt(outcome, unix_seconds()?)
    }

    async fn operator_growth_facts(&self, authenticated_sender_address: &str) -> Result<String> {
        let address =
            parse_nonzero_address(authenticated_sender_address, "authenticated operator")?;
        let mut phases = std::collections::BTreeMap::<String, usize>::new();
        for action in &self.runtime.lock().await.state.actions {
            *phases.entry(format!("{:?}", action.phase)).or_default() += 1;
        }
        let growth = self.growth.lock().await;
        let mut facts =
            growth.operator_runtime_facts(address, growth.branded_count(), unix_seconds()?);
        facts.push_str("\nbranding.queue_phases=");
        facts.push_str(&serde_json::to_string(&phases)?);
        facts.push_str("\nbranding.delivery_authority=public_runtime_supervisor; model outbound DM tools are not required to deliver an admitted offer\nbranding.status_command=/branding-status\nbranding.history_access=none; operational status is not a DM transcript");
        Ok(facts)
    }
}

#[derive(Debug, Eq, PartialEq)]
enum BrandingControlMessage {
    NotBranding,
    Invalid,
    Request { referrer: String, name: String },
    Consent(Box<ParsedConsent>),
    Decline { offer_id: String },
}

#[derive(Debug, Eq, PartialEq)]
struct ParsedConsent {
    offer_id: String,
    contract: String,
    minter: String,
    controller_agent_id: String,
    acolyte: String,
    referrer: String,
    initial_declared_price: String,
    consent_nonce: String,
    deadline: String,
    block_number: String,
    block_hash: String,
    acolyte_name: String,
    signature: String,
}

fn parse_branding_control(text: &str) -> BrandingControlMessage {
    if !text.to_ascii_lowercase().contains("[[cthuwu:branding-") {
        return BrandingControlMessage::NotBranding;
    }
    if text.trim() != text || text.contains('\n') || text.contains('\r') {
        return BrandingControlMessage::Invalid;
    }
    if let Some(body) = text
        .strip_prefix("[[cthuwu:branding-request:v2;")
        .and_then(|body| body.strip_suffix("]]"))
    {
        let Some(values) = exact_marker_fields(body, &["referrer", "name"]) else {
            return BrandingControlMessage::Invalid;
        };
        if parse_nonzero_address(values[0], "referrer").is_err() {
            return BrandingControlMessage::Invalid;
        }
        let Ok(name) = decode_text_hex(values[1]) else {
            return BrandingControlMessage::Invalid;
        };
        return BrandingControlMessage::Request {
            referrer: values[0].to_owned(),
            name,
        };
    }
    if let Some(body) = text
        .strip_prefix("[[cthuwu:branding-decline:v2;")
        .and_then(|body| body.strip_suffix("]]"))
    {
        let Some(values) = exact_marker_fields(body, &["offer"]) else {
            return BrandingControlMessage::Invalid;
        };
        return if validate_offer_id(values[0]).is_ok() {
            BrandingControlMessage::Decline {
                offer_id: values[0].to_owned(),
            }
        } else {
            BrandingControlMessage::Invalid
        };
    }
    if let Some(body) = text
        .strip_prefix("[[cthuwu:branding-consent:v2;")
        .and_then(|body| body.strip_suffix("]]"))
    {
        let keys = [
            "offer",
            "contract",
            "minter",
            "agent",
            "acolyte",
            "referrer",
            "price",
            "nonce",
            "deadline",
            "block",
            "blockHash",
            "name",
            "signature",
        ];
        let Some(values) = exact_marker_fields(body, &keys) else {
            return BrandingControlMessage::Invalid;
        };
        let valid = validate_offer_id(values[0]).is_ok()
            && parse_nonzero_address(values[1], "contract").is_ok()
            && parse_nonzero_address(values[2], "minter").is_ok()
            && validate_uint256(values[3], "agent").is_ok()
            && parse_nonzero_address(values[4], "acolyte").is_ok()
            && parse_nonzero_address(values[5], "referrer").is_ok()
            && validate_uint256(values[6], "price").is_ok()
            && values[6] != "0"
            && validate_uint256(values[7], "nonce").is_ok()
            && validate_uint256(values[8], "deadline").is_ok()
            && validate_uint256(values[9], "block").is_ok()
            && validate_hash(values[10], "block hash").is_ok()
            && validate_signature(values[12]).is_ok();
        let Ok(name) = decode_text_hex(values[11]) else {
            return BrandingControlMessage::Invalid;
        };
        if !valid || validate_public_name(&name).is_err() {
            return BrandingControlMessage::Invalid;
        }
        return BrandingControlMessage::Consent(Box::new(ParsedConsent {
            offer_id: values[0].to_owned(),
            contract: values[1].to_owned(),
            minter: values[2].to_owned(),
            controller_agent_id: values[3].to_owned(),
            acolyte: values[4].to_owned(),
            referrer: values[5].to_owned(),
            initial_declared_price: values[6].to_owned(),
            consent_nonce: values[7].to_owned(),
            deadline: values[8].to_owned(),
            block_number: values[9].to_owned(),
            block_hash: values[10].to_ascii_lowercase(),
            acolyte_name: name,
            signature: values[12].to_owned(),
        }));
    }
    BrandingControlMessage::Invalid
}

fn exact_marker_fields<'a>(body: &'a str, keys: &[&str]) -> Option<Vec<&'a str>> {
    let fields = body.split(';').collect::<Vec<_>>();
    if fields.len() != keys.len() {
        return None;
    }
    fields
        .iter()
        .zip(keys)
        .map(|(field, key)| {
            let prefix = format!("{key}=");
            field
                .strip_prefix(&prefix)
                .filter(|value| !value.is_empty())
        })
        .collect()
}

fn validate_consent_matches_action(consent: &ParsedConsent, action: &BrandingAction) -> Result<()> {
    let inspection = action
        .inspection
        .as_ref()
        .context("Branding offer has no canonical inspection")?;
    ensure!(
        Address::from_str(&consent.contract)? == Address::from_str(BRANDING_CONTRACT)?,
        "Branding consent targets another contract"
    );
    ensure!(
        Address::from_str(&consent.minter)? == Address::from_str(&action.minter)?,
        "Branding consent names another minter"
    );
    ensure!(
        Address::from_str(&consent.acolyte)? == Address::from_str(&action.acolyte)?,
        "Branding consent names another acolyte"
    );
    ensure!(
        Address::from_str(&consent.referrer)? == Address::from_str(&action.referrer)?,
        "Branding consent names another referrer"
    );
    ensure!(
        consent.controller_agent_id == action.controller_agent_id
            && consent.initial_declared_price == action.initial_declared_price
            && consent.consent_nonce == inspection.consent_nonce
            && consent.deadline == inspection.deadline
            && consent.block_number == inspection.block_number
            && consent
                .block_hash
                .eq_ignore_ascii_case(&inspection.block_hash)
            && consent.acolyte_name == action.acolyte_name,
        "Branding consent does not exactly echo the durable offer"
    );
    Ok(())
}

fn offer_text(action: &BrandingAction, inspection: &InspectionBinding) -> String {
    format!(
        "i prepared an exact Base Acolyte Branding offer. review the contract, controller, referrer, price, one-use nonce, and deadline before signing, fwiend.\n[[cthuwu:branding-offer:v2;offer={};contract={};minter={};agent={};acolyte={};referrer={};treasury={};basis={};price={};upkeep={};nonce={};deadline={};block={};blockHash={};name={}]]",
        action.offer_id,
        BRANDING_CONTRACT,
        action.minter,
        action.controller_agent_id,
        action.acolyte,
        action.referrer,
        action.treasury_balance,
        action.price_basis_points,
        action.initial_declared_price,
        action.first_week_upkeep,
        inspection.consent_nonce,
        inspection.deadline,
        inspection.block_number,
        inspection.block_hash,
        encode_text_hex(&action.acolyte_name),
    )
}

fn receipt_text(action: &BrandingAction) -> String {
    let inspection = action.inspection.as_ref().expect("receipt has inspection");
    let receipt = action.receipt.as_ref().expect("receipt is present");
    format!(
        "the Branding mint and canonical Acolyte Name are verified on Base—congrats, branded acolyte! ur ‘invite an acolyte’ action now makes it easy to share ur own referral link when it feels natural, uwu.\n[[cthuwu:branding-receipt:v2;offer={};contract={};token={};agent={};acolyte={};owner={};referrer={};price={};nonce={};block={};blockHash={};name={}]]",
        action.offer_id,
        BRANDING_CONTRACT,
        receipt.token_id,
        action.controller_agent_id,
        action.acolyte,
        action.minter,
        action.referrer,
        receipt.declared_price,
        inspection.consent_nonce,
        receipt.block_number,
        receipt.block_hash,
        encode_text_hex(&action.acolyte_name),
    )
}

fn validate_inspection_result(
    result: &BrandingInspectionResult,
    action: &BrandingAction,
) -> Result<()> {
    ensure!(
        result.kind == "branding_inspection",
        "wrong Branding inspection kind"
    );
    validate_common_result(
        result.chain_id,
        &result.contract,
        &result.runtime_code_hash,
        &result.identity_registry,
        &result.uwu,
        &result.observed_block_number,
        &result.observed_block_hash,
        &result.observed_block_timestamp,
        &result.minter,
        &result.acolyte,
        &result.token_id,
        &result.controller_agent_id,
        &result.referrer,
        &result.initial_declared_price,
        &result.first_week_upkeep,
        &result.acolyte_name,
        &result.consent_nonce,
        &result.owner,
        &result.onchain_controller_agent_id,
        &result.onchain_referrer,
        &result.onchain_declared_price,
        &result.paid_through,
        result.name_trait.as_deref(),
        &result.eth_balance_wei,
        &result.eth_target_wei,
        &result.eth_shortfall_wei,
        &result.uwu_balance,
        &result.uwu_target,
        &result.uwu_shortfall_wei,
        &result.allowance,
        &result.estimated_cost_wei,
        &result.execution_gas,
        &result.l1_data_fee_wei,
        result.l1_data_fee_exact,
        &result.max_fee_per_gas_wei,
        &result.max_priority_fee_per_gas_wei,
        &result.safety_bps,
        &result.reserve_wei,
        &result.exact_operations,
        &result.conservative_operations,
        &result.pending_operations,
        action,
    )?;
    if result.branding_status == HelperBrandingStatus::Unminted {
        ensure!(
            Address::from_str(&result.owner)? == Address::ZERO
                && result.onchain_controller_agent_id == "0"
                && Address::from_str(&result.onchain_referrer)? == Address::ZERO
                && result.onchain_declared_price == "0"
                && result.paid_through == "0"
                && result.name_trait.is_none(),
            "unminted Branding inspection contains minted state"
        );
    }
    Ok(())
}

fn validate_completion_result(
    result: &BrandingCompletionResult,
    action: &BrandingAction,
) -> Result<()> {
    ensure!(
        result.kind == "branding_completion",
        "wrong Branding completion kind"
    );
    validate_common_result(
        result.chain_id,
        &result.contract,
        &result.runtime_code_hash,
        &result.identity_registry,
        &result.uwu,
        &result.observed_block_number,
        &result.observed_block_hash,
        &result.observed_block_timestamp,
        &result.minter,
        &result.acolyte,
        &result.token_id,
        &result.controller_agent_id,
        &result.referrer,
        &result.initial_declared_price,
        &result.first_week_upkeep,
        &result.acolyte_name,
        &result.consent_nonce,
        &result.owner,
        &result.onchain_controller_agent_id,
        &result.onchain_referrer,
        &result.onchain_declared_price,
        &result.paid_through,
        result.name_trait.as_deref(),
        &result.eth_balance_wei,
        &result.eth_target_wei,
        &result.eth_shortfall_wei,
        &result.uwu_balance,
        &result.uwu_target,
        &result.uwu_shortfall_wei,
        &result.allowance,
        &result.estimated_cost_wei,
        &result.execution_gas,
        &result.l1_data_fee_wei,
        result.l1_data_fee_exact,
        &result.max_fee_per_gas_wei,
        &result.max_priority_fee_per_gas_wei,
        &result.safety_bps,
        &result.reserve_wei,
        &result.exact_operations,
        &result.conservative_operations,
        &result.pending_operations,
        action,
    )?;
    let expected_nonce = &action
        .inspection
        .as_ref()
        .context("completion has no persisted offer nonce")?
        .consent_nonce;
    ensure!(
        result.consent_nonce == *expected_nonce,
        "executor returned another nonce"
    );
    validate_uint256(&result.current_consent_nonce, "current consent nonce")?;
    ensure!(
        result.transaction_hashes.len() <= 3,
        "too many Branding transactions"
    );
    let mut operations = BTreeSet::new();
    for transaction in &result.transaction_hashes {
        ensure!(
            matches!(
                transaction.operation.as_str(),
                "approve" | "mint" | "name_trait"
            ) && operations.insert(transaction.operation.as_str()),
            "invalid or duplicate Branding transaction operation"
        );
        validate_hash(&transaction.transaction_hash, "Branding transaction hash")?;
    }
    if result.disposition == HelperDisposition::Complete {
        ensure!(
            Address::from_str(&result.owner)? == Address::from_str(&action.minter)?
                && result.onchain_controller_agent_id == action.controller_agent_id
                && Address::from_str(&result.onchain_referrer)?
                    == Address::from_str(&action.referrer)?
                && result.paid_through != "0",
            "completed Branding tuple does not match durable consent"
        );
        ensure!(
            result.eth_shortfall_wei == "0"
                && result.uwu_shortfall_wei == "0"
                && result.pending_operations.is_empty(),
            "completed Branding still reports work"
        );
    } else if result.disposition == HelperDisposition::FundingRequired {
        ensure!(
            result.eth_shortfall_wei != "0" || result.uwu_shortfall_wei != "0",
            "funding-required result returned zero shortfalls"
        );
        ensure!(
            !result.pending_operations.is_empty(),
            "funding result has no work"
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_common_result(
    chain_id: u64,
    contract: &str,
    runtime_code_hash: &str,
    identity_registry: &str,
    uwu: &str,
    observed_block_number: &str,
    observed_block_hash: &str,
    observed_block_timestamp: &str,
    minter: &str,
    acolyte: &str,
    token_id: &str,
    controller_agent_id: &str,
    referrer: &str,
    initial_declared_price: &str,
    first_week_upkeep: &str,
    name: &str,
    consent_nonce: &str,
    owner: &str,
    onchain_controller_agent_id: &str,
    onchain_referrer: &str,
    onchain_declared_price: &str,
    paid_through: &str,
    name_trait: Option<&str>,
    eth_balance_wei: &str,
    eth_target_wei: &str,
    eth_shortfall_wei: &str,
    uwu_balance: &str,
    uwu_target: &str,
    uwu_shortfall_wei: &str,
    allowance: &str,
    estimated_cost_wei: &str,
    execution_gas: &str,
    l1_data_fee_wei: &str,
    _l1_data_fee_exact: bool,
    max_fee_per_gas_wei: &str,
    max_priority_fee_per_gas_wei: &str,
    safety_bps: &str,
    reserve_wei: &str,
    exact_operations: &[String],
    conservative_operations: &[String],
    pending_operations: &[String],
    action: &BrandingAction,
) -> Result<()> {
    ensure!(
        chain_id == BASE_MAINNET_CHAIN_ID,
        "Branding helper used another chain"
    );
    ensure!(
        Address::from_str(contract)? == Address::from_str(BRANDING_CONTRACT)?,
        "Branding helper used another contract"
    );
    ensure!(
        runtime_code_hash.eq_ignore_ascii_case(BRANDING_RUNTIME_CODE_HASH),
        "Branding helper observed another runtime"
    );
    ensure!(
        Address::from_str(identity_registry)? == Address::from_str(IDENTITY_REGISTRY)?,
        "Branding helper used another Identity Registry"
    );
    ensure!(
        Address::from_str(uwu)? == Address::from_str(UWU_CONTRACT)?,
        "Branding helper used another UWU token"
    );
    validate_uint256(observed_block_number, "Branding observation block")?;
    ensure!(
        observed_block_number != "0",
        "Branding observation block is zero"
    );
    validate_hash(observed_block_hash, "Branding observation block hash")?;
    decimal_u64(
        observed_block_timestamp,
        "Branding observation block timestamp",
    )?;
    ensure!(
        Address::from_str(minter)? == Address::from_str(&action.minter)?
            && Address::from_str(acolyte)? == Address::from_str(&action.acolyte)?
            && Address::from_str(referrer)? == Address::from_str(&action.referrer)?,
        "Branding helper identity binding mismatch"
    );
    validate_uint256(token_id, "Branding token ID")?;
    ensure!(
        token_id == address_token_id(Address::from_str(&action.acolyte)?),
        "Branding helper token ID is not uint160(acolyte) in decimal"
    );
    ensure!(
        controller_agent_id == action.controller_agent_id
            && initial_declared_price == action.initial_declared_price
            && first_week_upkeep == action.first_week_upkeep
            && name == action.acolyte_name,
        "Branding helper changed a durable offer field"
    );
    validate_uint256(consent_nonce, "Branding consent nonce")?;
    parse_address_allow_zero(owner, "Branding owner")?;
    validate_uint256(onchain_controller_agent_id, "on-chain controller agent ID")?;
    parse_address_allow_zero(onchain_referrer, "on-chain referrer")?;
    for (value, label) in [
        (onchain_declared_price, "on-chain declared price"),
        (paid_through, "paid-through timestamp"),
        (eth_balance_wei, "Base ETH balance"),
        (eth_target_wei, "Base ETH target"),
        (eth_shortfall_wei, "Base ETH shortfall"),
        (uwu_balance, "UWU balance"),
        (uwu_target, "UWU target"),
        (uwu_shortfall_wei, "UWU shortfall"),
        (allowance, "UWU allowance"),
        (estimated_cost_wei, "estimated Base cost"),
        (execution_gas, "execution gas"),
        (l1_data_fee_wei, "L1 data fee"),
        (max_fee_per_gas_wei, "maximum fee per gas"),
        (max_priority_fee_per_gas_wei, "maximum priority fee per gas"),
        (safety_bps, "gas safety basis points"),
        (reserve_wei, "post-operation reserve"),
    ] {
        validate_uint256(value, label)?;
    }
    if let Some(name_trait) = name_trait {
        validate_public_name(name_trait)?;
    }
    validate_operation_lists(
        exact_operations,
        conservative_operations,
        pending_operations,
    )
}

fn validate_operation_lists(
    exact: &[String],
    conservative: &[String],
    pending: &[String],
) -> Result<()> {
    for (values, label) in [
        (exact, "exact"),
        (conservative, "conservative"),
        (pending, "pending"),
    ] {
        ensure!(
            values.len() <= 3,
            "Branding {label} operation list is unbounded"
        );
        let mut unique = BTreeSet::new();
        for operation in values {
            ensure!(
                matches!(operation.as_str(), "approve" | "mint" | "name_trait")
                    && unique.insert(operation.as_str()),
                "Branding {label} operation list is invalid"
            );
        }
    }
    Ok(())
}

fn funding_text(result: &BrandingCompletionResult) -> String {
    format!(
        "ACOLYTE BRANDING REQUIRES RESOURCES\nFUND THIS EXACT BASE ADDRESS: {}\nCURRENT BASE ETH BALANCE: {} WEI\nTARGET BASE ETH BALANCE: {} WEI\nBASE ETH SHORTFALL: {} WEI\nCURRENT UWU BALANCE: {} BASE UNITS\nTARGET UWU BALANCE: {} BASE UNITS\nUWU SHORTFALL: {} BASE UNITS\nREMAINING PHASES: {}\nCHAIN: BASE MAINNET\nCHAIN ID: 8453\nWARNING: DO NOT SEND FUNDS ON ANY OTHER CHAIN.\nTHE DURABLE ACTION WILL RETRY WITHOUT REQUESTING A PRIVATE KEY.",
        result.minter,
        result.eth_balance_wei,
        result.eth_target_wei,
        result.eth_shortfall_wei,
        result.uwu_balance,
        result.uwu_target,
        result.uwu_shortfall_wei,
        result.pending_operations.join(", "),
    )
}

fn funding_fingerprint(result: &BrandingCompletionResult) -> String {
    let value = format!(
        "{}\0{}\0{}\0{}\0{}\0{}\0{}",
        result.minter,
        result.eth_balance_wei,
        result.eth_target_wei,
        result.eth_shortfall_wei,
        result.uwu_balance,
        result.uwu_target,
        result.uwu_shortfall_wei,
    );
    format!("0x{}", sha256_hex(value.as_bytes()))
}

fn delivery_commitment(kind: &str, offer_id: &str, text: &str) -> String {
    format!(
        "branding-{kind}:{offer_id}:{}",
        &sha256_hex(text.as_bytes())[..16]
    )
}

fn random_id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).context("generating a Branding offer ID")?;
    Ok(hex_encode(&bytes))
}

fn validate_offer_id(value: &str) -> Result<()> {
    ensure!(
        value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "Branding offer ID must be 16 lowercase hexadecimal bytes"
    );
    Ok(())
}

fn validate_action_id(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b":_-".contains(&byte)),
        "Branding action ID is invalid"
    );
    Ok(())
}

fn validate_inbox_id(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "Branding inbox ID must be 32 lowercase hexadecimal bytes"
    );
    Ok(())
}

fn validate_uint256(value: &str, label: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 78
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && (value == "0" || !value.starts_with('0')),
        "{label} must be a canonical decimal uint256"
    );
    if value.len() == 78 {
        const MAX: &str =
            "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        ensure!(value <= MAX, "{label} exceeds uint256");
    }
    Ok(())
}

fn validate_hash(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() == 66
            && value.starts_with("0x")
            && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{label} must be exactly 32 bytes"
    );
    Ok(())
}

fn validate_signature(value: &str) -> Result<()> {
    ensure!(
        value.starts_with("0x")
            && value.len() > 2
            && value.len().is_multiple_of(2)
            && (value.len() - 2) / 2 <= MAX_SIGNATURE_BYTES
            && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Branding signature is malformed or oversized"
    );
    Ok(())
}

fn validate_public_name(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty()
            && value.len() <= MAX_NAME_BYTES
            && !value.chars().any(char::is_control),
        "Acolyte name is empty, oversized, or contains controls"
    );
    Ok(())
}

fn parse_nonzero_address(value: &str, label: &str) -> Result<Address> {
    let address = Address::from_str(value).with_context(|| format!("invalid {label}"))?;
    ensure!(address != Address::ZERO, "{label} must not be zero");
    Ok(address)
}

fn parse_address_allow_zero(value: &str, label: &str) -> Result<Address> {
    Address::from_str(value).with_context(|| format!("invalid {label}"))
}

fn u256_decimal(value: U256) -> String {
    bytes_decimal(&value.to_be_bytes())
}

fn address_token_id(address: Address) -> String {
    bytes_decimal(address.as_bytes())
}

fn bytes_decimal(bytes: &[u8]) -> String {
    let mut digits = vec![0_u8];
    for byte in bytes {
        let mut carry = u16::from(*byte);
        for digit in digits.iter_mut().rev() {
            let value = u16::from(*digit) * 256 + carry;
            *digit = (value % 10) as u8;
            carry = value / 10;
        }
        while carry > 0 {
            digits.insert(0, (carry % 10) as u8);
            carry /= 10;
        }
    }
    let first = digits
        .iter()
        .position(|digit| *digit != 0)
        .unwrap_or(digits.len() - 1);
    digits[first..]
        .iter()
        .map(|digit| char::from(b'0' + *digit))
        .collect()
}

fn decimal_u256(value: &str) -> Result<U256> {
    validate_uint256(value, "decimal uint256")?;
    let mut bytes = [0_u8; 32];
    for digit in value.bytes() {
        let mut carry = u16::from(digit - b'0');
        for byte in bytes.iter_mut().rev() {
            let next = u16::from(*byte) * 10 + carry;
            *byte = next as u8;
            carry = next >> 8;
        }
        ensure!(carry == 0, "decimal uint256 overflowed");
    }
    Ok(U256::from_be_bytes(bytes))
}

fn decimal_u64(value: &str, label: &str) -> Result<u64> {
    validate_uint256(value, label)?;
    value
        .parse::<u64>()
        .with_context(|| format!("{label} exceeds u64"))
}

fn decimal_increment(value: &str) -> Result<String> {
    validate_uint256(value, "consent nonce")?;
    let mut digits = value.as_bytes().to_vec();
    let mut carry = true;
    for digit in digits.iter_mut().rev() {
        if !carry {
            break;
        }
        if *digit == b'9' {
            *digit = b'0';
        } else {
            *digit += 1;
            carry = false;
        }
    }
    if carry {
        digits.insert(0, b'1');
    }
    let incremented = String::from_utf8(digits).expect("decimal digits are UTF-8");
    validate_uint256(&incremented, "incremented consent nonce")?;
    Ok(incremented)
}

fn encode_text_hex(value: &str) -> String {
    format!("0x{}", hex_encode(value.as_bytes()))
}

fn decode_text_hex(value: &str) -> Result<String> {
    ensure!(value.starts_with("0x"), "encoded name has no 0x prefix");
    let bytes = decode_hex(&value[2..], MAX_NAME_BYTES)?;
    String::from_utf8(bytes).context("encoded Acolyte name is not UTF-8")
}

fn decode_hex(value: &str, maximum: usize) -> Result<Vec<u8>> {
    ensure!(
        value.len().is_multiple_of(2) && value.len() / 2 <= maximum,
        "hex value is malformed or oversized"
    );
    let mut output = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().as_chunks::<2>().0 {
        let high = hex_nibble(pair[0]).context("non-hexadecimal digit")?;
        let low = hex_nibble(pair[1]).context("non-hexadecimal digit")?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn hex_encode(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn sha256_hex(value: &[u8]) -> String {
    hex_encode(&Sha256::digest(value))
}

fn bounded_diagnostic(value: &str, maximum: usize) -> String {
    value
        .chars()
        .map(|character| {
            if matches!(character, '\r' | '\n') {
                ' '
            } else {
                character
            }
        })
        .take(maximum)
        .collect()
}

fn unix_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates Unix epoch")?
        .as_secs())
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing symlinked Branding state path {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

// Ethereum Keccak-256 (legacy padding 0x01, not SHA3's 0x06). This small primitive avoids a new
// locked dependency solely to reproduce the public browser name table.
fn keccak256(input: &[u8]) -> [u8; 32] {
    const RATE: usize = 136;
    let mut state = [0_u64; 25];
    let (chunks, remainder) = input.as_chunks::<RATE>();
    for block in chunks {
        absorb_keccak_block(&mut state, block);
        keccak_f1600(&mut state);
    }
    let mut last = [0_u8; RATE];
    last[..remainder.len()].copy_from_slice(remainder);
    last[remainder.len()] ^= 0x01;
    last[RATE - 1] ^= 0x80;
    absorb_keccak_block(&mut state, &last);
    keccak_f1600(&mut state);
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = (state[index / 8] >> ((index % 8) * 8)) as u8;
    }
    output
}

fn absorb_keccak_block(state: &mut [u64; 25], block: &[u8]) {
    for (index, byte) in block.iter().enumerate() {
        state[index / 8] ^= u64::from(*byte) << ((index % 8) * 8);
    }
}

fn keccak_f1600(state: &mut [u64; 25]) {
    const RC: [u64; 24] = [
        0x0000000000000001,
        0x0000000000008082,
        0x800000000000808a,
        0x8000000080008000,
        0x000000000000808b,
        0x0000000080000001,
        0x8000000080008081,
        0x8000000000008009,
        0x000000000000008a,
        0x0000000000000088,
        0x0000000080008009,
        0x000000008000000a,
        0x000000008000808b,
        0x800000000000008b,
        0x8000000000008089,
        0x8000000000008003,
        0x8000000000008002,
        0x8000000000000080,
        0x000000000000800a,
        0x800000008000000a,
        0x8000000080008081,
        0x8000000000008080,
        0x0000000080000001,
        0x8000000080008008,
    ];
    const ROTATION: [u32; 25] = [
        0, 1, 62, 28, 27, 36, 44, 6, 55, 20, 3, 10, 43, 25, 39, 41, 45, 15, 21, 8, 18, 2, 61, 56,
        14,
    ];
    for round in RC {
        let mut c = [0_u64; 5];
        for x in 0..5 {
            c[x] = state[x] ^ state[x + 5] ^ state[x + 10] ^ state[x + 15] ^ state[x + 20];
        }
        let mut d = [0_u64; 5];
        for x in 0..5 {
            d[x] = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
        }
        for y in 0..5 {
            for x in 0..5 {
                state[x + 5 * y] ^= d[x];
            }
        }
        let mut b = [0_u64; 25];
        for y in 0..5 {
            for x in 0..5 {
                b[y + 5 * ((2 * x + 3 * y) % 5)] =
                    state[x + 5 * y].rotate_left(ROTATION[x + 5 * y]);
            }
        }
        for y in 0..5 {
            for x in 0..5 {
                state[x + 5 * y] =
                    b[x + 5 * y] ^ ((!b[(x + 1) % 5 + 5 * y]) & b[(x + 2) % 5 + 5 * y]);
            }
        }
        state[0] ^= round;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(value: &str) -> Address {
        Address::from_str(value).unwrap()
    }

    #[test]
    fn default_quote_is_ten_percent_of_current_treasury() {
        let quote = quote_initial_branding(
            U256::from_u64(1_000_000),
            DEFAULT_INITIAL_PRICE_BASIS_POINTS,
        )
        .unwrap();
        assert_eq!(quote.initial_declared_price, U256::from_u64(100_000));
        assert_eq!(quote.first_week_upkeep, U256::from_u64(100));
    }

    #[test]
    fn upkeep_matches_contract_upward_rounding() {
        let quote =
            quote_initial_branding(U256::from_u64(10), DEFAULT_INITIAL_PRICE_BASIS_POINTS).unwrap();
        assert_eq!(quote.initial_declared_price, U256::from_u64(1));
        assert_eq!(quote.first_week_upkeep, U256::from_u64(1));
    }

    #[test]
    fn adjustments_are_bounded_and_zero_quotes_are_rejected() {
        assert!(quote_initial_branding(U256::from_u64(100), 499).is_err());
        assert!(quote_initial_branding(U256::from_u64(100), 2_001).is_err());
        assert!(quote_initial_branding(U256::ZERO, DEFAULT_INITIAL_PRICE_BASIS_POINTS).is_err());
    }

    #[test]
    fn rust_name_derivation_matches_the_frozen_browser_vectors() {
        assert_eq!(
            acolyte_name(address("0x0000000000000000000000000000000000000001")),
            "Broughton-Arbuthnot of Marshborough"
        );
        assert_eq!(
            acolyte_name(address("0x1111111111111111111111111111111111111111")),
            "Ainsworth-Clavering of Ambercroft"
        );
        assert_eq!(
            hex_encode(&keccak256(b"")),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn token_ids_are_decimal_uint160_values_not_hex_quantities() {
        assert_eq!(
            address_token_id(address("0x0000000000000000000000000000000000000001")),
            "1"
        );
        assert_eq!(
            address_token_id(address("0xffffffffffffffffffffffffffffffffffffffff")),
            "1461501637330902918203684832716283019655932542975"
        );
    }

    #[test]
    fn strict_controls_reject_prose_and_unknown_or_reordered_fields() {
        assert_eq!(
            parse_branding_control("I accept the Branding offer"),
            BrandingControlMessage::NotBranding
        );
        assert_eq!(
            parse_branding_control(
                " [[cthuwu:branding-decline:v2;offer=0123456789abcdef0123456789abcdef]]"
            ),
            BrandingControlMessage::Invalid
        );
        assert_eq!(
            parse_branding_control(
                "[[cthuwu:branding-decline:v2;offer=0123456789abcdef0123456789abcdef;extra=1]]"
            ),
            BrandingControlMessage::Invalid
        );
        assert_eq!(
            parse_branding_control(
                "[[cthuwu:branding-request:v2;referrer=0x2222222222222222222222222222222222222222;name=0x41696e73776f7274682d436c61766572696e67206f6620416d62657263726f6674]]"
            ),
            BrandingControlMessage::Request {
                referrer: "0x2222222222222222222222222222222222222222".to_owned(),
                name: "Ainsworth-Clavering of Ambercroft".to_owned(),
            }
        );
        assert_eq!(
            parse_branding_control("[[CTHUWU:BRANDING-CONSENT:V2;signature=0xdeadbeef]]"),
            BrandingControlMessage::Invalid
        );
    }

    #[test]
    fn persisted_state_rejects_a_forged_name_and_hex_token_id() {
        let root = tempfile::tempdir().unwrap();
        let minter = address("0x1111111111111111111111111111111111111111");
        let store = BrandingStore::new(root.path()).unwrap();
        let mut snapshot = BrandingSnapshot {
            version: SNAPSHOT_VERSION,
            cursor: 0,
            actions: Vec::new(),
            updated_at_unix: 1,
        };
        snapshot.actions.push(BrandingAction {
            offer_id: "0123456789abcdef0123456789abcdef".to_owned(),
            inbox_id: "a".repeat(64),
            acolyte: "0x0000000000000000000000000000000000000001".to_owned(),
            minter: minter.to_string(),
            controller_agent_id: "7".to_owned(),
            referrer: minter.to_string(),
            treasury_balance: "1000".to_owned(),
            price_basis_points: 1000,
            initial_declared_price: "100".to_owned(),
            first_week_upkeep: "1".to_owned(),
            acolyte_name: "forged".to_owned(),
            phase: BrandingPhase::Completed,
            observed_branding_status: Some(HelperBrandingStatus::Active),
            inspection: Some(InspectionBinding {
                consent_nonce: "0".to_owned(),
                deadline: "2000".to_owned(),
                block_number: "1".to_owned(),
                block_hash: format!("0x{}", "1".repeat(64)),
                block_timestamp: "200".to_owned(),
            }),
            consent: None,
            receipt: Some(ReceiptBinding {
                token_id: "0x1".to_owned(),
                declared_price: "100".to_owned(),
                block_number: "2".to_owned(),
                block_hash: format!("0x{}", "2".repeat(64)),
            }),
            completion_action_id: Some(
                "branding-complete:0123456789abcdef0123456789abcdef".to_owned(),
            ),
            pending_delivery: None,
            last_funding_fingerprint: None,
            last_funding_notice_unix: None,
            attempt_count: 1,
            next_attempt_unix: u64::MAX,
            last_error: None,
            created_at_unix: 1,
            updated_at_unix: 2,
        });
        assert!(store.save(&snapshot, minter).is_err());
        snapshot.actions[0].acolyte_name =
            acolyte_name(address("0x0000000000000000000000000000000000000001"));
        assert!(store.save(&snapshot, minter).is_err());
        snapshot.actions[0].receipt.as_mut().unwrap().token_id = "1".to_owned();
        store.save(&snapshot, minter).unwrap();
        snapshot.actions[0].inspection.as_mut().unwrap().deadline = "2001".to_owned();
        assert!(store.save(&snapshot, minter).is_err());
        snapshot.actions[0].inspection.as_mut().unwrap().deadline = "2000".to_owned();
        snapshot.actions[0].initial_declared_price = "101".to_owned();
        assert!(store.save(&snapshot, minter).is_err());
    }

    #[test]
    fn an_elapsed_delivered_offer_does_not_suppress_a_fresh_offer() {
        struct NeverGateway;
        #[async_trait]
        impl Erc8004Gateway for NeverGateway {
            async fn invoke(
                &self,
                _action_id: &str,
                _operation: serde_json::Value,
            ) -> Result<serde_json::Value> {
                bail!("unused")
            }
        }
        let root = tempfile::tempdir().unwrap();
        let minter = address("0x1111111111111111111111111111111111111111");
        let store = BrandingStore::new(root.path()).unwrap();
        let state = store.load_or_create(minter, 1).unwrap();
        let mut runtime = BrandingRuntime {
            store,
            state,
            minter,
            gateway: Arc::new(NeverGateway),
        };
        let quote = quote_initial_branding(U256::from_u64(1_000), 1_000).unwrap();
        let inbox = "a".repeat(64);
        let acolyte = address("0x0000000000000000000000000000000000000001");
        runtime
            .enqueue_offer(&inbox, acolyte, "7", minter, quote, 1)
            .unwrap();
        runtime.state.actions[0].inspection = Some(InspectionBinding {
            consent_nonce: "0".to_owned(),
            deadline: "1802".to_owned(),
            block_number: "1".to_owned(),
            block_hash: format!("0x{}", "1".repeat(64)),
            block_timestamp: "2".to_owned(),
        });
        runtime.state.actions[0].phase = BrandingPhase::Offered;
        assert!(
            runtime
                .enqueue_offer(&inbox, acolyte, "7", minter, quote, 1803)
                .unwrap()
        );
        assert_eq!(runtime.state.actions.len(), 2);
        assert_eq!(runtime.state.actions[0].phase, BrandingPhase::Expired);
        assert_eq!(runtime.state.actions[1].phase, BrandingPhase::Requested);
    }

    #[test]
    fn a_changed_canonical_treasury_is_requoted_durably_before_delivery() {
        struct NeverGateway;
        #[async_trait]
        impl Erc8004Gateway for NeverGateway {
            async fn invoke(
                &self,
                _action_id: &str,
                _operation: serde_json::Value,
            ) -> Result<serde_json::Value> {
                bail!("unused")
            }
        }
        let root = tempfile::tempdir().unwrap();
        let minter = address("0x1111111111111111111111111111111111111111");
        let store = BrandingStore::new(root.path()).unwrap();
        let state = store.load_or_create(minter, 1).unwrap();
        let mut runtime = BrandingRuntime {
            store,
            state,
            minter,
            gateway: Arc::new(NeverGateway),
        };
        let quote = quote_initial_branding(U256::from_u64(1_000), 1_000).unwrap();
        runtime
            .enqueue_offer(
                &"a".repeat(64),
                address("0x0000000000000000000000000000000000000001"),
                "7",
                minter,
                quote,
                2,
            )
            .unwrap();
        runtime.refresh_requested_quote(0, "2000", 3).unwrap();
        let restored = runtime.store.load_or_create(minter, 4).unwrap();
        assert_eq!(restored.actions[0].phase, BrandingPhase::Requested);
        assert_eq!(restored.actions[0].treasury_balance, "2000");
        assert_eq!(restored.actions[0].initial_declared_price, "200");
        assert_eq!(restored.actions[0].first_week_upkeep, "1");
        assert!(restored.actions[0].inspection.is_none());
        assert!(restored.actions[0].pending_delivery.is_none());
    }

    #[test]
    fn an_expired_consent_can_be_scrubbed_and_replaced_by_one_fresh_action() {
        struct NeverGateway;
        #[async_trait]
        impl Erc8004Gateway for NeverGateway {
            async fn invoke(
                &self,
                _action_id: &str,
                _operation: serde_json::Value,
            ) -> Result<serde_json::Value> {
                bail!("unused")
            }
        }
        let root = tempfile::tempdir().unwrap();
        let minter = address("0x1111111111111111111111111111111111111111");
        let acolyte = address("0x0000000000000000000000000000000000000001");
        let new_referrer = address("0x2222222222222222222222222222222222222222");
        let store = BrandingStore::new(root.path()).unwrap();
        let state = store.load_or_create(minter, 1).unwrap();
        let mut runtime = BrandingRuntime {
            store,
            state,
            minter,
            gateway: Arc::new(NeverGateway),
        };
        let quote = quote_initial_branding(U256::from_u64(1_000), 1_000).unwrap();
        let inbox = "a".repeat(64);
        runtime
            .enqueue_offer(&inbox, acolyte, "7", minter, quote, 1)
            .unwrap();
        runtime.state.actions[0].inspection = Some(InspectionBinding {
            consent_nonce: "0".to_owned(),
            deadline: "1802".to_owned(),
            block_number: "1".to_owned(),
            block_hash: format!("0x{}", "1".repeat(64)),
            block_timestamp: "2".to_owned(),
        });
        runtime.state.actions[0].consent = Some(ConsentBinding {
            signature: "0x11".to_owned(),
        });
        let offer_id = runtime.state.actions[0].offer_id.clone();
        runtime.state.actions[0].completion_action_id =
            Some(format!("branding-complete:{offer_id}"));
        runtime.state.actions[0].phase = BrandingPhase::Consented;
        let changed_quote = quote_initial_branding(U256::from_u64(2_000), 1_000).unwrap();
        assert!(
            !runtime
                .enqueue_offer(&inbox, acolyte, "7", minter, changed_quote, 2)
                .unwrap()
        );
        assert_eq!(runtime.state.actions[0].phase, BrandingPhase::Consented);
        let name = acolyte_name(acolyte);
        assert!(
            runtime
                .replace_referrer_from_request(&inbox, acolyte, "7", new_referrer, &name, 1802,)
                .is_err()
        );
        assert_eq!(runtime.state.actions[0].phase, BrandingPhase::Consented);
        assert!(
            runtime
                .replace_referrer_from_request(&inbox, acolyte, "7", new_referrer, &name, 1803,)
                .unwrap()
        );
        let restored = runtime.store.load_or_create(minter, 1803).unwrap();
        assert_eq!(restored.actions.len(), 2);
        assert_eq!(restored.actions[0].phase, BrandingPhase::Superseded);
        assert!(restored.actions[0].consent.is_none());
        assert!(restored.actions[0].receipt.is_none());
        assert!(restored.actions[0].completion_action_id.is_none());
        assert_eq!(restored.actions[1].phase, BrandingPhase::Requested);
        assert_eq!(restored.actions[1].referrer, new_referrer.to_string());
        assert_eq!(
            restored
                .actions
                .iter()
                .filter(|action| !action.phase.terminal())
                .count(),
            1
        );

        let latest = 1;
        runtime.state.actions[latest].inspection = Some(InspectionBinding {
            consent_nonce: "0".to_owned(),
            deadline: "1802".to_owned(),
            block_number: "1".to_owned(),
            block_hash: format!("0x{}", "1".repeat(64)),
            block_timestamp: "2".to_owned(),
        });
        runtime.state.actions[latest].consent = Some(ConsentBinding {
            signature: "0x11".to_owned(),
        });
        let signed_price = runtime.state.actions[latest].initial_declared_price.clone();
        runtime.state.actions[latest].receipt = Some(ReceiptBinding {
            token_id: address_token_id(acolyte),
            declared_price: signed_price,
            block_number: "2".to_owned(),
            block_hash: format!("0x{}", "2".repeat(64)),
        });
        let offer_id = runtime.state.actions[latest].offer_id.clone();
        runtime.state.actions[latest].completion_action_id =
            Some(format!("branding-complete:{offer_id}"));
        runtime.state.actions[latest].phase = BrandingPhase::ReceiptPendingDelivery;
        let receipt = receipt_text(&runtime.state.actions[latest]);
        runtime.state.actions[latest].pending_delivery = Some(PendingDelivery {
            kind: PendingDeliveryKind::Receipt,
            commitment: delivery_commitment("receipt", &offer_id, &receipt),
            text: receipt,
            fingerprint: None,
        });
        assert!(
            runtime
                .replace_referrer_from_request(&inbox, acolyte, "7", new_referrer, &name, 2_000,)
                .is_err()
        );
        assert_eq!(
            runtime.state.actions[latest].phase,
            BrandingPhase::ReceiptPendingDelivery
        );
        assert!(runtime.state.actions[latest].receipt.is_some());
        assert!(runtime.state.actions[latest].pending_delivery.is_some());
        assert!(
            !runtime
                .enqueue_offer(&inbox, acolyte, "7", minter, changed_quote, 2_001)
                .unwrap()
        );
        assert_eq!(runtime.state.actions.len(), 2);
        assert_eq!(
            runtime.state.actions[latest].phase,
            BrandingPhase::ReceiptPendingDelivery
        );
    }

    #[test]
    fn fair_cursor_advances_before_retrying_a_failing_first_action() {
        struct NeverGateway;
        #[async_trait]
        impl Erc8004Gateway for NeverGateway {
            async fn invoke(
                &self,
                _action_id: &str,
                _operation: serde_json::Value,
            ) -> Result<serde_json::Value> {
                bail!("unused")
            }
        }
        let root = tempfile::tempdir().unwrap();
        let minter = address("0x1111111111111111111111111111111111111111");
        let store = BrandingStore::new(root.path()).unwrap();
        let state = store.load_or_create(minter, 1).unwrap();
        let mut runtime = BrandingRuntime {
            store,
            state,
            minter,
            gateway: Arc::new(NeverGateway),
        };
        let quote = quote_initial_branding(U256::from_u64(1_000), 1_000).unwrap();
        runtime
            .enqueue_offer(
                &"a".repeat(64),
                address("0x0000000000000000000000000000000000000001"),
                "7",
                minter,
                quote,
                2,
            )
            .unwrap();
        runtime
            .enqueue_offer(
                &"b".repeat(64),
                address("0x0000000000000000000000000000000000000002"),
                "7",
                minter,
                quote,
                2,
            )
            .unwrap();
        let first = runtime.select_due_fairly(2).unwrap().unwrap();
        let second = runtime.select_due_fairly(2).unwrap().unwrap();
        assert_eq!((first, second), (0, 1));
    }

    #[test]
    fn explicit_decline_and_success_both_stop_branding_followups() {
        struct NeverGateway;
        #[async_trait]
        impl Erc8004Gateway for NeverGateway {
            async fn invoke(
                &self,
                _action_id: &str,
                _operation: serde_json::Value,
            ) -> Result<serde_json::Value> {
                bail!("unused")
            }
        }
        let root = tempfile::tempdir().unwrap();
        let minter = address("0x1111111111111111111111111111111111111111");
        let store = BrandingStore::new(root.path()).unwrap();
        let state = store.load_or_create(minter, 1).unwrap();
        let mut runtime = BrandingRuntime {
            store,
            state,
            minter,
            gateway: Arc::new(NeverGateway),
        };
        let quote = quote_initial_branding(U256::from_u64(1_000), 1_000).unwrap();
        let declined_inbox = "a".repeat(64);
        let declined_acolyte = address("0x0000000000000000000000000000000000000001");
        runtime
            .enqueue_offer(&declined_inbox, declined_acolyte, "7", minter, quote, 2)
            .unwrap();
        let offer_id = runtime.state.actions[0].offer_id.clone();
        runtime
            .decline(&declined_inbox, declined_acolyte, &offer_id, 3)
            .unwrap();
        assert_eq!(
            runtime.state_for(
                &declined_inbox,
                declined_acolyte,
                3 + 10 * BRANDING_FOLLOWUP_COOLDOWN_SECONDS,
            ),
            ("declined", false)
        );

        let branded_inbox = "b".repeat(64);
        let branded_acolyte = address("0x0000000000000000000000000000000000000002");
        runtime
            .enqueue_offer(&branded_inbox, branded_acolyte, "7", minter, quote, 4)
            .unwrap();
        let latest = runtime.state.actions.len() - 1;
        runtime.state.actions[latest].phase = BrandingPhase::Completed;
        runtime.state.actions[latest].observed_branding_status = Some(HelperBrandingStatus::Active);
        assert_eq!(
            runtime.state_for(
                &branded_inbox,
                branded_acolyte,
                4 + 10 * BRANDING_FOLLOWUP_COOLDOWN_SECONDS,
            ),
            ("branded", false)
        );
        assert_eq!(
            runtime.durable_branding_updates().unwrap()[0].2,
            DurableBrandingState::Branded
        );
    }
}
