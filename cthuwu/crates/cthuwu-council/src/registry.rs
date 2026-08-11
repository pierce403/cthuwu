use cthuwu_protocol::{CthulhuId, RegistryRef, TentacleId, XmtpInboxRef};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    str::FromStr,
    sync::Arc,
};
use thiserror::Error;

const MAX_ENDPOINTS: usize = 32;
const MAX_AGENTS: usize = 4_096;
const MAX_CAPABILITY_REFS: usize = 64;
const MAX_SIGNALS: usize = 64;
const MAX_REFERENCE_LENGTH: usize = 512;
const MAX_AGENT_URI_LENGTH: usize = 4_096;
const LOCAL_REGISTRY_SCHEMA_VERSION: u32 = 2;
const U256_MAX_DECIMAL: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639935";

pub const BASE_MAINNET_CHAIN_ID: u64 = 8_453;
pub const BASE_IDENTITY_REGISTRY_ADDRESS: &str = "0x8004A169FB4a3325136EB29fA0ceB6D2e539a432";
pub const BASE_REPUTATION_REGISTRY_ADDRESS: &str = "0x8004BAa17C55a88189AE136b182e5fdA19dE9b63";
pub const BASE_IDENTITY_IMPLEMENTATION_ADDRESS: &str = "0x7274e874CA62410a93Bd8bf61c69d8045E399c02";
pub const BASE_REPUTATION_IMPLEMENTATION_ADDRESS: &str =
    "0x16e0FA7f7C56B9a767E34B192B51f921BE31dA34";
pub const ERC8004_REGISTRATION_REVISION: &str = "registration-v1";
pub const ERC8004_IDENTITY_CONTRACT_VERSION: &str = "2.0.0";
pub const ERC8004_REPUTATION_CONTRACT_VERSION: &str = "2.0.0";
pub const CTHUWU_ALLEGIANCE_KEY: &str = "cthuwu.allegiance";
pub const CTHUWU_ALLEGIANCE_VALUE: &[u8] = b"uwu-tentacle-v1";
pub const CTHUWU_PROTOCOL_KEY: &str = "cthuwu.protocol";
pub const CTHUWU_PROTOCOL_VALUE: &[u8] = b"1";
pub const CTHUWU_TENTACLE_ID_KEY: &str = "cthuwu.tentacle-id";

#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EvmAddress([u8; 20]);

impl EvmAddress {
    pub const ZERO: Self = Self([0; 20]);

    pub const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    pub fn is_zero(self) -> bool {
        self == Self::ZERO
    }
}

