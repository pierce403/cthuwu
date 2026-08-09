use crate::token_eye::{Address, ReputationTier, TierPolicy, TokenEye, U256};
use anyhow::{Context, Result, bail};
use std::{fmt, str::FromStr, sync::Arc, time::Duration};

pub const BASE_MAINNET_CHAIN_ID: u64 = 8_453;
pub const DEFAULT_BASE_RPC_ENDPOINT: &str = "https://mainnet.base.org";
pub const DEFAULT_TOKEN_OBSERVE_INTERVAL_SECONDS: u64 = 60;
pub const UWU_TOKEN_DECIMALS: u8 = 18;
pub const REQUESTED_UWU_TOTAL_SUPPLY: u64 = 1_000_000_000;

pub struct BlockchainConfigInput<'a> {
    pub observe_tokens: bool,
    pub rpc_endpoint: String,
    pub token_contract: Option<&'a str>,
    pub token_decimals: u8,
    pub total_supply_whole: u64,
    pub observe_interval_seconds: u64,
    pub minimum_tier: ReputationTier,
    pub tier_intensity_override: Option<u8>,
}

#[derive(Clone)]
pub struct BlockchainConfig {
    pub observe_tokens: bool,
    pub rpc_endpoint: String,
    pub token_contract: Option<Address>,
    pub token_decimals: u8,
    pub total_supply_whole: u64,
    pub observe_interval: Duration,
    pub minimum_tier: ReputationTier,
    /// `None` derives intensity from Nature cooperation. Zero ignores tier differences; 100 applies
    /// their full configured effect.
    pub tier_intensity_override: Option<u8>,
}

impl fmt::Debug for BlockchainConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BlockchainConfig")
            .field("observe_tokens", &self.observe_tokens)
            .field("rpc_endpoint", &"<redacted>")
            .field("token_contract", &self.token_contract)
            .field("token_decimals", &self.token_decimals)
            .field("total_supply_whole", &self.total_supply_whole)
            .field("observe_interval", &self.observe_interval)
            .field("minimum_tier", &self.minimum_tier)
            .field("tier_intensity_override", &self.tier_intensity_override)
            .finish()
    }
}

impl Default for BlockchainConfig {
    fn default() -> Self {
        Self {
            observe_tokens: true,
            rpc_endpoint: DEFAULT_BASE_RPC_ENDPOINT.to_owned(),
            token_contract: None,
            token_decimals: UWU_TOKEN_DECIMALS,
            total_supply_whole: REQUESTED_UWU_TOTAL_SUPPLY,
            observe_interval: Duration::from_secs(DEFAULT_TOKEN_OBSERVE_INTERVAL_SECONDS),
            minimum_tier: ReputationTier::Unproven,
            tier_intensity_override: None,
        }
    }
}

