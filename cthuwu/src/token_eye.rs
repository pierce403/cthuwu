//! Local ERC-20 observation for mandatory economic admission and lifecycle scoring.
//!
//! The types in this module deliberately do not depend on an Ethereum SDK.  A
//! Tentacle needs only an RPC endpoint, a token contract address, and the
//! addresses it has learned through its existing peer-to-peer relationships.
//! No signing material is accepted or retained here; transaction execution is handled by the
//! explicit economic action state machine.

use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    error::Error,
    fmt,
    net::IpAddr,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::Mutex;

const BALANCE_OF_SELECTOR: &str = "70a08231";
const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RPC_RESPONSE_BYTES: usize = 64 * 1024;
const BASIS_POINTS_DENOMINATOR: u128 = 10_000;
const DEFAULT_ACOLYTE_MINIMUM_RAW: u64 = 1_000_000_000_000_000_000;
const MAX_FAILURE_BACKOFF: Duration = Duration::from_secs(30);
const MIN_FAILURE_BACKOFF: Duration = Duration::from_secs(1);

/// A strict 20-byte Ethereum address.
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Address([u8; 20]);

impl Address {
    pub const ZERO: Self = Self([0; 20]);

    pub const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }
}

impl FromStr for Address {
    type Err = TokenEyeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if !value.starts_with("0x") {
            return Err(TokenEyeError::InvalidAddress(
                "address must start with lowercase 0x",
            ));
        }
        let encoded = &value[2..];
        if encoded.len() != 40 {
            return Err(TokenEyeError::InvalidAddress(
                "address must contain exactly 40 hexadecimal digits",
            ));
        }

        let mut bytes = [0_u8; 20];
        decode_hex_exact(encoded.as_bytes(), &mut bytes).map_err(TokenEyeError::InvalidAddress)?;
        Ok(Self(bytes))
    }
}

impl fmt::Display for Address {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("0x")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

/// An unsigned 256-bit integer stored in big-endian byte order.
///
/// Ordering is numeric because fixed-width big-endian byte strings sort in the
/// same order as the represented integers.
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct U256([u8; 32]);

impl U256 {
    pub const ZERO: Self = Self([0; 32]);

    pub const fn from_be_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn to_be_bytes(self) -> [u8; 32] {
        self.0
    }

    pub fn from_u64(value: u64) -> Self {
        let mut bytes = [0_u8; 32];
        bytes[24..].copy_from_slice(&value.to_be_bytes());
        Self(bytes)
    }

    pub fn is_zero(self) -> bool {
        self == Self::ZERO
    }

    /// Parses an Ethereum JSON-RPC quantity.
    ///
    /// Quantities must use a lowercase `0x` prefix, must not have leading zero
    /// digits, and may contain at most 64 hexadecimal digits. Hexadecimal digits
    /// themselves may be lowercase or uppercase, as required by JSON-RPC.
    pub fn from_quantity(value: &str) -> Result<Self, TokenEyeError> {
        if !value.starts_with("0x") {
            return Err(TokenEyeError::InvalidQuantity(
                "quantity must start with lowercase 0x",
            ));
        }
        let encoded = &value[2..];
        if encoded.is_empty() {
            return Err(TokenEyeError::InvalidQuantity(
                "quantity must contain at least one hexadecimal digit",
            ));
        }
        if encoded.len() > 64 {
            return Err(TokenEyeError::InvalidQuantity("quantity exceeds 256 bits"));
        }
        if encoded.len() > 1 && encoded.starts_with('0') {
            return Err(TokenEyeError::InvalidQuantity(
                "quantity must not contain leading zero digits",
            ));
        }

        let mut bytes = [0_u8; 32];
        decode_hex_right_aligned(encoded.as_bytes(), &mut bytes)
            .map_err(TokenEyeError::InvalidQuantity)?;
        Ok(Self(bytes))
    }

    /// Parses the exact 32-byte ABI word returned by `eth_call` for
    /// `balanceOf(address)`.
    pub fn from_abi_word(value: &str) -> Result<Self, TokenEyeError> {
        if !value.starts_with("0x") {
            return Err(TokenEyeError::InvalidResponse(
                "balance result must start with lowercase 0x",
            ));
        }
        let encoded = &value[2..];
        if encoded.len() != 64 {
            return Err(TokenEyeError::InvalidResponse(
                "balance result must be one 32-byte ABI word",
            ));
        }

        let mut bytes = [0_u8; 32];
        decode_hex_exact(encoded.as_bytes(), &mut bytes).map_err(TokenEyeError::InvalidResponse)?;
        Ok(Self(bytes))
    }

    pub fn to_quantity(self) -> String {
        let first_nonzero = self.0.iter().position(|byte| *byte != 0);
        let Some(first_nonzero) = first_nonzero else {
            return "0x0".to_owned();
        };

        let mut encoded = String::with_capacity(66);
        encoded.push_str("0x");
        let first = self.0[first_nonzero];
        if first < 16 {
            push_hex_nibble(&mut encoded, first);
        } else {
            push_hex_byte(&mut encoded, first);
        }
        for byte in &self.0[first_nonzero + 1..] {
            push_hex_byte(&mut encoded, *byte);
        }
        encoded
    }

    pub fn to_abi_word(self) -> String {
        let mut encoded = String::with_capacity(66);
        encoded.push_str("0x");
        for byte in self.0 {
            push_hex_byte(&mut encoded, byte);
        }
        encoded
    }

    /// Returns whole-token units after removing `decimals`, saturating at `u64::MAX`.
    pub fn whole_units(self, decimals: u8) -> u64 {
        let mut value = self;
        for _ in 0..decimals {
            value = value.div_small(10);
        }
        value.saturating_to_u64()
    }

    pub fn saturating_to_u64(self) -> u64 {
        if self.0[..24].iter().any(|byte| *byte != 0) {
            return u64::MAX;
        }
        u64::from_be_bytes(self.0[24..].try_into().expect("slice has eight bytes"))
    }

    pub fn checked_to_u64(self) -> Option<u64> {
        (!self.0[..24].iter().any(|byte| *byte != 0))
            .then(|| u64::from_be_bytes(self.0[24..].try_into().expect("slice has eight bytes")))
    }

    pub fn power_of_ten(exponent: u8) -> Option<Self> {
        let mut value = Self::from_u64(1);
        for _ in 0..exponent {
            value = value.checked_mul_small(10)?;
        }
        Some(value)
    }

    /// Multiplies by a machine-sized unsigned value without wrapping the ERC-20 `uint256` range.
    pub fn checked_mul_u64(self, multiplier: u64) -> Option<Self> {
        let mut product = [0_u8; 32];
        let mut carry = 0_u128;
        for index in (0..32).rev() {
            let value = u128::from(self.0[index]) * u128::from(multiplier) + carry;
            product[index] = value as u8;
            carry = value >> 8;
        }
        (carry == 0).then_some(Self(product))
    }

    fn div_small(self, divisor: u16) -> Self {
        debug_assert!(divisor != 0);
        let mut quotient = [0_u8; 32];
        let mut remainder = 0_u16;
        for (index, byte) in self.0.into_iter().enumerate() {
            let dividend = (remainder << 8) | u16::from(byte);
            quotient[index] =
                u8::try_from(dividend / divisor).expect("a byte-wise quotient always fits in u8");
            remainder = dividend % divisor;
        }
        Self(quotient)
    }