impl FromStr for EvmAddress {
    type Err = RegistryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some(encoded) = value.strip_prefix("0x") else {
            return Err(RegistryError::Invalid("address must start with 0x"));
        };
        if encoded.len() != 40 {
            return Err(RegistryError::Invalid(
                "address must contain exactly 40 hexadecimal digits",
            ));
        }
        let mut bytes = [0_u8; 20];
        for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
            let high = decode_hex_digit(pair[0])?;
            let low = decode_hex_digit(pair[1])?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for EvmAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("0x")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for EvmAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Serialize for EvmAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for EvmAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

fn decode_hex_digit(value: u8) -> Result<u8, RegistryError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(RegistryError::Invalid(
            "address contains a non-hexadecimal digit",
        )),
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Erc8004AgentId(String);

impl Erc8004AgentId {
    pub fn new(value: impl Into<String>) -> Result<Self, RegistryError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > U256_MAX_DECIMAL.len()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
            || (value.len() == U256_MAX_DECIMAL.len() && value.as_str() > U256_MAX_DECIMAL)
        {
            return Err(RegistryError::Invalid(
                "ERC-8004 agent ID must be a canonical uint256 decimal string",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Erc8004AgentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Erc8004AgentId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// Durable local selection of an ERC-8004 identity for one Tentacle.
///
/// This contains no signing material. The wallet is the expected public Base address of the
/// persistent XMTP identity and is checked against current on-chain `agentWallet` state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Erc8004Binding {
    pub tentacle_id: TentacleId,
    pub agent_id: Erc8004AgentId,
    pub tentacle_wallet: EvmAddress,
}

impl Erc8004Binding {
    pub fn validate(&self) -> Result<(), RegistryError> {
        if self.tentacle_wallet.is_zero() {
            return Err(RegistryError::Invalid(
                "persisted Tentacle wallet must be nonzero",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryEndpoint {
    pub tentacle_id: TentacleId,
    pub xmtp_inbox: XmtpInboxRef,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrustSignal {
    /// Names the source or method; it is never interpreted as universal truth.
    pub provenance: String,
    pub kind: String,
    pub value: i32,
    pub observed_at: u64,
    pub evidence_ref: Option<String>,
}

impl TrustSignal {
    pub fn validate(&self) -> Result<(), RegistryError> {
        validate_label(&self.provenance, "trust provenance", 128)?;
        validate_label(&self.kind, "trust signal kind", 64)?;
        if !(-1_000..=1_000).contains(&self.value) {
            return Err(RegistryError::Invalid("trust value is out of bounds"));
        }
        if self
            .evidence_ref
            .as_ref()
            .is_some_and(|value| value.len() > MAX_REFERENCE_LENGTH)
        {
            return Err(RegistryError::Invalid(
                "trust evidence reference is too long",
            ));
        }
        Ok(())
    }
}

/// Public registry state for one durable Tentacle identity.
///
/// Runtime incarnations, Cthulhu-wide state, DMs, contact memory, and load have no representation
/// here. `active` is adapter policy derived from current public registry state; for ERC-8004 it
/// requires exact current allegiance and the expected nonzero verified `agentWallet`. Protocol
/// metadata is reported independently and does not create or erase voluntary membership.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisteredTentacle {
    pub id: TentacleId,
    pub display_name: String,
    pub registry_ref: Option<RegistryRef>,
    pub endpoints: Vec<RegistryEndpoint>,
    pub capability_refs: Vec<String>,
    pub trust_signals: Vec<TrustSignal>,
    pub active: bool,
    pub metadata_version: u64,
}

impl RegisteredTentacle {
    pub fn validate(&self) -> Result<(), RegistryError> {
        validate_label(&self.display_name, "display name", 128)?;
        if self.metadata_version == 0 {
            return Err(RegistryError::Invalid("metadata version must be positive"));
        }
        if let Some(reference) = &self.registry_ref {
            reference
                .validate()
                .map_err(|_| RegistryError::Invalid("registry reference is invalid"))?;
        }
        if self.endpoints.len() > MAX_ENDPOINTS {
            return Err(RegistryError::Invalid("too many registered endpoints"));
        }
        if self.capability_refs.len() > MAX_CAPABILITY_REFS {
            return Err(RegistryError::Invalid("too many capability references"));
        }
        if self.trust_signals.len() > MAX_SIGNALS {
            return Err(RegistryError::Invalid("too many trust signals"));
        }
        let mut inboxes = BTreeSet::new();
        for endpoint in &self.endpoints {
            if endpoint.tentacle_id != self.id {
                return Err(RegistryError::Invalid(
                    "registry endpoint belongs to a different Tentacle",
                ));
            }
            if !inboxes.insert(endpoint.xmtp_inbox.clone()) {
                return Err(RegistryError::Invalid("duplicate registry endpoint"));
            }
        }
        for reference in &self.capability_refs {
            validate_label(reference, "capability reference", MAX_REFERENCE_LENGTH)?;
        }
        for signal in &self.trust_signals {
            signal.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("invalid registry data: {0}")]
    Invalid(&'static str),
    #[error("Tentacle is not registered")]
    NotFound,
    #[error("registry update is stale")]
    StaleUpdate,
    #[error("registry backend is unavailable: {0}")]
    BackendUnavailable(&'static str),
    #[error("ERC-8004 deployment verification failed: {0}")]
    DeploymentMismatch(&'static str),
    #[error("ERC-8004 registry adapter is read-only; registration uses the narrow signer workflow")]
    ReadOnly,
}

pub trait AgentRegistry {
    fn resolve(&self, id: &TentacleId) -> Result<RegisteredTentacle, RegistryError>;
    fn register_or_update(&mut self, agent: RegisteredTentacle) -> Result<(), RegistryError>;
    fn endpoints(&self, id: &TentacleId) -> Result<Vec<RegistryEndpoint>, RegistryError>;
    fn capability_references(&self, id: &TentacleId) -> Result<Vec<String>, RegistryError>;
    fn trust_signals(&self, id: &TentacleId) -> Result<Vec<TrustSignal>, RegistryError>;
    fn verify_endpoint_association(
        &self,
        id: &TentacleId,
        tentacle_id: &TentacleId,
        inbox: &XmtpInboxRef,
    ) -> Result<bool, RegistryError>;
    fn is_active(&self, id: &TentacleId) -> Result<bool, RegistryError>;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyRegistryMigration {
    pub source_schema_version: u32,
    pub legacy_cthulhu_ids: Vec<CthulhuId>,
}

#[derive(Clone, Debug, Default)]
pub struct LocalRegistry {
    tentacles: BTreeMap<TentacleId, RegisteredTentacle>,
    migration: Option<LegacyRegistryMigration>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LocalRegistryV2 {
    schema_version: u32,
    tentacles: BTreeMap<TentacleId, RegisteredTentacle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    migration: Option<LegacyRegistryMigration>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyRegisteredCthulhuV1 {
    id: CthulhuId,
    display_name: String,
    registry_ref: Option<RegistryRef>,
    endpoints: Vec<RegistryEndpoint>,
    capability_refs: Vec<String>,
    trust_signals: Vec<TrustSignal>,
    active: bool,
    metadata_version: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalRegistryV1 {
    agents: BTreeMap<CthulhuId, LegacyRegisteredCthulhuV1>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LocalRegistryWire {
    V2(LocalRegistryV2),
    V1(LocalRegistryV1),
}

impl Serialize for LocalRegistry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        LocalRegistryV2 {
            schema_version: LOCAL_REGISTRY_SCHEMA_VERSION,
            tentacles: self.tentacles.clone(),
            migration: self.migration.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LocalRegistry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match LocalRegistryWire::deserialize(deserializer)? {
            LocalRegistryWire::V2(wire) => {
                if wire.schema_version != LOCAL_REGISTRY_SCHEMA_VERSION {
                    return Err(de::Error::custom(
                        "unsupported LocalRegistry schema version",
                    ));
                }
                let registry = Self {
                    tentacles: wire.tentacles,
                    migration: wire.migration,
                };
                registry
                    .validate_loaded_state()
                    .map_err(de::Error::custom)?;
                Ok(registry)
            }
            LocalRegistryWire::V1(wire) => Self::migrate_v1(wire).map_err(de::Error::custom),
        }
    }
}

impl LocalRegistry {
    pub fn records(&self) -> impl Iterator<Item = &RegisteredTentacle> {
        self.tentacles.values()
    }

    pub fn migration(&self) -> Option<&LegacyRegistryMigration> {
        self.migration.as_ref()
    }

    fn migrate_v1(legacy: LocalRegistryV1) -> Result<Self, RegistryError> {
        if legacy.agents.len() > MAX_AGENTS {
            return Err(RegistryError::Invalid("too many legacy registered agents"));
        }
        let mut tentacles = BTreeMap::new();
        let mut legacy_cthulhu_ids = Vec::with_capacity(legacy.agents.len());
        for (key, agent) in legacy.agents {
            if key != agent.id {
                return Err(RegistryError::Invalid(
                    "legacy registry key and Cthulhu ID differ",
                ));
            }
            if agent.endpoints.len() != 1 {
                return Err(RegistryError::Invalid(
                    "legacy Cthulhu registry records require exactly one Tentacle for migration",
                ));
            }
            let id = agent.endpoints[0].tentacle_id.clone();
            let migrated = RegisteredTentacle {
                id: id.clone(),
                display_name: agent.display_name,
                registry_ref: agent.registry_ref,
                endpoints: agent.endpoints,
                capability_refs: agent.capability_refs,
                trust_signals: agent.trust_signals,
                active: agent.active,
                metadata_version: agent.metadata_version,
            };
            migrated.validate()?;
            if tentacles.insert(id, migrated).is_some() {
                return Err(RegistryError::Invalid(
                    "legacy registry maps multiple coordination profiles to one Tentacle",
                ));
            }
            legacy_cthulhu_ids.push(key);
        }
        legacy_cthulhu_ids.sort();
        let registry = Self {
            tentacles,
            migration: Some(LegacyRegistryMigration {
                source_schema_version: 1,
                legacy_cthulhu_ids,
            }),
        };
        registry.validate_loaded_state()?;
        Ok(registry)
    }

    /// Revalidates all invariants after loading untrusted durable state.
    pub fn validate_loaded_state(&self) -> Result<(), RegistryError> {
        if self.tentacles.len() > MAX_AGENTS {
            return Err(RegistryError::Invalid("too many registered Tentacles"));
        }
        if let Some(migration) = &self.migration {
            if migration.source_schema_version != 1
                || (migration.legacy_cthulhu_ids.is_empty() && !self.tentacles.is_empty())
            {
                return Err(RegistryError::Invalid(
                    "legacy registry migration provenance is invalid",
                ));
            }
            let unique = migration.legacy_cthulhu_ids.iter().collect::<BTreeSet<_>>();
            if unique.len() != migration.legacy_cthulhu_ids.len() {
                return Err(RegistryError::Invalid(
                    "legacy registry migration provenance contains duplicates",
                ));
            }
        }
        let mut inbox_owners = BTreeMap::new();
        for (key, agent) in &self.tentacles {
            if key != &agent.id {
                return Err(RegistryError::Invalid(
                    "registry key and Tentacle ID differ",
                ));
            }
            agent.validate()?;
            for endpoint in &agent.endpoints {
                if inbox_owners
                    .insert(endpoint.xmtp_inbox.clone(), agent.id.clone())
                    .is_some()
                {
                    return Err(RegistryError::Invalid(
                        "XMTP endpoint association must be unique",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl AgentRegistry for LocalRegistry {
    fn resolve(&self, id: &TentacleId) -> Result<RegisteredTentacle, RegistryError> {
        self.tentacles
            .get(id)
            .cloned()
            .ok_or(RegistryError::NotFound)
    }

    fn register_or_update(&mut self, agent: RegisteredTentacle) -> Result<(), RegistryError> {
        agent.validate()?;
        if self.tentacles.len() >= MAX_AGENTS && !self.tentacles.contains_key(&agent.id) {
            return Err(RegistryError::Invalid("too many registered Tentacles"));
        }
        if self
            .tentacles
            .get(&agent.id)
            .is_some_and(|current| current.metadata_version >= agent.metadata_version)
        {
            return Err(RegistryError::StaleUpdate);
        }
        if self.tentacles.values().any(|other| {
            other.id != agent.id
                && other.endpoints.iter().any(|existing| {
                    agent
                        .endpoints
                        .iter()
                        .any(|incoming| existing.xmtp_inbox == incoming.xmtp_inbox)
                })
        }) {
            return Err(RegistryError::Invalid(
                "XMTP inbox is already associated with another Tentacle",
            ));
        }
        self.tentacles.insert(agent.id.clone(), agent);
        Ok(())
    }

    fn endpoints(&self, id: &TentacleId) -> Result<Vec<RegistryEndpoint>, RegistryError> {
        Ok(self.resolve(id)?.endpoints)
    }

    fn capability_references(&self, id: &TentacleId) -> Result<Vec<String>, RegistryError> {
        Ok(self.resolve(id)?.capability_refs)
    }

    fn trust_signals(&self, id: &TentacleId) -> Result<Vec<TrustSignal>, RegistryError> {
        Ok(self.resolve(id)?.trust_signals)
    }

    fn verify_endpoint_association(
        &self,
        id: &TentacleId,
        tentacle_id: &TentacleId,
        inbox: &XmtpInboxRef,
    ) -> Result<bool, RegistryError> {
        if id != tentacle_id {
            return Ok(false);
        }
        let record = self.resolve(id)?;
        Ok(record.active
            && record.endpoints.iter().any(|endpoint| {
                endpoint.active
                    && &endpoint.tentacle_id == tentacle_id
                    && &endpoint.xmtp_inbox == inbox
            }))
    }

    fn is_active(&self, id: &TentacleId) -> Result<bool, RegistryError> {
        Ok(self.resolve(id)?.active)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Erc8004InterfaceRevision {
    RegistrationV1,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Erc8004InterfaceSupport {
    pub owner_of: bool,
    pub agent_uri: bool,
    pub get_agent_wallet: bool,
    pub get_metadata: bool,
    pub get_version: bool,
    pub is_authorized_or_owner: bool,
    pub register: bool,
    pub set_agent_uri: bool,
    pub set_metadata: bool,
    pub set_agent_wallet: bool,
    pub unset_agent_wallet: bool,
    pub registered_event: bool,
    pub uri_updated_event: bool,
    pub metadata_set_event: bool,
    pub transfer_event: bool,
}

impl Erc8004InterfaceSupport {
    fn is_complete(self) -> bool {
        self.owner_of
            && self.agent_uri
            && self.get_agent_wallet
            && self.get_metadata
            && self.get_version
            && self.is_authorized_or_owner
            && self.register
            && self.set_agent_uri
            && self.set_metadata
            && self.set_agent_wallet
            && self.unset_agent_wallet
            && self.registered_event
            && self.uri_updated_event
            && self.metadata_set_event
            && self.transfer_event
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Erc8004DeploymentObservation {
    pub chain_id: u64,
    pub identity_registry: EvmAddress,
    pub reputation_registry: EvmAddress,
    pub identity_proxy_implementation: Option<EvmAddress>,
    pub reputation_proxy_implementation: Option<EvmAddress>,
    pub identity_proxy_code_bytes: usize,
    pub reputation_proxy_code_bytes: usize,
    pub identity_implementation_code_bytes: usize,
    pub reputation_implementation_code_bytes: usize,
    pub interface_revision: Erc8004InterfaceRevision,
    pub identity_contract_version: String,
    pub reputation_contract_version: String,
    pub interface_support: Erc8004InterfaceSupport,
    pub reputation_identity_registry: EvmAddress,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Erc8004AgentSnapshot {
    pub agent_id: Erc8004AgentId,
    pub tentacle_id: TentacleId,
    pub owner: EvmAddress,
    pub agent_uri: String,
    pub agent_wallet: EvmAddress,
    /// Result of canonical `isAuthorizedOrOwner(expectedTentacleWallet, agentId)` at the observed
    /// block. This is independent from the verified receiving-wallet metadata.
    pub tentacle_wallet_is_authorized: bool,
    pub allegiance: Vec<u8>,
    pub protocol: Vec<u8>,
    pub metadata_tentacle_id: Option<Vec<u8>>,
    pub display_name: String,
    pub endpoints: Vec<RegistryEndpoint>,
    pub capability_refs: Vec<String>,
    pub trust_signals: Vec<TrustSignal>,
    pub observed_block: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Erc8004AgentWalletStatus {
    Verified,
    Missing,
    Unexpected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Erc8004ControlStatus {
    Owner,
    ApprovedOperator,
    NotAuthorized,
}

impl Erc8004AgentSnapshot {
    fn validate(&self) -> Result<(), RegistryError> {
        if self.owner.is_zero() {
            return Err(RegistryError::Invalid("ERC-8004 owner must be nonzero"));
        }
        if self.agent_uri.len() > MAX_AGENT_URI_LENGTH
            || self.agent_uri.chars().any(char::is_control)
        {
            return Err(RegistryError::Invalid("ERC-8004 agent URI is invalid"));
        }
        if self.allegiance.len() > 256
            || self.protocol.len() > 32
            || self
                .metadata_tentacle_id
                .as_ref()
                .is_some_and(|value| value.len() > 96)
        {
            return Err(RegistryError::Invalid("ERC-8004 metadata is oversized"));
        }
        if self
            .metadata_tentacle_id
            .as_deref()
            .is_some_and(|value| value != self.tentacle_id.as_str().as_bytes())
        {
            return Err(RegistryError::Invalid(
                "ERC-8004 Tentacle metadata does not match the selected Tentacle",
            ));
        }
        if self.observed_block == 0 {
            return Err(RegistryError::Invalid(
                "ERC-8004 observation block must be positive",
            ));
        }
        let record = RegisteredTentacle {
            id: self.tentacle_id.clone(),
            display_name: self.display_name.clone(),
            registry_ref: None,
            endpoints: self.endpoints.clone(),
            capability_refs: self.capability_refs.clone(),
            trust_signals: self.trust_signals.clone(),
            active: false,
            metadata_version: self.observed_block,
        };
        record.validate()
    }

    pub fn declares_tentacle_allegiance(&self) -> bool {
        self.allegiance.as_slice() == CTHUWU_ALLEGIANCE_VALUE
    }

    pub fn declares_supported_protocol(&self) -> bool {
        self.protocol.as_slice() == CTHUWU_PROTOCOL_VALUE
    }

    pub fn agent_wallet_status(&self, expected: EvmAddress) -> Erc8004AgentWalletStatus {
        if self.agent_wallet.is_zero() {
            Erc8004AgentWalletStatus::Missing
        } else if self.agent_wallet == expected {
            Erc8004AgentWalletStatus::Verified
        } else {
            Erc8004AgentWalletStatus::Unexpected
        }
    }
}

/// Backend seam for an RPC/subgraph implementation. Implementations must obtain these values from
/// current chain state; the adapter rechecks the canonical Base deployment and pinned interface on
/// construction and every read. This interface has no signing or arbitrary-call operation.
pub trait Erc8004ReadBackend: Send + Sync {
    fn deployment(&self) -> Result<Erc8004DeploymentObservation, RegistryError>;
    fn read_agent(
        &self,
        agent_id: &Erc8004AgentId,
        expected_tentacle_wallet: EvmAddress,
    ) -> Result<Erc8004AgentSnapshot, RegistryError>;
}

/// Read-only ERC-8004 adapter pinned to the canonical Base mainnet singleton deployment.
///
/// There is intentionally no production address or chain override. Tests inject a deterministic
/// backend and can make it report mismatches to exercise fail-closed behavior. Registration and
/// metadata writes belong to the separately isolated typed signer workflow, never this read trait.
#[derive(Clone)]
pub struct Erc8004Registry {
    backend: Arc<dyn Erc8004ReadBackend>,
    bindings: BTreeMap<TentacleId, Erc8004Binding>,
}

impl fmt::Debug for Erc8004Registry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Erc8004Registry")
            .field("chain_id", &BASE_MAINNET_CHAIN_ID)
            .field("identity_registry", &BASE_IDENTITY_REGISTRY_ADDRESS)
            .field("reputation_registry", &BASE_REPUTATION_REGISTRY_ADDRESS)
            .field("bindings", &self.bindings)
            .finish_non_exhaustive()
    }
}

impl Erc8004Registry {
    pub fn new(backend: Arc<dyn Erc8004ReadBackend>) -> Result<Self, RegistryError> {
        verify_canonical_deployment(&backend.deployment()?)?;
        Ok(Self {
            backend,
            bindings: BTreeMap::new(),
        })
    }

    pub fn with_bindings(
        backend: Arc<dyn Erc8004ReadBackend>,
        bindings: impl IntoIterator<Item = Erc8004Binding>,
    ) -> Result<Self, RegistryError> {
        let mut registry = Self::new(backend)?;
        for binding in bindings {
            registry.bind(binding)?;
        }
        Ok(registry)
    }

    pub fn bind(&mut self, binding: Erc8004Binding) -> Result<(), RegistryError> {
        binding.validate()?;
        if self.bindings.contains_key(&binding.tentacle_id) {
            return Err(RegistryError::Invalid(
                "Tentacle already has a persisted ERC-8004 binding",
            ));
        }
        if self
            .bindings
            .values()
            .any(|other| other.agent_id == binding.agent_id)
        {
            return Err(RegistryError::Invalid(
                "ERC-8004 agent ID is already bound to another Tentacle",
            ));
        }
        self.bindings.insert(binding.tentacle_id.clone(), binding);
        Ok(())
    }

    pub fn agent_id(&self, tentacle_id: &TentacleId) -> Option<&Erc8004AgentId> {
        self.bindings
            .get(tentacle_id)
            .map(|binding| &binding.agent_id)
    }

    pub fn binding(&self, tentacle_id: &TentacleId) -> Option<&Erc8004Binding> {
        self.bindings.get(tentacle_id)
    }

    pub fn read_agent(
        &self,
        tentacle_id: &TentacleId,
    ) -> Result<Erc8004AgentSnapshot, RegistryError> {
        verify_canonical_deployment(&self.backend.deployment()?)?;
        let binding = self
            .bindings
            .get(tentacle_id)
            .ok_or(RegistryError::NotFound)?;
        let snapshot = self
            .backend
            .read_agent(&binding.agent_id, binding.tentacle_wallet)?;
        snapshot.validate()?;
        if snapshot.agent_id != binding.agent_id || &snapshot.tentacle_id != tentacle_id {
            return Err(RegistryError::Invalid(
                "ERC-8004 backend returned a different bound identity",
            ));
        }
        Ok(snapshot)
    }

    pub fn control_status(
        &self,
        tentacle_id: &TentacleId,
    ) -> Result<Erc8004ControlStatus, RegistryError> {
        let binding = self
            .bindings
            .get(tentacle_id)
            .ok_or(RegistryError::NotFound)?;
        let snapshot = self.read_agent(tentacle_id)?;
        Ok(if snapshot.owner == binding.tentacle_wallet {
            Erc8004ControlStatus::Owner
        } else if snapshot.tentacle_wallet_is_authorized {
            Erc8004ControlStatus::ApprovedOperator
        } else {
            Erc8004ControlStatus::NotAuthorized
        })
    }

    fn registered_tentacle(
        &self,
        tentacle_id: &TentacleId,
    ) -> Result<RegisteredTentacle, RegistryError> {
        let snapshot = self.read_agent(tentacle_id)?;
        let binding = self
            .bindings
            .get(tentacle_id)
            .ok_or(RegistryError::NotFound)?;
        let identity_registry: EvmAddress = BASE_IDENTITY_REGISTRY_ADDRESS.parse()?;
        let registry_ref = RegistryRef::new(
            "erc-8004",
            format!(
                "eip155:{BASE_MAINNET_CHAIN_ID}:{identity_registry}:{}",
                snapshot.agent_id
            ),
        )
        .map_err(|_| RegistryError::Invalid("ERC-8004 registry reference is invalid"))?;
        let active = snapshot.declares_tentacle_allegiance()
            && snapshot.agent_wallet_status(binding.tentacle_wallet)
                == Erc8004AgentWalletStatus::Verified;
        let record = RegisteredTentacle {
            id: snapshot.tentacle_id,
            display_name: snapshot.display_name,
            registry_ref: Some(registry_ref),
            endpoints: snapshot.endpoints,
            capability_refs: snapshot.capability_refs,
            trust_signals: snapshot.trust_signals,
            active,
            metadata_version: snapshot.observed_block,
        };
        record.validate()?;
        Ok(record)
    }
}

impl AgentRegistry for Erc8004Registry {
    fn resolve(&self, id: &TentacleId) -> Result<RegisteredTentacle, RegistryError> {
        self.registered_tentacle(id)
    }

    fn register_or_update(&mut self, _agent: RegisteredTentacle) -> Result<(), RegistryError> {
        Err(RegistryError::ReadOnly)
    }

    fn endpoints(&self, id: &TentacleId) -> Result<Vec<RegistryEndpoint>, RegistryError> {
        Ok(self.resolve(id)?.endpoints)
    }

    fn capability_references(&self, id: &TentacleId) -> Result<Vec<String>, RegistryError> {
        Ok(self.resolve(id)?.capability_refs)
    }

    fn trust_signals(&self, id: &TentacleId) -> Result<Vec<TrustSignal>, RegistryError> {
        Ok(self.resolve(id)?.trust_signals)
    }

    fn verify_endpoint_association(
        &self,
        id: &TentacleId,
        tentacle_id: &TentacleId,
        inbox: &XmtpInboxRef,
    ) -> Result<bool, RegistryError> {
        if id != tentacle_id {
            return Ok(false);
        }
        let record = self.resolve(id)?;
        Ok(record.active
            && record.endpoints.iter().any(|endpoint| {
                endpoint.active
                    && &endpoint.tentacle_id == tentacle_id
                    && &endpoint.xmtp_inbox == inbox
            }))
    }

    fn is_active(&self, id: &TentacleId) -> Result<bool, RegistryError> {
        Ok(self.resolve(id)?.active)
    }
}

fn verify_canonical_deployment(
    observed: &Erc8004DeploymentObservation,
) -> Result<(), RegistryError> {
    let identity: EvmAddress = BASE_IDENTITY_REGISTRY_ADDRESS.parse()?;
    let reputation: EvmAddress = BASE_REPUTATION_REGISTRY_ADDRESS.parse()?;
    let identity_implementation: EvmAddress = BASE_IDENTITY_IMPLEMENTATION_ADDRESS.parse()?;
    let reputation_implementation: EvmAddress = BASE_REPUTATION_IMPLEMENTATION_ADDRESS.parse()?;
    if observed.chain_id != BASE_MAINNET_CHAIN_ID {
        return Err(RegistryError::DeploymentMismatch(
            "RPC chain is not Base mainnet 8453",
        ));
    }
    if observed.identity_registry != identity || observed.reputation_registry != reputation {
        return Err(RegistryError::DeploymentMismatch(
            "registry address is not the canonical Base deployment",
        ));
    }
    if observed.identity_proxy_code_bytes == 0 || observed.reputation_proxy_code_bytes == 0 {
        return Err(RegistryError::DeploymentMismatch(
            "canonical registry proxy has no deployed code",
        ));
    }
    if observed.identity_proxy_implementation != Some(identity_implementation)
        || observed.reputation_proxy_implementation != Some(reputation_implementation)
        || observed.identity_implementation_code_bytes == 0
        || observed.reputation_implementation_code_bytes == 0
    {
        return Err(RegistryError::DeploymentMismatch(
            "canonical registry proxy implementation does not match the pinned deployment",
        ));
    }
    if observed.interface_revision != Erc8004InterfaceRevision::RegistrationV1
        || observed.identity_contract_version != ERC8004_IDENTITY_CONTRACT_VERSION
        || observed.reputation_contract_version != ERC8004_REPUTATION_CONTRACT_VERSION
        || !observed.interface_support.is_complete()
    {
        return Err(RegistryError::DeploymentMismatch(
            "registry does not expose the pinned ERC-8004 registration-v1 interface",
        ));
    }
    if observed.reputation_identity_registry != identity {
        return Err(RegistryError::DeploymentMismatch(
            "Reputation Registry is bound to a different Identity Registry",
        ));
    }
    Ok(())
}

fn validate_label(value: &str, name: &'static str, maximum: usize) -> Result<(), RegistryError> {
    if value.trim().is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(RegistryError::Invalid(name));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn tentacle_id(name: &str) -> TentacleId {
        TentacleId::new(format!("tentacle_{name}")).unwrap()
    }

    fn record(version: u64) -> RegisteredTentacle {
        let id = tentacle_id("archive");
        RegisteredTentacle {
            id: id.clone(),
            display_name: "Archivist".to_owned(),
            registry_ref: Some(RegistryRef::new("erc-8004", "agent:42").unwrap()),
            endpoints: vec![RegistryEndpoint {
                tentacle_id: id,
                xmtp_inbox: XmtpInboxRef::new("012345abcdef").unwrap(),
                active: true,
            }],
            capability_refs: vec!["ipfs:capabilities-v1".to_owned()],
            trust_signals: vec![TrustSignal {
                provenance: "local-allowlist".to_owned(),
                kind: "operator-attestation".to_owned(),
                value: 50,
                observed_at: 100,
                evidence_ref: None,
            }],
            active: true,
            metadata_version: version,
        }
    }

    #[test]
    fn local_registry_resolves_tentacles_and_verifies_endpoints() {
        let mut registry = LocalRegistry::default();
        let agent = record(1);
        registry.register_or_update(agent.clone()).unwrap();
        assert_eq!(registry.resolve(&agent.id).unwrap(), agent);
        assert!(
            registry
                .verify_endpoint_association(&agent.id, &agent.id, &agent.endpoints[0].xmtp_inbox,)
                .unwrap()
        );
        assert!(registry.is_active(&agent.id).unwrap());
    }

    #[test]
    fn local_registry_rejects_stale_metadata_and_cross_tentacle_endpoints() {
        let mut registry = LocalRegistry::default();
        registry.register_or_update(record(2)).unwrap();
        assert_eq!(
            registry.register_or_update(record(1)).unwrap_err(),
            RegistryError::StaleUpdate
        );

        let mut wrong = record(3);
        wrong.endpoints[0].tentacle_id = tentacle_id("intruder");
        assert!(wrong.validate().is_err());
    }

    #[test]
    fn versioned_registry_round_trips_and_revalidates_keys() {
        let mut registry = LocalRegistry::default();
        registry.register_or_update(record(1)).unwrap();
        let encoded = serde_json::to_value(&registry).unwrap();
        assert_eq!(encoded["schemaVersion"], LOCAL_REGISTRY_SCHEMA_VERSION);
        let restored: LocalRegistry = serde_json::from_value(encoded.clone()).unwrap();
        restored.validate_loaded_state().unwrap();

        let mut corrupt = encoded;
        let records = corrupt["tentacles"].as_object_mut().unwrap();
        let agent = records.remove("tentacle_archive").unwrap();
        records.insert("tentacle_intruder".to_owned(), agent);
        assert!(serde_json::from_value::<LocalRegistry>(corrupt).is_err());
    }

    #[test]
    fn legacy_single_tentacle_snapshots_migrate_with_provenance() {
        let legacy = serde_json::json!({
            "agents": {
                "cthulhu_archivist": {
                    "id": "cthulhu_archivist",
                    "displayName": "Archivist",
                    "registryRef": {"registry": "erc-8004", "reference": "agent:42"},
                    "endpoints": [{
                        "tentacleId": "tentacle_archive",
                        "xmtpInbox": "012345abcdef",
                        "active": true
                    }],
                    "capabilityRefs": ["ipfs:capabilities-v1"],
                    "trustSignals": [],
                    "active": true,
                    "metadataVersion": 1
                }
            }
        });
        let migrated: LocalRegistry = serde_json::from_value(legacy).unwrap();
        assert!(migrated.resolve(&tentacle_id("archive")).is_ok());
        let provenance = migrated.migration().unwrap();
        assert_eq!(provenance.source_schema_version, 1);
        assert_eq!(
            provenance.legacy_cthulhu_ids,
            vec![CthulhuId::new("cthulhu_archivist").unwrap()]
        );
        let rewritten = serde_json::to_value(migrated).unwrap();
        assert_eq!(rewritten["schemaVersion"], 2);
        assert!(rewritten.get("agents").is_none());
    }

    #[test]
    fn ambiguous_legacy_multi_tentacle_identity_is_not_silently_reinterpreted() {
        let legacy = serde_json::json!({
            "agents": {
                "cthulhu_archivist": {
                    "id": "cthulhu_archivist",
                    "displayName": "Archivist",
                    "registryRef": null,
                    "endpoints": [
                        {"tentacleId": "tentacle_one", "xmtpInbox": "000000000001", "active": true},
                        {"tentacleId": "tentacle_two", "xmtpInbox": "000000000002", "active": true}
                    ],
                    "capabilityRefs": [],
                    "trustSignals": [],
                    "active": true,
                    "metadataVersion": 1
                }
            }
        });
        assert!(serde_json::from_value::<LocalRegistry>(legacy).is_err());
    }

    fn complete_interface() -> Erc8004InterfaceSupport {
        Erc8004InterfaceSupport {
            owner_of: true,
            agent_uri: true,
            get_agent_wallet: true,
            get_metadata: true,
            get_version: true,
            is_authorized_or_owner: true,
            register: true,
            set_agent_uri: true,
            set_metadata: true,
            set_agent_wallet: true,
            unset_agent_wallet: true,
            registered_event: true,
            uri_updated_event: true,
            metadata_set_event: true,
            transfer_event: true,
        }
    }

    fn canonical_deployment() -> Erc8004DeploymentObservation {
        Erc8004DeploymentObservation {
            chain_id: BASE_MAINNET_CHAIN_ID,
            identity_registry: BASE_IDENTITY_REGISTRY_ADDRESS.parse().unwrap(),
            reputation_registry: BASE_REPUTATION_REGISTRY_ADDRESS.parse().unwrap(),
            identity_proxy_implementation: Some(
                BASE_IDENTITY_IMPLEMENTATION_ADDRESS.parse().unwrap(),
            ),
            reputation_proxy_implementation: Some(
                BASE_REPUTATION_IMPLEMENTATION_ADDRESS.parse().unwrap(),
            ),
            identity_proxy_code_bytes: 128,
            reputation_proxy_code_bytes: 128,
            identity_implementation_code_bytes: 1_024,
            reputation_implementation_code_bytes: 1_024,
            interface_revision: Erc8004InterfaceRevision::RegistrationV1,
            identity_contract_version: ERC8004_IDENTITY_CONTRACT_VERSION.to_owned(),
            reputation_contract_version: ERC8004_REPUTATION_CONTRACT_VERSION.to_owned(),
            interface_support: complete_interface(),
            reputation_identity_registry: BASE_IDENTITY_REGISTRY_ADDRESS.parse().unwrap(),
        }
    }

    fn active_snapshot() -> Erc8004AgentSnapshot {
        let id = tentacle_id("archive");
        Erc8004AgentSnapshot {
            agent_id: Erc8004AgentId::new("42").unwrap(),
            tentacle_id: id.clone(),
            owner: "0x3333333333333333333333333333333333333333"
                .parse()
                .unwrap(),
            agent_uri: "data:application/json;base64,e30=".to_owned(),
            agent_wallet: "0x3333333333333333333333333333333333333333"
                .parse()
                .unwrap(),
            tentacle_wallet_is_authorized: true,
            allegiance: CTHUWU_ALLEGIANCE_VALUE.to_vec(),
            protocol: CTHUWU_PROTOCOL_VALUE.to_vec(),
            metadata_tentacle_id: Some(id.as_str().as_bytes().to_vec()),
            display_name: "Archivist".to_owned(),
            endpoints: vec![RegistryEndpoint {
                tentacle_id: id,
                xmtp_inbox: XmtpInboxRef::new("012345abcdef").unwrap(),
                active: true,
            }],
            capability_refs: vec!["data:application/json;base64,e30=".to_owned()],
            trust_signals: vec![],
            observed_block: 42,
        }
    }

    struct MockBackend {
        deployment: Mutex<Erc8004DeploymentObservation>,
        agents: BTreeMap<Erc8004AgentId, Erc8004AgentSnapshot>,
    }

    impl Erc8004ReadBackend for MockBackend {
        fn deployment(&self) -> Result<Erc8004DeploymentObservation, RegistryError> {
            Ok(self.deployment.lock().unwrap().clone())
        }

        fn read_agent(
            &self,
            agent_id: &Erc8004AgentId,
            expected_tentacle_wallet: EvmAddress,
        ) -> Result<Erc8004AgentSnapshot, RegistryError> {
            let snapshot = self
                .agents
                .get(agent_id)
                .cloned()
                .ok_or(RegistryError::NotFound)?;
            if expected_tentacle_wallet.is_zero() {
                return Err(RegistryError::Invalid(
                    "backend received a zero expected Tentacle wallet",
                ));
            }
            Ok(snapshot)
        }
    }

    fn adapter_with(
        deployment: Erc8004DeploymentObservation,
        snapshot: Erc8004AgentSnapshot,
    ) -> Result<Erc8004Registry, RegistryError> {
        let agent_id = snapshot.agent_id.clone();
        let tentacle_id = snapshot.tentacle_id.clone();
        let tentacle_wallet = active_snapshot().agent_wallet;
        let backend = Arc::new(MockBackend {
            deployment: Mutex::new(deployment),
            agents: BTreeMap::from([(agent_id.clone(), snapshot)]),
        });
        let mut adapter = Erc8004Registry::new(backend)?;
        adapter.bind(Erc8004Binding {
            tentacle_id,
            agent_id,
            tentacle_wallet,
        })?;
        Ok(adapter)
    }

    #[test]
    fn erc8004_adapter_accepts_only_exact_current_opt_in_and_wallet() {
        let snapshot = active_snapshot();
        let id = snapshot.tentacle_id.clone();
        let adapter = adapter_with(canonical_deployment(), snapshot).unwrap();
        let record = adapter.resolve(&id).unwrap();
        assert!(record.active);
        assert!(adapter.is_active(&id).unwrap());
        assert_eq!(
            adapter.control_status(&id).unwrap(),
            Erc8004ControlStatus::Owner
        );
        assert!(
            adapter
                .verify_endpoint_association(&id, &id, &record.endpoints[0].xmtp_inbox)
                .unwrap()
        );
        assert_eq!(
            record.registry_ref.unwrap().reference,
            "eip155:8453:0x8004a169fb4a3325136eb29fa0ceb6d2e539a432:42"
        );
    }

    #[test]
    fn nonexact_allegiance_and_unverified_wallet_are_inactive_not_membership() {
        let mutations: [fn(&mut Erc8004AgentSnapshot); 5] = [
            |snapshot: &mut Erc8004AgentSnapshot| snapshot.allegiance = b"UWU-TENTACLE-V1".to_vec(),
            |snapshot: &mut Erc8004AgentSnapshot| {
                snapshot.allegiance = b"uwu-tentacle-v1 ".to_vec()
            },
            |snapshot: &mut Erc8004AgentSnapshot| snapshot.allegiance.clear(),
            |snapshot: &mut Erc8004AgentSnapshot| snapshot.agent_wallet = EvmAddress::ZERO,
            |snapshot: &mut Erc8004AgentSnapshot| {
                snapshot.agent_wallet = "0x6666666666666666666666666666666666666666"
                    .parse()
                    .unwrap()
            },
        ];
        for mutate in mutations {
            let mut snapshot = active_snapshot();
            mutate(&mut snapshot);
            let id = snapshot.tentacle_id.clone();
            let adapter = adapter_with(canonical_deployment(), snapshot).unwrap();
            assert!(!adapter.resolve(&id).unwrap().active);
        }
    }

    #[test]
    fn unsupported_protocol_is_visible_without_overriding_exact_membership() {
        let mut snapshot = active_snapshot();
        snapshot.protocol = b"2".to_vec();
        assert!(!snapshot.declares_supported_protocol());
        let id = snapshot.tentacle_id.clone();
        let adapter = adapter_with(canonical_deployment(), snapshot).unwrap();
        assert!(adapter.resolve(&id).unwrap().active);
    }

    #[test]
    fn operator_control_is_verified_separately_from_allegiance_membership() {
        let mut approved = active_snapshot();
        approved.owner = "0x7777777777777777777777777777777777777777"
            .parse()
            .unwrap();
        let id = approved.tentacle_id.clone();
        let adapter = adapter_with(canonical_deployment(), approved).unwrap();
        assert!(adapter.resolve(&id).unwrap().active);
        assert_eq!(
            adapter.control_status(&id).unwrap(),
            Erc8004ControlStatus::ApprovedOperator
        );

        let mut unauthorized = active_snapshot();
        unauthorized.owner = "0x7777777777777777777777777777777777777777"
            .parse()
            .unwrap();
        unauthorized.tentacle_wallet_is_authorized = false;
        let id = unauthorized.tentacle_id.clone();
        let adapter = adapter_with(canonical_deployment(), unauthorized).unwrap();
        assert!(adapter.resolve(&id).unwrap().active);
        assert_eq!(
            adapter.control_status(&id).unwrap(),
            Erc8004ControlStatus::NotAuthorized
        );
    }

    #[test]
    fn erc8004_adapter_rejects_wrong_chain_address_code_proxy_and_interface() {
        let snapshot = active_snapshot();
        let mut cases = Vec::new();
        let mut wrong_chain = canonical_deployment();
        wrong_chain.chain_id = 84_532;
        cases.push(wrong_chain);
        let mut wrong_address = canonical_deployment();
        wrong_address.identity_registry = "0x4444444444444444444444444444444444444444"
            .parse()
            .unwrap();
        cases.push(wrong_address);
        let mut no_code = canonical_deployment();
        no_code.identity_proxy_code_bytes = 0;
        cases.push(no_code);
        let mut no_implementation = canonical_deployment();
        no_implementation.identity_proxy_implementation = None;
        cases.push(no_implementation);
        let mut wrong_interface = canonical_deployment();
        wrong_interface.interface_support.get_metadata = false;
        cases.push(wrong_interface);
        let mut wrong_version = canonical_deployment();
        wrong_version.identity_contract_version = "1.0.0".to_owned();
        cases.push(wrong_version);
        let mut wrong_reputation_version = canonical_deployment();
        wrong_reputation_version.reputation_contract_version = "1.0.0".to_owned();
        cases.push(wrong_reputation_version);
        let mut wrong_reputation_binding = canonical_deployment();
        wrong_reputation_binding.reputation_identity_registry =
            "0x5555555555555555555555555555555555555555"
                .parse()
                .unwrap();
        cases.push(wrong_reputation_binding);

        for deployment in cases {
            assert!(adapter_with(deployment, snapshot.clone()).is_err());
        }
    }

    #[test]
    fn erc8004_adapter_rechecks_deployment_and_rejects_generic_writes() {
        let snapshot = active_snapshot();
        let agent_id = snapshot.agent_id.clone();
        let tentacle_id = snapshot.tentacle_id.clone();
        let backend = Arc::new(MockBackend {
            deployment: Mutex::new(canonical_deployment()),
            agents: BTreeMap::from([(agent_id.clone(), snapshot)]),
        });
        let mut adapter = Erc8004Registry::new(backend.clone()).unwrap();
        adapter
            .bind(Erc8004Binding {
                tentacle_id: tentacle_id.clone(),
                agent_id,
                tentacle_wallet: active_snapshot().agent_wallet,
            })
            .unwrap();
        assert_eq!(
            adapter.register_or_update(record(1)).unwrap_err(),
            RegistryError::ReadOnly
        );
        backend.deployment.lock().unwrap().chain_id = 84_532;
        assert!(matches!(
            adapter.resolve(&tentacle_id),
            Err(RegistryError::DeploymentMismatch(_))
        ));
    }

    #[test]
    fn erc8004_agent_id_and_optional_tentacle_metadata_fail_closed() {
        assert!(Erc8004AgentId::new("").is_err());
        assert!(Erc8004AgentId::new("01").is_err());
        assert!(Erc8004AgentId::new(U256_MAX_DECIMAL).is_ok());
        assert!(Erc8004AgentId::new("9".repeat(78)).is_err());
        assert!(Erc8004AgentId::new("1".repeat(79)).is_err());

        let mut snapshot = active_snapshot();
        snapshot.metadata_tentacle_id = Some(b"tentacle_someone_else".to_vec());
        let id = snapshot.tentacle_id.clone();
        let adapter = adapter_with(canonical_deployment(), snapshot).unwrap();
        assert!(adapter.resolve(&id).is_err());
    }

    #[test]
    fn persisted_binding_contains_public_identity_only_and_rejects_zero_wallet() {
        let binding = Erc8004Binding {
            tentacle_id: tentacle_id("archive"),
            agent_id: Erc8004AgentId::new("42").unwrap(),
            tentacle_wallet: active_snapshot().agent_wallet,
        };
        let encoded = serde_json::to_string(&binding).unwrap();
        assert!(!encoded.contains("private"));
        assert_eq!(
            serde_json::from_str::<Erc8004Binding>(&encoded).unwrap(),
            binding
        );

        let mut invalid = binding;
        invalid.tentacle_wallet = EvmAddress::ZERO;
        assert!(invalid.validate().is_err());
    }
}