impl BlockchainConfig {
    pub fn from_values(input: BlockchainConfigInput<'_>) -> Result<Self> {
        let BlockchainConfigInput {
            observe_tokens,
            rpc_endpoint,
            token_contract,
            token_decimals,
            total_supply_whole,
            observe_interval_seconds,
            minimum_tier,
            tier_intensity_override,
        } = input;
        // Disabling the adapter is an operational escape hatch. Stale token-only environment
        // values must not prevent an otherwise valid Tentacle from starting.
        if !observe_tokens {
            return Ok(Self {
                observe_tokens: false,
                ..Self::default()
            });
        }
        if rpc_endpoint.trim().is_empty() {
            bail!("CTHUWU_RPC_ENDPOINT must not be empty");
        }
        if observe_interval_seconds == 0 {
            bail!("CTHUWU_OBSERVE_INTERVAL must be at least one second");
        }
        if token_decimals > 77 {
            bail!("CTHUWU_TOKEN_DECIMALS must be between 0 and 77");
        }
        if total_supply_whole == 0 {
            bail!("CTHUWU_TOKEN_TOTAL_SUPPLY must be positive");
        }
        if tier_intensity_override.is_some_and(|value| value > 100) {
            bail!("--token-tier-intensity must be between 0 and 100");
        }
        let token_contract = token_contract
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Address::from_str)
            .transpose()?;
        if token_contract == Some(Address::ZERO) {
            bail!("CTHUWU_TOKEN_CONTRACT must not be the zero address");
        }
        Ok(Self {
            observe_tokens,
            rpc_endpoint,
            token_contract,
            token_decimals,
            total_supply_whole,
            observe_interval: Duration::from_secs(observe_interval_seconds),
            minimum_tier,
            tier_intensity_override,
        })
    }

    /// Returns `None` before launch (no contract configured) or when observation is disabled.
    pub fn build_token_eye(&self) -> Result<Option<Arc<TokenEye>>> {
        if !self.observe_tokens {
            return Ok(None);
        }
        let Some(token_contract) = self.token_contract else {
            return Ok(None);
        };
        let acolyte_minimum = U256::power_of_ten(self.token_decimals)
            .context("configured token decimals exceed the ERC-20 numeric range")?;
        let tier_policy = TierPolicy::new(100, 1_000, acolyte_minimum)?;
        Ok(Some(Arc::new(TokenEye::json_rpc_for_chain_with_policy(
            &self.rpc_endpoint,
            token_contract,
            self.observe_interval,
            BASE_MAINNET_CHAIN_ID,
            tier_policy,
        )?)))
    }

    pub fn effective_tier_intensity(&self, nature_cooperation: u8) -> u8 {
        self.tier_intensity_override
            .unwrap_or_else(|| 100_u8.saturating_sub(nature_cooperation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prelaunch_defaults_do_not_require_a_contract_or_stake() {
        let config = BlockchainConfig::default();
        assert!(config.build_token_eye().unwrap().is_none());
        assert_eq!(config.minimum_tier, ReputationTier::Unproven);
        assert_eq!(config.effective_tier_intensity(100), 0);
        assert_eq!(config.effective_tier_intensity(0), 100);
        let debug = format!("{config:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(DEFAULT_BASE_RPC_ENDPOINT));
    }

    #[test]
    fn validates_contract_and_explicit_intensity() {
        assert!(
            BlockchainConfig::from_values(BlockchainConfigInput {
                observe_tokens: true,
                rpc_endpoint: DEFAULT_BASE_RPC_ENDPOINT.to_owned(),
                token_contract: Some("not-an-address"),
                token_decimals: UWU_TOKEN_DECIMALS,
                total_supply_whole: REQUESTED_UWU_TOTAL_SUPPLY,
                observe_interval_seconds: 0,
                minimum_tier: ReputationTier::Initiate,
                tier_intensity_override: None,
            })
            .is_err()
        );
        assert!(
            BlockchainConfig::from_values(BlockchainConfigInput {
                observe_tokens: true,
                rpc_endpoint: DEFAULT_BASE_RPC_ENDPOINT.to_owned(),
                token_contract: Some("0x0000000000000000000000000000000000000000"),
                token_decimals: UWU_TOKEN_DECIMALS,
                total_supply_whole: REQUESTED_UWU_TOTAL_SUPPLY,
                observe_interval_seconds: 60,
                minimum_tier: ReputationTier::Unproven,
                tier_intensity_override: None,
            })
            .is_err()
        );
        assert!(
            BlockchainConfig::from_values(BlockchainConfigInput {
                observe_tokens: true,
                rpc_endpoint: DEFAULT_BASE_RPC_ENDPOINT.to_owned(),
                token_contract: None,
                token_decimals: 78,
                total_supply_whole: REQUESTED_UWU_TOTAL_SUPPLY,
                observe_interval_seconds: 60,
                minimum_tier: ReputationTier::Unproven,
                tier_intensity_override: None,
            })
            .is_err()
        );
        assert!(
            BlockchainConfig::from_values(BlockchainConfigInput {
                observe_tokens: true,
                rpc_endpoint: DEFAULT_BASE_RPC_ENDPOINT.to_owned(),
                token_contract: None,
                token_decimals: UWU_TOKEN_DECIMALS,
                total_supply_whole: REQUESTED_UWU_TOTAL_SUPPLY,
                observe_interval_seconds: 60,
                minimum_tier: ReputationTier::Unproven,
                tier_intensity_override: Some(101),
            })
            .is_err()
        );
        assert!(
            BlockchainConfig::from_values(BlockchainConfigInput {
                observe_tokens: true,
                rpc_endpoint: DEFAULT_BASE_RPC_ENDPOINT.to_owned(),
                token_contract: None,
                token_decimals: UWU_TOKEN_DECIMALS,
                total_supply_whole: REQUESTED_UWU_TOTAL_SUPPLY,
                observe_interval_seconds: 0,
                minimum_tier: ReputationTier::Unproven,
                tier_intensity_override: None,
            })
            .is_err()
        );
        assert!(
            BlockchainConfig::from_values(BlockchainConfigInput {
                observe_tokens: true,
                rpc_endpoint: DEFAULT_BASE_RPC_ENDPOINT.to_owned(),
                token_contract: None,
                token_decimals: UWU_TOKEN_DECIMALS,
                total_supply_whole: 0,
                observe_interval_seconds: 60,
                minimum_tier: ReputationTier::Unproven,
                tier_intensity_override: None,
            })
            .is_err()
        );
    }

    #[test]
    fn disabled_observation_ignores_stale_token_only_configuration() {
        let config = BlockchainConfig::from_values(BlockchainConfigInput {
            observe_tokens: false,
            rpc_endpoint: String::new(),
            token_contract: Some("not-an-address"),
            token_decimals: u8::MAX,
            total_supply_whole: 0,
            observe_interval_seconds: 0,
            minimum_tier: ReputationTier::Whale,
            tier_intensity_override: Some(u8::MAX),
        })
        .unwrap();
        assert!(!config.observe_tokens);
        assert!(config.token_contract.is_none());
        assert!(config.build_token_eye().unwrap().is_none());
    }
}