    fn checked_mul_small(self, multiplier: u16) -> Option<Self> {
        let mut product = [0_u8; 32];
        let mut carry = 0_u16;
        for index in (0..32).rev() {
            let value = u16::from(self.0[index]) * multiplier + carry;
            product[index] = value as u8;
            carry = value >> 8;
        }
        (carry == 0).then_some(Self(product))
    }
}

impl From<u64> for U256 {
    fn from(value: u64) -> Self {
        Self::from_u64(value)
    }
}

impl FromStr for U256 {
    type Err = TokenEyeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_quantity(value)
    }
}

impl fmt::Display for U256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_quantity())
    }
}

impl fmt::Debug for U256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Serialize for U256 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_quantity())
    }
}

impl<'de> Deserialize<'de> for U256 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_quantity(&value).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TokenEyeError {
    InvalidAddress(&'static str),
    InvalidQuantity(&'static str),
    InvalidEndpoint(&'static str),
    InvalidTierPolicy(&'static str),
    InvalidResponse(&'static str),
    Http(&'static str),
    HttpStatus(u16),
    Rpc { code: i64, message: String },
    Transport(String),
    EconomicDataUnavailable(&'static str),
}

impl fmt::Display for TokenEyeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAddress(reason) => write!(formatter, "invalid Ethereum address: {reason}"),
            Self::InvalidQuantity(reason) => {
                write!(formatter, "invalid Ethereum quantity: {reason}")
            }
            Self::InvalidEndpoint(reason) => write!(formatter, "invalid RPC endpoint: {reason}"),
            Self::InvalidTierPolicy(reason) => {
                write!(formatter, "invalid reputation tier policy: {reason}")
            }
            Self::InvalidResponse(reason) => write!(formatter, "invalid RPC response: {reason}"),
            Self::Http(reason) => write!(formatter, "RPC HTTP request failed: {reason}"),
            Self::HttpStatus(status) => {
                write!(formatter, "RPC HTTP request returned status {status}")
            }
            Self::Rpc { code, message } => {
                write!(formatter, "RPC returned error {code}: {message}")
            }
            Self::Transport(message) => write!(formatter, "token transport failed: {message}"),
            Self::EconomicDataUnavailable(reason) => {
                write!(formatter, "economic operation blocked: {reason}")
            }
        }
    }
}

impl Error for TokenEyeError {}

/// ABI-encodes `balanceOf(holder)` as Ethereum call data.
pub fn encode_balance_of_call(holder: Address) -> String {
    let mut encoded = String::with_capacity(74);
    encoded.push_str("0x");
    encoded.push_str(BALANCE_OF_SELECTOR);
    // ABI addresses are right-aligned in a 32-byte word.
    encoded.push_str("000000000000000000000000");
    for byte in holder.0 {
        push_hex_byte(&mut encoded, byte);
    }
    encoded
}

/// Constructs the exact JSON-RPC request used by the live transport.
pub fn balance_of_rpc_request(id: u64, contract: Address, holder: Address) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "eth_call",
        "params": [
            {
                "to": contract.to_string(),
                "data": encode_balance_of_call(holder),
            },
            "latest"
        ]
    })
}

/// Constructs the JSON-RPC request used to bind an endpoint to the configured chain.
pub fn chain_id_rpc_request(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "eth_chainId",
        "params": []
    })
}

#[derive(Deserialize)]
struct RpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    error: Option<RpcErrorBody>,
}

#[derive(Deserialize)]
struct RpcErrorBody {
    code: i64,
    message: String,
}

/// Validates and decodes a JSON-RPC `balanceOf` response.
pub fn parse_balance_of_rpc_response(
    response: Value,
    expected_id: u64,
) -> Result<U256, TokenEyeError> {
    let response: RpcResponse = serde_json::from_value(response)
        .map_err(|_| TokenEyeError::InvalidResponse("response is not valid JSON-RPC"))?;
    if response.jsonrpc != "2.0" {
        return Err(TokenEyeError::InvalidResponse(
            "jsonrpc version must be 2.0",
        ));
    }
    if response.id.as_u64() != Some(expected_id) {
        return Err(TokenEyeError::InvalidResponse(
            "response id does not match request id",
        ));
    }
    if let Some(error) = response.error {
        return Err(TokenEyeError::Rpc {
            code: error.code,
            message: limit_chars(&error.message, 512),
        });
    }
    let result = response.result.ok_or(TokenEyeError::InvalidResponse(
        "response has neither a result nor an error",
    ))?;
    U256::from_abi_word(&result)
}

pub fn parse_chain_id_rpc_response(
    response: Value,
    expected_id: u64,
) -> Result<u64, TokenEyeError> {
    let response: RpcResponse = serde_json::from_value(response)
        .map_err(|_| TokenEyeError::InvalidResponse("response is not valid JSON-RPC"))?;
    if response.jsonrpc != "2.0" {
        return Err(TokenEyeError::InvalidResponse(
            "jsonrpc version must be 2.0",
        ));
    }
    if response.id.as_u64() != Some(expected_id) {
        return Err(TokenEyeError::InvalidResponse(
            "response id does not match request id",
        ));
    }
    if let Some(error) = response.error {
        return Err(TokenEyeError::Rpc {
            code: error.code,
            message: limit_chars(&error.message, 512),
        });
    }
    let result = response.result.ok_or(TokenEyeError::InvalidResponse(
        "response has neither a result nor an error",
    ))?;
    U256::from_quantity(&result)?
        .checked_to_u64()
        .ok_or(TokenEyeError::InvalidResponse("chain id exceeds 64 bits"))
}

/// A mockable source of ERC-20 balances used for economic admission.
#[async_trait]
pub trait TokenBalanceTransport: Send + Sync {
    async fn balance_of(
        &self,
        token_contract: Address,
        holder: Address,
    ) -> Result<U256, TokenEyeError>;
}

/// An Ethereum JSON-RPC implementation of [`TokenBalanceTransport`].
pub struct JsonRpcTokenTransport {
    client: Client,
    endpoint: RpcEndpointHandle,
    next_request_id: AtomicU64,
    expected_chain_id: Option<u64>,
}

#[derive(Clone)]
pub struct RpcEndpointHandle(Arc<std::sync::RwLock<Url>>);

impl fmt::Debug for RpcEndpointHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RpcEndpointHandle(<redacted>)")
    }
}

impl RpcEndpointHandle {
    pub fn new(endpoint: &str) -> Result<Self, TokenEyeError> {
        Ok(Self(Arc::new(std::sync::RwLock::new(parse_rpc_endpoint(
            endpoint,
        )?))))
    }

    pub fn replace(&self, endpoint: &str) -> Result<(), TokenEyeError> {
        let endpoint = parse_rpc_endpoint(endpoint)?;
        *self
            .0
            .write()
            .map_err(|_| TokenEyeError::Transport("RPC endpoint lock is poisoned".to_owned()))? =
            endpoint;
        Ok(())
    }

    pub fn current(&self) -> Result<String, TokenEyeError> {
        Ok(self
            .0
            .read()
            .map_err(|_| TokenEyeError::Transport("RPC endpoint lock is poisoned".to_owned()))?
            .as_str()
            .to_owned())
    }

    fn url(&self) -> Result<Url, TokenEyeError> {
        Ok(self
            .0
            .read()
            .map_err(|_| TokenEyeError::Transport("RPC endpoint lock is poisoned".to_owned()))?
            .clone())
    }
}

fn parse_rpc_endpoint(endpoint: &str) -> Result<Url, TokenEyeError> {
    if endpoint.len() > 4_096 {
        return Err(TokenEyeError::InvalidEndpoint(
            "endpoint exceeds the size limit",
        ));
    }
    let endpoint = Url::parse(endpoint)
        .map_err(|_| TokenEyeError::InvalidEndpoint("endpoint is not a valid URL"))?;
    if !matches!(endpoint.scheme(), "http" | "https") || endpoint.host_str().is_none() {
        return Err(TokenEyeError::InvalidEndpoint(
            "endpoint must be an HTTP or HTTPS URL with a host",
        ));
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err(TokenEyeError::InvalidEndpoint(
            "endpoint must not contain embedded credentials",
        ));
    }
    if endpoint.scheme() != "https" && !endpoint.host_str().is_some_and(is_loopback_host) {
        return Err(TokenEyeError::InvalidEndpoint(
            "endpoint must use HTTPS except for loopback development",
        ));
    }
    Ok(endpoint)
}

impl JsonRpcTokenTransport {
    pub fn new(endpoint: &str) -> Result<Self, TokenEyeError> {
        Self::with_timeout(endpoint, DEFAULT_RPC_TIMEOUT)
    }

    pub fn with_timeout(endpoint: &str, timeout: Duration) -> Result<Self, TokenEyeError> {
        Self::with_timeout_and_chain(endpoint, timeout, None)
    }

    pub fn for_chain(endpoint: &str, expected_chain_id: u64) -> Result<Self, TokenEyeError> {
        Self::with_timeout_and_chain(endpoint, DEFAULT_RPC_TIMEOUT, Some(expected_chain_id))
    }

    fn with_timeout_and_chain(
        endpoint: &str,
        timeout: Duration,
        expected_chain_id: Option<u64>,
    ) -> Result<Self, TokenEyeError> {
        let client = Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| TokenEyeError::Http("could not build HTTP client"))?;
        Ok(Self {
            client,
            endpoint: RpcEndpointHandle::new(endpoint)?,
            next_request_id: AtomicU64::new(1),
            expected_chain_id,
        })
    }

    pub fn for_chain_with_handle(
        endpoint: RpcEndpointHandle,
        expected_chain_id: u64,
    ) -> Result<Self, TokenEyeError> {
        let client = Client::builder()
            .timeout(DEFAULT_RPC_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| TokenEyeError::Http("could not build HTTP client"))?;
        Ok(Self {
            client,
            endpoint,
            next_request_id: AtomicU64::new(1),
            expected_chain_id: Some(expected_chain_id),
        })
    }

    pub async fn validate_chain(&self) -> Result<(), TokenEyeError> {
        self.verify_chain().await
    }

    async fn post_json(&self, request: Value) -> Result<Value, TokenEyeError> {
        let mut response = self
            .client
            .post(self.endpoint.url()?)
            .header("Accept", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(sanitize_reqwest_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(TokenEyeError::HttpStatus(status.as_u16()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| TokenEyeError::Http("could not read response"))?
        {
            if body
                .len()
                .checked_add(chunk.len())
                .is_none_or(|size| size > MAX_RPC_RESPONSE_BYTES)
            {
                return Err(TokenEyeError::InvalidResponse(
                    "response body exceeds the size limit",
                ));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body)
            .map_err(|_| TokenEyeError::InvalidResponse("response body is not valid JSON"))
    }

    async fn verify_chain(&self) -> Result<(), TokenEyeError> {
        let Some(expected_chain_id) = self.expected_chain_id else {
            return Ok(());
        };
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let body = self.post_json(chain_id_rpc_request(id)).await?;
        let actual_chain_id = parse_chain_id_rpc_response(body, id)?;
        if actual_chain_id != expected_chain_id {
            return Err(TokenEyeError::Transport(format!(
                "RPC endpoint reported chain id {actual_chain_id}, expected {expected_chain_id}"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl TokenBalanceTransport for JsonRpcTokenTransport {
    async fn balance_of(
        &self,
        token_contract: Address,
        holder: Address,
    ) -> Result<U256, TokenEyeError> {
        self.verify_chain().await?;
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let body = self
            .post_json(balance_of_rpc_request(id, token_contract, holder))
            .await?;
        parse_balance_of_rpc_response(body, id)
    }
}

fn sanitize_reqwest_error(error: reqwest::Error) -> TokenEyeError {
    // Do not surface the URL: RPC provider URLs commonly contain API tokens.
    let reason = if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "could not connect"
    } else if error.is_request() {
        "could not send request"
    } else {
        "request failed"
    };
    TokenEyeError::Http(reason)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReputationTier {
    Whale,
    Elder,
    Acolyte,
    Initiate,
    Unproven,
}

impl ReputationTier {
    pub const fn priority(self) -> u8 {
        match self {
            Self::Unproven => 0,
            Self::Initiate => 1,
            Self::Acolyte => 2,
            Self::Elder => 3,
            Self::Whale => 4,
        }
    }

    pub const fn meets(self, minimum: Self) -> bool {
        self.priority() >= minimum.priority()
    }
}

/// Controls percentile tiers and what counts as more than a minimal holding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TierPolicy {
    whale_basis_points: u16,
    elder_basis_points: u16,
    acolyte_minimum: U256,
}

impl TierPolicy {
    pub fn new(
        whale_basis_points: u16,
        elder_basis_points: u16,
        acolyte_minimum: U256,
    ) -> Result<Self, TokenEyeError> {
        if whale_basis_points > elder_basis_points || elder_basis_points > 10_000 {
            return Err(TokenEyeError::InvalidTierPolicy(
                "percentiles must satisfy whale <= elder <= 10000 basis points",
            ));
        }
        Ok(Self {
            whale_basis_points,
            elder_basis_points,
            acolyte_minimum,
        })
    }

    pub const fn whale_basis_points(self) -> u16 {
        self.whale_basis_points
    }

    pub const fn elder_basis_points(self) -> u16 {
        self.elder_basis_points
    }

    pub const fn acolyte_minimum(self) -> U256 {
        self.acolyte_minimum
    }
}

impl Default for TierPolicy {
    fn default() -> Self {
        Self {
            whale_basis_points: 100,
            elder_basis_points: 1_000,
            // Clanker tokens conventionally use 18 decimals. Callers can
            // replace this raw-unit threshold for a token with other decimals.
            acolyte_minimum: U256::from_u64(DEFAULT_ACOLYTE_MINIMUM_RAW),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationFreshness {
    /// A successful RPC query was performed for this observation.
    Fresh,
    /// An unexpired local value was used without an RPC query.
    Cached,
    /// RPC failed and an expired local value was retained.
    Stale,
    /// No successful observation exists for this address.
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BalanceObservation {
    pub holder: Address,
    pub balance: Option<U256>,
    pub observed_at: Option<u64>,
    pub tier: ReputationTier,
    pub freshness: ObservationFreshness,
    /// Present when a refresh failed. Cached stale data is diagnostic only and cannot authorize an
    /// economic or lifecycle operation.
    pub error: Option<TokenEyeError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationalBalanceObservation {
    pub holder: Address,
    pub balance: U256,
    pub observed_at: u64,
    pub tier: ReputationTier,
    pub freshness: ObservationFreshness,
}

impl BalanceObservation {
    /// Converts a diagnostic observation into operation-bearing economic data.
    ///
    /// Stale and unknown observations are hard failures even when a historical balance is present.
    pub fn require_operational(self) -> Result<OperationalBalanceObservation, TokenEyeError> {
        match self.freshness {
            ObservationFreshness::Fresh | ObservationFreshness::Cached => {
                let balance = self.balance.ok_or(TokenEyeError::EconomicDataUnavailable(
                    "current observation has no balance",
                ))?;
                let observed_at =
                    self.observed_at
                        .ok_or(TokenEyeError::EconomicDataUnavailable(
                            "current observation has no timestamp",
                        ))?;
                if let Some(error) = self.error {
                    return Err(error);
                }
                Ok(OperationalBalanceObservation {
                    holder: self.holder,
                    balance,
                    observed_at,
                    tier: self.tier,
                    freshness: self.freshness,
                })
            }
            ObservationFreshness::Stale => Err(TokenEyeError::EconomicDataUnavailable(
                "token balance is stale after an RPC failure",
            )),
            ObservationFreshness::Unknown => Err(TokenEyeError::EconomicDataUnavailable(
                "token balance is unknown",
            )),
        }
    }
}

#[derive(Clone, Debug)]
struct FailedRefresh {
    failed_at: u64,
    error: TokenEyeError,
}

/// The local token view maintained by one Tentacle.
#[derive(Clone, Debug)]
pub struct TokenObservance {
    pub balances: HashMap<Address, U256>,
    pub last_seen: HashMap<Address, u64>,
    pub reputation_tiers: HashMap<Address, ReputationTier>,
    observation_interval: Duration,
    tier_policy: TierPolicy,
    failed_refreshes: HashMap<Address, FailedRefresh>,
}

impl TokenObservance {
    pub fn new(observation_interval: Duration) -> Self {
        Self::with_policy(observation_interval, TierPolicy::default())
    }

    pub fn with_policy(observation_interval: Duration, tier_policy: TierPolicy) -> Self {
        Self {
            balances: HashMap::new(),
            last_seen: HashMap::new(),
            reputation_tiers: HashMap::new(),
            observation_interval,
            tier_policy,
            failed_refreshes: HashMap::new(),
        }
    }

    pub const fn observation_interval(&self) -> Duration {
        self.observation_interval
    }

    pub fn set_observation_interval(&mut self, observation_interval: Duration) {
        self.observation_interval = observation_interval;
    }

    pub const fn tier_policy(&self) -> TierPolicy {
        self.tier_policy
    }

    pub fn set_tier_policy(&mut self, tier_policy: TierPolicy) {
        self.tier_policy = tier_policy;
        self.recompute_tiers();
    }

    pub fn is_refresh_due(&self, holder: Address, now_unix_seconds: u64) -> bool {
        self.last_seen.get(&holder).is_none_or(|last_seen| {
            Duration::from_secs(now_unix_seconds.saturating_sub(*last_seen))
                >= self.observation_interval
        })
    }

    pub fn cached(&self, holder: Address, now_unix_seconds: u64) -> BalanceObservation {
        let Some(balance) = self.balances.get(&holder).copied() else {
            return unknown_observation(holder, None);
        };
        let observed_at = self.last_seen.get(&holder).copied();
        let freshness = if self.is_refresh_due(holder, now_unix_seconds) {
            ObservationFreshness::Stale
        } else {
            ObservationFreshness::Cached
        };
        BalanceObservation {
            holder,
            balance: Some(balance),
            observed_at,
            tier: self.tier_for(holder),
            freshness,
            error: None,
        }
    }

    /// Returns a cached value while it is current; otherwise refreshes it.
    ///
    /// Transport failures do not erase prior knowledge and do not turn an
    /// unknown address into a fabricated zero balance. Repeated ordinary
    /// observations of a failed holder use a short per-holder negative cache.
    pub async fn observe<T: TokenBalanceTransport + ?Sized>(
        &mut self,
        transport: &T,
        token_contract: Address,
        holder: Address,
        now_unix_seconds: u64,
    ) -> BalanceObservation {
        if !self.is_refresh_due(holder, now_unix_seconds) {
            return self.cached(holder, now_unix_seconds);
        }
        if let Some(observation) = self.backed_off(holder, now_unix_seconds) {
            return observation;
        }

        self.refresh(transport, token_contract, holder, now_unix_seconds)
            .await
    }

    /// Forces an immediate chain read, bypassing both the success cache and
    /// failure backoff. Economic action admission uses this path instead of
    /// waiting for the ordinary observation interval.
    pub async fn observe_fresh<T: TokenBalanceTransport + ?Sized>(
        &mut self,
        transport: &T,
        token_contract: Address,
        holder: Address,
        now_unix_seconds: u64,
    ) -> BalanceObservation {
        self.refresh(transport, token_contract, holder, now_unix_seconds)
            .await
    }

    async fn refresh<T: TokenBalanceTransport + ?Sized>(
        &mut self,
        transport: &T,
        token_contract: Address,
        holder: Address,
        now_unix_seconds: u64,
    ) -> BalanceObservation {
        let result = transport.balance_of(token_contract, holder).await;
        self.apply_refresh(holder, now_unix_seconds, result)
    }

    fn apply_refresh(
        &mut self,
        holder: Address,
        now_unix_seconds: u64,
        result: Result<U256, TokenEyeError>,
    ) -> BalanceObservation {
        match result {
            Ok(balance) => {
                self.record_balance(holder, balance, now_unix_seconds);
                BalanceObservation {
                    holder,
                    balance: Some(balance),
                    observed_at: Some(now_unix_seconds),
                    tier: self.tier_for(holder),
                    freshness: ObservationFreshness::Fresh,
                    error: None,
                }
            }
            Err(error) => {
                self.failed_refreshes.insert(
                    holder,
                    FailedRefresh {
                        failed_at: now_unix_seconds,
                        error: error.clone(),
                    },
                );
                self.failed_observation(holder, error)
            }
        }
    }

    pub fn record_balance(&mut self, holder: Address, balance: U256, observed_at: u64) {
        self.balances.insert(holder, balance);
        self.last_seen.insert(holder, observed_at);
        self.failed_refreshes.remove(&holder);
        self.recompute_tiers();
    }

    pub fn forget(&mut self, holder: Address) {
        self.balances.remove(&holder);
        self.last_seen.remove(&holder);
        self.reputation_tiers.remove(&holder);
        self.failed_refreshes.remove(&holder);
        self.recompute_tiers();
    }

    pub fn tier_for(&self, holder: Address) -> ReputationTier {
        self.reputation_tiers
            .get(&holder)
            .copied()
            .unwrap_or(ReputationTier::Unproven)
    }

    /// Returns a diagnostic negative-cache entry while the deterministic
    /// per-holder retry backoff is active. The backoff tracks the configured
    /// observation interval but is always between one and thirty seconds.
    fn backed_off(&self, holder: Address, now_unix_seconds: u64) -> Option<BalanceObservation> {
        let failed = self.failed_refreshes.get(&holder)?;
        let elapsed = Duration::from_secs(now_unix_seconds.saturating_sub(failed.failed_at));
        if elapsed >= self.failure_backoff() {
            return None;
        }
        Some(self.failed_observation(holder, failed.error.clone()))
    }

    fn failure_backoff(&self) -> Duration {
        self.observation_interval
            .max(MIN_FAILURE_BACKOFF)
            .min(MAX_FAILURE_BACKOFF)
    }

    fn failed_observation(&self, holder: Address, error: TokenEyeError) -> BalanceObservation {
        let Some(balance) = self.balances.get(&holder).copied() else {
            return unknown_observation(holder, Some(error));
        };
        BalanceObservation {
            holder,
            balance: Some(balance),
            observed_at: self.last_seen.get(&holder).copied(),
            tier: self.tier_for(holder),
            freshness: ObservationFreshness::Stale,
            error: Some(error),
        }
    }

    /// Recomputes ranks only from qualifying balances this Tentacle has observed locally.
    ///
    /// Percentile ranks exclude zero balances and holdings below the acolyte
    /// minimum. This prevents a collection of dust wallets from manufacturing
    /// a meaningful population and promoting an otherwise ordinary holder.
    pub fn recompute_tiers(&mut self) {
        let mut eligible: Vec<U256> = self
            .balances
            .values()
            .copied()
            .filter(|balance| !balance.is_zero() && *balance >= self.tier_policy.acolyte_minimum)
            .collect();
        eligible.sort_unstable_by(|left, right| right.cmp(left));

        self.reputation_tiers.clear();
        for (holder, balance) in &self.balances {
            let tier = classify_balance(*balance, &eligible, self.tier_policy);
            self.reputation_tiers.insert(*holder, tier);
        }
    }
}

/// Thread-safe runtime wrapper around one Tentacle's local token view.
pub struct TokenEye {
    token_contract: Address,
    expected_chain_id: Option<u64>,
    transport: Arc<dyn TokenBalanceTransport>,
    observance: Mutex<TokenObservance>,
}

impl TokenEye {
    pub fn new(
        token_contract: Address,
        transport: Arc<dyn TokenBalanceTransport>,
        observation_interval: Duration,
    ) -> Self {
        Self::new_with_policy(
            token_contract,
            transport,
            observation_interval,
            TierPolicy::default(),
        )
    }

    pub fn new_with_policy(
        token_contract: Address,
        transport: Arc<dyn TokenBalanceTransport>,
        observation_interval: Duration,
        tier_policy: TierPolicy,
    ) -> Self {
        Self {
            token_contract,
            expected_chain_id: None,
            transport,
            observance: Mutex::new(TokenObservance::with_policy(
                observation_interval,
                tier_policy,
            )),
        }
    }

    pub fn json_rpc(
        endpoint: &str,
        token_contract: Address,
        observation_interval: Duration,
    ) -> Result<Self, TokenEyeError> {
        Ok(Self::new(
            token_contract,
            Arc::new(JsonRpcTokenTransport::new(endpoint)?),
            observation_interval,
        ))
    }

    pub fn json_rpc_for_chain(
        endpoint: &str,
        token_contract: Address,
        observation_interval: Duration,
        expected_chain_id: u64,
    ) -> Result<Self, TokenEyeError> {
        Self::json_rpc_for_chain_with_policy(
            endpoint,
            token_contract,
            observation_interval,
            expected_chain_id,
            TierPolicy::default(),
        )
    }

    pub fn json_rpc_for_chain_with_policy(
        endpoint: &str,
        token_contract: Address,
        observation_interval: Duration,
        expected_chain_id: u64,
        tier_policy: TierPolicy,
    ) -> Result<Self, TokenEyeError> {
        let mut eye = Self::new_with_policy(
            token_contract,
            Arc::new(JsonRpcTokenTransport::for_chain(
                endpoint,
                expected_chain_id,
            )?),
            observation_interval,
            tier_policy,
        );
        eye.expected_chain_id = Some(expected_chain_id);
        Ok(eye)
    }

    pub fn json_rpc_for_chain_with_handle_and_policy(
        endpoint: RpcEndpointHandle,
        token_contract: Address,
        observation_interval: Duration,
        expected_chain_id: u64,
        tier_policy: TierPolicy,
    ) -> Result<Self, TokenEyeError> {
        let mut eye = Self::new_with_policy(
            token_contract,
            Arc::new(JsonRpcTokenTransport::for_chain_with_handle(
                endpoint,
                expected_chain_id,
            )?),
            observation_interval,
            tier_policy,
        );
        eye.expected_chain_id = Some(expected_chain_id);
        Ok(eye)
    }

    pub const fn token_contract(&self) -> Address {
        self.token_contract
    }

    pub const fn expected_chain_id(&self) -> Option<u64> {
        self.expected_chain_id
    }

    pub async fn observe(&self, holder: Address, now_unix_seconds: u64) -> BalanceObservation {
        {
            let observance = self.observance.lock().await;
            if !observance.is_refresh_due(holder, now_unix_seconds) {
                return observance.cached(holder, now_unix_seconds);
            }
            if let Some(observation) = observance.backed_off(holder, now_unix_seconds) {
                return observation;
            }
        }

        // Never retain the global observance lock across network I/O. Balance
        // reads for unrelated holders can take the full RPC timeout and must
        // remain independent of one another.
        let result = self.transport.balance_of(self.token_contract, holder).await;
        self.observance
            .lock()
            .await
            .apply_refresh(holder, now_unix_seconds, result)
    }

    pub async fn observe_fresh(
        &self,
        holder: Address,
        now_unix_seconds: u64,
    ) -> BalanceObservation {
        let result = self.transport.balance_of(self.token_contract, holder).await;
        self.observance
            .lock()
            .await
            .apply_refresh(holder, now_unix_seconds, result)
    }

    /// Requires a current balance for an operation that changes economic or lifecycle state.
    pub async fn observe_required(
        &self,
        holder: Address,
        now_unix_seconds: u64,
    ) -> Result<OperationalBalanceObservation, TokenEyeError> {
        self.observe(holder, now_unix_seconds)
            .await
            .require_operational()
    }

    /// Forces a chain read and hard-fails if the RPC response cannot authorize the operation.
    pub async fn observe_fresh_required(
        &self,
        holder: Address,
        now_unix_seconds: u64,
    ) -> Result<OperationalBalanceObservation, TokenEyeError> {
        self.observe_fresh(holder, now_unix_seconds)
            .await
            .require_operational()
    }
}

fn classify_balance(
    balance: U256,
    positive_descending: &[U256],
    policy: TierPolicy,
) -> ReputationTier {
    if balance.is_zero() {
        return ReputationTier::Unproven;
    }
    if balance < policy.acolyte_minimum {
        return ReputationTier::Initiate;
    }

    // `partition_point` counts values strictly greater than this balance. Tied
    // holders therefore receive the same tier rather than being split by an
    // arbitrary address ordering.
    let greater = positive_descending.partition_point(|candidate| *candidate > balance);
    let population = positive_descending.len();
    if falls_within_percentile(greater, population, policy.whale_basis_points) {
        ReputationTier::Whale
    } else if falls_within_percentile(greater, population, policy.elder_basis_points) {
        ReputationTier::Elder
    } else {
        ReputationTier::Acolyte
    }
}

fn falls_within_percentile(greater: usize, population: usize, basis_points: u16) -> bool {
    // A percentile becomes meaningful only when the eligible local sample can
    // contain at least one whole member of it. This yields a minimum of 100
    // qualifying holders for the default top 1% Whale tier and 10 for the
    // default top 10% Elder tier (generically ceil(10_000 / basis_points)).
    basis_points != 0
        && (population as u128) * u128::from(basis_points) >= BASIS_POINTS_DENOMINATOR
        && (greater as u128) * BASIS_POINTS_DENOMINATOR
            < (population as u128) * u128::from(basis_points)
}

fn unknown_observation(holder: Address, error: Option<TokenEyeError>) -> BalanceObservation {
    BalanceObservation {
        holder,
        balance: None,
        observed_at: None,
        tier: ReputationTier::Unproven,
        freshness: ObservationFreshness::Unknown,
        error,
    }
}

fn decode_hex_exact(encoded: &[u8], output: &mut [u8]) -> Result<(), &'static str> {
    if encoded.len() != output.len() * 2 {
        return Err("hexadecimal value has the wrong length");
    }
    for (pair, byte) in encoded.chunks_exact(2).zip(output.iter_mut()) {
        let high = decode_hex_nibble(pair[0]).ok_or("value contains a non-hexadecimal digit")?;
        let low = decode_hex_nibble(pair[1]).ok_or("value contains a non-hexadecimal digit")?;
        *byte = (high << 4) | low;
    }
    Ok(())
}

fn decode_hex_right_aligned(encoded: &[u8], output: &mut [u8]) -> Result<(), &'static str> {
    let mut source_index = encoded.len();
    let mut output_index = output.len();
    while source_index > 0 {
        output_index = output_index
            .checked_sub(1)
            .ok_or("hexadecimal value exceeds destination width")?;
        let low_index = source_index - 1;
        let low = decode_hex_nibble(encoded[low_index])
            .ok_or("value contains a non-hexadecimal digit")?;
        let (high, consumed) = if low_index > 0 {
            (
                decode_hex_nibble(encoded[low_index - 1])
                    .ok_or("value contains a non-hexadecimal digit")?,
                2,
            )
        } else {
            (0, 1)
        };
        output[output_index] = (high << 4) | low;
        source_index -= consumed;
    }
    Ok(())
}

const fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn push_hex_byte(output: &mut String, value: u8) {
    push_hex_nibble(output, value >> 4);
    push_hex_nibble(output, value & 0x0f);
}

fn push_hex_nibble(output: &mut String, value: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    output.push(char::from(HEX[usize::from(value)]));
}

fn limit_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering as AtomicOrdering},
        },
    };

    const CONTRACT: Address = Address::from_bytes([0x11; 20]);
    const HOLDER: Address = Address::from_bytes([0x22; 20]);

    #[test]
    fn address_parsing_is_strict_and_round_trips() {
        let lower = "0x0123456789abcdef0123456789abcdef01234567";
        let upper = "0x0123456789ABCDEF0123456789ABCDEF01234567";
        let parsed: Address = lower.parse().unwrap();
        assert_eq!(parsed.to_string(), lower);
        assert_eq!(upper.parse::<Address>().unwrap(), parsed);
        assert!(lower[2..].parse::<Address>().is_err());
        assert!(format!("0X{}", &lower[2..]).parse::<Address>().is_err());
        assert!(format!("{lower}00").parse::<Address>().is_err());
        assert!(
            "0x0123456789abcdef0123456789abcdef0123456g"
                .parse::<Address>()
                .is_err()
        );

        let serialized = serde_json::to_string(&parsed).unwrap();
        assert_eq!(serialized, format!("\"{lower}\""));
        assert_eq!(
            serde_json::from_str::<Address>(&serialized).unwrap(),
            parsed
        );
    }

    #[test]
    fn quantity_parsing_enforces_json_rpc_canonical_form() {
        assert_eq!(U256::from_quantity("0x0").unwrap(), U256::ZERO);
        assert_eq!(U256::from_quantity("0xf").unwrap(), U256::from_u64(15));
        assert_eq!(U256::from_quantity("0x10").unwrap(), U256::from_u64(16));
        assert_eq!(U256::from_quantity("0xABC").unwrap().to_quantity(), "0xabc");

        assert!(U256::from_quantity("0x").is_err());
        assert!(U256::from_quantity("0x00").is_err());
        assert!(U256::from_quantity("0x01").is_err());
        assert!(U256::from_quantity("0X1").is_err());
        assert!(U256::from_quantity("0xgg").is_err());
        assert!(U256::from_quantity(&format!("0x1{}", "0".repeat(64))).is_err());
    }

    #[test]
    fn full_width_u256_and_abi_words_round_trip() {
        let maximum_quantity = format!("0x{}", "f".repeat(64));
        let maximum = U256::from_quantity(&maximum_quantity).unwrap();
        assert_eq!(maximum.to_quantity(), maximum_quantity);
        assert_eq!(maximum.to_abi_word(), maximum_quantity);
        assert_eq!(
            U256::from_abi_word(&maximum.to_abi_word()).unwrap(),
            maximum
        );

        let one_word = format!("0x{}1", "0".repeat(63));
        assert_eq!(U256::from_abi_word(&one_word).unwrap(), U256::from_u64(1));
        assert!(U256::from_abi_word("0x1").is_err());
        assert!(U256::from_abi_word(&format!("0x{}z", "0".repeat(63))).is_err());
        let one_billion_with_18_decimals =
            U256::from_quantity("0x33b2e3c9fd0803ce8000000").unwrap();
        assert_eq!(one_billion_with_18_decimals.whole_units(18), 1_000_000_000);
        assert_eq!(U256::power_of_ten(18).unwrap().whole_units(18), 1);
        assert!(U256::power_of_ten(77).is_some());
        assert!(U256::power_of_ten(78).is_none());
        assert!(maximum.checked_to_u64().is_none());
    }

    #[test]
    fn checked_supply_scaling_rejects_uint256_overflow() {
        let ten_to_77 = U256::power_of_ten(77).unwrap();
        assert!(ten_to_77.checked_mul_u64(1).is_some());
        assert!(ten_to_77.checked_mul_u64(2).is_none());
    }

    #[test]
    fn balance_of_calldata_and_rpc_request_are_exact() {
        let calldata = encode_balance_of_call(HOLDER);
        assert_eq!(
            calldata,
            format!(
                "0x70a08231000000000000000000000000{}",
                &HOLDER.to_string()[2..]
            )
        );
        assert_eq!(calldata.len(), 74);

        let request = balance_of_rpc_request(7, CONTRACT, HOLDER);
        assert_eq!(request["jsonrpc"], "2.0");
        assert_eq!(request["id"], 7);
        assert_eq!(request["method"], "eth_call");
        assert_eq!(request["params"][0]["to"], CONTRACT.to_string());
        assert_eq!(request["params"][0]["data"], calldata);
        assert_eq!(request["params"][1], "latest");
    }

    #[test]
    fn balance_of_rpc_response_is_strict() {
        let word = U256::from_u64(42).to_abi_word();
        assert_eq!(
            parse_balance_of_rpc_response(json!({"jsonrpc": "2.0", "id": 9, "result": word}), 9,)
                .unwrap(),
            U256::from_u64(42)
        );
        assert!(
            parse_balance_of_rpc_response(
                json!({"jsonrpc": "2.0", "id": 8, "result": U256::ZERO.to_abi_word()}),
                9,
            )
            .is_err()
        );
        assert!(
            parse_balance_of_rpc_response(json!({"jsonrpc": "2.0", "id": 9, "result": "0x0"}), 9,)
                .is_err()
        );

        let error = parse_balance_of_rpc_response(
            json!({
                "jsonrpc": "2.0",
                "id": 9,
                "error": {"code": -32000, "message": "execution reverted"}
            }),
            9,
        )
        .unwrap_err();
        assert_eq!(
            error,
            TokenEyeError::Rpc {
                code: -32000,
                message: "execution reverted".to_owned(),
            }
        );
    }

    #[test]
    fn chain_id_request_and_response_bind_base_without_a_private_key() {
        let request = chain_id_rpc_request(3);
        assert_eq!(request["method"], "eth_chainId");
        assert_eq!(request["params"], json!([]));
        assert_eq!(
            parse_chain_id_rpc_response(json!({"jsonrpc": "2.0", "id": 3, "result": "0x2105"}), 3,)
                .unwrap(),
            8_453
        );
        assert!(
            parse_chain_id_rpc_response(json!({"jsonrpc": "2.0", "id": 4, "result": "0x1"}), 3,)
                .is_err()
        );
    }

    #[tokio::test]
    async fn live_transport_revalidates_chain_id_before_every_balance_read() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let responses = [
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x2105"}).to_string(),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": U256::from_u64(7).to_abi_word()
            })
            .to_string(),
            json!({"jsonrpc": "2.0", "id": 3, "result": "0x1"}).to_string(),
        ];
        let server = std::thread::spawn(move || {
            let mut methods = Vec::new();
            for response_body in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0_u8; 4_096];
                    let read = stream.read(&mut chunk).unwrap();
                    assert_ne!(read, 0, "client closed before sending an HTTP request");
                    request.extend_from_slice(&chunk[..read]);

                    let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = std::str::from_utf8(&request[..header_end]).unwrap();
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    let body_start = header_end + 4;
                    if request.len() >= body_start + content_length {
                        let body: Value = serde_json::from_slice(
                            &request[body_start..body_start + content_length],
                        )
                        .unwrap();
                        methods.push(body["method"].as_str().unwrap().to_owned());
                        break;
                    }
                }

                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                )
                .unwrap();
                stream.flush().unwrap();
            }
            methods
        });

        let transport = JsonRpcTokenTransport::for_chain(&endpoint, 8_453).unwrap();
        assert_eq!(
            transport.balance_of(CONTRACT, HOLDER).await.unwrap(),
            U256::from_u64(7)
        );
        assert_eq!(
            transport.balance_of(CONTRACT, HOLDER).await.unwrap_err(),
            TokenEyeError::Transport("RPC endpoint reported chain id 1, expected 8453".to_owned())
        );
        assert_eq!(
            server.join().unwrap(),
            ["eth_chainId", "eth_call", "eth_chainId"]
        );
    }

    #[test]
    fn rpc_endpoint_validation_rejects_embedded_credentials() {
        assert!(JsonRpcTokenTransport::new("https://base.example.invalid").is_ok());
        assert!(JsonRpcTokenTransport::new("http://127.0.0.1:8545").is_ok());
        assert!(JsonRpcTokenTransport::new("http://base.example.invalid").is_err());
        assert!(JsonRpcTokenTransport::new("file:///tmp/base.sock").is_err());
        let error = JsonRpcTokenTransport::new("https://secret@base.example.invalid")
            .err()
            .unwrap();
        assert_eq!(
            error,
            TokenEyeError::InvalidEndpoint("endpoint must not contain embedded credentials")
        );
        assert!(!error.to_string().contains("secret"));
    }

    struct MockTransport {
        replies: Mutex<VecDeque<Result<U256, TokenEyeError>>>,
        calls: AtomicUsize,
    }

    impl MockTransport {
        fn new(replies: Vec<Result<U256, TokenEyeError>>) -> Self {
            Self {
                replies: Mutex::new(replies.into()),
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(AtomicOrdering::Relaxed)
        }
    }

    #[async_trait]
    impl TokenBalanceTransport for MockTransport {
        async fn balance_of(
            &self,
            _token_contract: Address,
            _holder: Address,
        ) -> Result<U256, TokenEyeError> {
            self.calls.fetch_add(1, AtomicOrdering::Relaxed);
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(TokenEyeError::Transport("no mock reply".to_owned())))
        }
    }

    #[tokio::test]
    async fn stale_diagnostics_are_retained_but_operations_hard_fail() {
        let transport = MockTransport::new(vec![
            Ok(U256::from_u64(25)),
            Err(TokenEyeError::Transport("chain unavailable".to_owned())),
            Ok(U256::from_u64(30)),
        ]);
        let mut eye = TokenObservance::new(Duration::from_secs(10));

        let fresh = eye.observe(&transport, CONTRACT, HOLDER, 100).await;
        assert_eq!(fresh.balance, Some(U256::from_u64(25)));
        assert_eq!(fresh.freshness, ObservationFreshness::Fresh);
        assert_eq!(fresh.observed_at, Some(100));
        assert_eq!(transport.calls(), 1);

        let cached = eye.observe(&transport, CONTRACT, HOLDER, 109).await;
        assert_eq!(cached.freshness, ObservationFreshness::Cached);
        assert_eq!(cached.error, None);
        assert_eq!(transport.calls(), 1);

        let stale = eye.observe(&transport, CONTRACT, HOLDER, 110).await;
        assert_eq!(stale.balance, Some(U256::from_u64(25)));
        assert_eq!(stale.freshness, ObservationFreshness::Stale);
        assert_eq!(stale.observed_at, Some(100));
        assert!(stale.error.is_some());
        assert_eq!(
            stale.clone().require_operational().unwrap_err(),
            TokenEyeError::EconomicDataUnavailable("token balance is stale after an RPC failure")
        );
        assert_eq!(eye.last_seen[&HOLDER], 100);
        assert_eq!(transport.calls(), 2);

        let backed_off = eye.observe(&transport, CONTRACT, HOLDER, 119).await;
        assert_eq!(backed_off.balance, Some(U256::from_u64(25)));
        assert_eq!(backed_off.freshness, ObservationFreshness::Stale);
        assert_eq!(backed_off.error, stale.error);
        assert_eq!(transport.calls(), 2);

        let recovered = eye.observe(&transport, CONTRACT, HOLDER, 120).await;
        assert_eq!(recovered.balance, Some(U256::from_u64(30)));
        assert_eq!(recovered.freshness, ObservationFreshness::Fresh);
        assert_eq!(transport.calls(), 3);

        // A clock moving backwards never creates an underflow or forces a
        // refresh of a value observed in the apparent future.
        assert!(!eye.is_refresh_due(HOLDER, 99));
    }

    #[tokio::test]
    async fn unknown_failure_is_negatively_cached_but_fresh_reads_bypass_backoff() {
        let outage = TokenEyeError::Transport("chain unavailable".to_owned());
        let transport = MockTransport::new(vec![Err(outage.clone()), Ok(U256::from_u64(7))]);
        let mut eye = TokenObservance::new(Duration::from_secs(60));
        assert_eq!(eye.failure_backoff(), Duration::from_secs(30));

        let unknown = eye.observe(&transport, CONTRACT, HOLDER, 100).await;
        assert_eq!(unknown.freshness, ObservationFreshness::Unknown);
        assert_eq!(unknown.error, Some(outage.clone()));
        assert_eq!(
            unknown.require_operational().unwrap_err(),
            TokenEyeError::EconomicDataUnavailable("token balance is unknown")
        );
        assert_eq!(transport.calls(), 1);

        let backed_off = eye.observe(&transport, CONTRACT, HOLDER, 129).await;
        assert_eq!(backed_off.freshness, ObservationFreshness::Unknown);
        assert_eq!(backed_off.error, Some(outage));
        assert_eq!(transport.calls(), 1);

        let fresh = eye.observe_fresh(&transport, CONTRACT, HOLDER, 129).await;
        assert_eq!(fresh.balance, Some(U256::from_u64(7)));
        assert_eq!(fresh.freshness, ObservationFreshness::Fresh);
        assert_eq!(transport.calls(), 2);

        let zero_interval = TokenObservance::new(Duration::ZERO);
        assert_eq!(zero_interval.failure_backoff(), Duration::from_secs(1));
    }

    #[tokio::test]
    async fn rpc_failure_for_unknown_holder_does_not_fabricate_zero() {
        let transport = MockTransport::new(vec![Err(TokenEyeError::Transport(
            "chain unavailable".to_owned(),
        ))]);
        let mut eye = TokenObservance::new(Duration::from_secs(30));
        let observation = eye.observe(&transport, CONTRACT, HOLDER, 100).await;
        assert_eq!(observation.balance, None);
        assert_eq!(observation.observed_at, None);
        assert_eq!(observation.tier, ReputationTier::Unproven);
        assert_eq!(observation.freshness, ObservationFreshness::Unknown);
        assert!(observation.error.is_some());
        assert!(!eye.balances.contains_key(&HOLDER));
        assert!(!eye.last_seen.contains_key(&HOLDER));
    }

    #[test]
    fn local_percentile_and_balance_tier_boundaries_are_deterministic() {
        let policy = TierPolicy::new(100, 1_000, U256::from_u64(50)).unwrap();
        let mut eye = TokenObservance::with_policy(Duration::from_secs(60), policy);
        let mut holders = Vec::new();
        // Values 50..=149 form exactly 100 percentile-eligible holders;
        // values below 50 remain Initiates and do not affect those ranks.
        for value in 1_u64..=149 {
            let mut bytes = [0_u8; 20];
            bytes[12..].copy_from_slice(&value.to_be_bytes());
            let holder = Address::from_bytes(bytes);
            holders.push(holder);
            eye.balances.insert(holder, U256::from_u64(value));
        }
        eye.recompute_tiers();

        assert_eq!(eye.tier_for(holders[148]), ReputationTier::Whale); // rank 1
        assert_eq!(eye.tier_for(holders[147]), ReputationTier::Elder); // rank 2
        assert_eq!(eye.tier_for(holders[139]), ReputationTier::Elder); // rank 10
        assert_eq!(eye.tier_for(holders[138]), ReputationTier::Acolyte); // rank 11
        assert_eq!(eye.tier_for(holders[49]), ReputationTier::Acolyte); // threshold
        assert_eq!(eye.tier_for(holders[48]), ReputationTier::Initiate);

        let zero = Address::from_bytes([0xee; 20]);
        eye.balances.insert(zero, U256::ZERO);
        eye.recompute_tiers();
        assert_eq!(eye.tier_for(zero), ReputationTier::Unproven);
    }

    #[test]
    fn dust_and_small_samples_do_not_inflate_percentile_tiers() {
        let policy = TierPolicy::new(100, 1_000, U256::from_u64(50)).unwrap();
        let mut eye = TokenObservance::with_policy(Duration::from_secs(60), policy);

        eye.record_balance(HOLDER, U256::from_u64(1), 1);
        assert_eq!(eye.tier_for(HOLDER), ReputationTier::Initiate);

        // Ninety-nine dust addresses cannot bootstrap one qualifying holder
        // into Whale or Elder: only balances meeting the acolyte minimum form
        // the percentile population.
        for suffix in 1_u8..100 {
            let mut bytes = [0_u8; 20];
            bytes[19] = suffix;
            eye.balances
                .insert(Address::from_bytes(bytes), U256::from_u64(1));
        }
        let eligible = Address::from_bytes([0xaa; 20]);
        eye.balances.insert(eligible, U256::from_u64(100));
        eye.recompute_tiers();
        assert_eq!(eye.tier_for(eligible), ReputationTier::Acolyte);

        // Ten qualifying holders make the top-10% Elder rank meaningful, but
        // still do not create a top-1% Whale rank.
        for suffix in 1_u8..10 {
            let mut bytes = [0xbb; 20];
            bytes[19] = suffix;
            eye.balances
                .insert(Address::from_bytes(bytes), U256::from_u64(50));
        }
        eye.recompute_tiers();
        assert_eq!(eye.tier_for(eligible), ReputationTier::Elder);
        assert!(
            eye.reputation_tiers
                .values()
                .all(|tier| *tier != ReputationTier::Whale)
        );
    }

    #[test]
    fn tied_balances_receive_the_same_percentile_tier() {
        let policy = TierPolicy::new(100, 1_000, U256::from_u64(1)).unwrap();
        let mut eye = TokenObservance::with_policy(Duration::ZERO, policy);
        for suffix in 0_u8..100 {
            let mut bytes = [0_u8; 20];
            bytes[19] = suffix;
            let balance = if suffix < 2 {
                100
            } else {
                u64::from(100 - suffix)
            };
            eye.balances
                .insert(Address::from_bytes(bytes), U256::from_u64(balance));
        }
        eye.recompute_tiers();

        assert_eq!(
            eye.tier_for(Address::from_bytes([0; 20])),
            ReputationTier::Whale
        );
        let mut second = [0_u8; 20];
        second[19] = 1;
        assert_eq!(
            eye.tier_for(Address::from_bytes(second)),
            ReputationTier::Whale
        );
    }

    #[test]
    fn tier_policy_and_interval_are_reconfigurable() {
        let mut eye = TokenObservance::new(Duration::from_secs(60));
        eye.record_balance(HOLDER, U256::from_u64(1), 10);
        assert_eq!(eye.tier_for(HOLDER), ReputationTier::Initiate);

        eye.set_observation_interval(Duration::ZERO);
        assert!(eye.is_refresh_due(HOLDER, 10));
        let disabled_percentiles = TierPolicy::new(0, 0, U256::from_u64(2)).unwrap();
        eye.set_tier_policy(disabled_percentiles);
        assert_eq!(eye.tier_for(HOLDER), ReputationTier::Initiate);
        assert!(TierPolicy::new(1_001, 1_000, U256::ZERO).is_err());
        assert!(TierPolicy::new(100, 10_001, U256::ZERO).is_err());
    }

    struct ConcurrentTransport {
        entered: tokio::sync::Barrier,
    }

    #[async_trait]
    impl TokenBalanceTransport for ConcurrentTransport {
        async fn balance_of(
            &self,
            _token_contract: Address,
            _holder: Address,
        ) -> Result<U256, TokenEyeError> {
            self.entered.wait().await;
            Ok(U256::from_u64(1))
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn token_eye_does_not_hold_observance_lock_during_rpc() {
        let transport = Arc::new(ConcurrentTransport {
            entered: tokio::sync::Barrier::new(2),
        });
        let eye = TokenEye::new_with_policy(
            CONTRACT,
            transport,
            Duration::from_secs(60),
            TierPolicy::new(100, 1_000, U256::from_u64(1)).unwrap(),
        );
        let other_holder = Address::from_bytes([0x33; 20]);

        let (first, second) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(eye.observe(HOLDER, 100), eye.observe(other_holder, 100))
        })
        .await
        .expect("unrelated holders should enter the transport concurrently");

        assert_eq!(first.freshness, ObservationFreshness::Fresh);
        assert_eq!(second.freshness, ObservationFreshness::Fresh);
    }

    #[tokio::test]
    async fn token_eye_ordinary_reads_back_off_while_fresh_reads_do_not() {
        let transport = Arc::new(MockTransport::new(vec![
            Err(TokenEyeError::Transport("chain unavailable".to_owned())),
            Ok(U256::from_u64(9)),
        ]));
        let eye = TokenEye::new(CONTRACT, transport.clone(), Duration::from_secs(10));

        assert_eq!(
            eye.observe(HOLDER, 100).await.freshness,
            ObservationFreshness::Unknown
        );
        assert_eq!(
            eye.observe(HOLDER, 101).await.freshness,
            ObservationFreshness::Unknown
        );
        assert_eq!(transport.calls(), 1);

        let fresh = eye.observe_fresh(HOLDER, 101).await;
        assert_eq!(fresh.balance, Some(U256::from_u64(9)));
        assert_eq!(fresh.freshness, ObservationFreshness::Fresh);
        assert_eq!(transport.calls(), 2);
    }
}
