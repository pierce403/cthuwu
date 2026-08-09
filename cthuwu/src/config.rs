use crate::token_eye::{Address, ReputationTier, TierPolicy, TokenEye, U256};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
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
    pub tentacle_wallet: Option<&'a str>,
    pub stake_contract: Option<&'a str>,
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
    /// Treasury whose verified holdings drive this Tentacle's Wealth and starvation state.
    pub tentacle_wallet: Option<Address>,
    /// Optional ERC-20-compatible staking receipt contract queried with `balanceOf(tentacle_wallet)`.
    pub stake_contract: Option<Address>,
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
            .field("tentacle_wallet", &self.tentacle_wallet)
            .field("stake_contract", &self.stake_contract)
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
            tentacle_wallet: None,
            stake_contract: None,
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
            tentacle_wallet,
            stake_contract,
            token_decimals,
            total_supply_whole,
            observe_interval_seconds,
            minimum_tier,
            tier_intensity_override,
        } = input;
        // Explicit disablement is a testing/development mode, not a fallback from an enabled
        // economy. An enabled runtime below requires its contract and treasury identity.
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
        U256::power_of_ten(token_decimals)
            .and_then(|scale| scale.checked_mul_u64(total_supply_whole))
            .context(
                "configured whole-token supply and decimals exceed the ERC-20 uint256 range",
            )?;
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
        let tentacle_wallet = tentacle_wallet
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Address::from_str)
            .transpose()?;
        if tentacle_wallet == Some(Address::ZERO) {
            bail!("CTHUWU_TENTACLE_WALLET must not be the zero address");
        }
        let stake_contract = stake_contract
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Address::from_str)
            .transpose()?;
        if stake_contract == Some(Address::ZERO) {
            bail!("CTHUWU_STAKE_CONTRACT must not be the zero address");
        }
        if token_contract.is_none() {
            bail!("CTHUWU_TOKEN_CONTRACT is required while token economics are enabled");
        }
        if tentacle_wallet.is_none() {
            bail!("CTHUWU_TENTACLE_WALLET is required while token economics are enabled");
        }
        Ok(Self {
            observe_tokens,
            rpc_endpoint,
            token_contract,
            tentacle_wallet,
            stake_contract,
            token_decimals,
            total_supply_whole,
            observe_interval: Duration::from_secs(observe_interval_seconds),
            minimum_tier,
            tier_intensity_override,
        })
    }

    /// Returns `None` only when observation was explicitly disabled.
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

    /// Builds a separate observer for an ERC-20-compatible staking receipt contract. A missing
    /// staking contract is represented as zero stake, so it blocks propagation under a positive
    /// stake policy without preventing a zero-stake Tentacle from starting.
    pub fn build_stake_eye(&self) -> Result<Option<Arc<TokenEye>>> {
        if !self.observe_tokens {
            return Ok(None);
        }
        let Some(stake_contract) = self.stake_contract else {
            return Ok(None);
        };
        let acolyte_minimum = U256::power_of_ten(self.token_decimals)
            .context("configured token decimals exceed the ERC-20 numeric range")?;
        let tier_policy = TierPolicy::new(100, 1_000, acolyte_minimum)?;
        Ok(Some(Arc::new(TokenEye::json_rpc_for_chain_with_policy(
            &self.rpc_endpoint,
            stake_contract,
            self.observe_interval,
            BASE_MAINNET_CHAIN_ID,
            tier_policy,
        )?)))
    }

    pub fn normalize_balance_basis_points(&self, balance: U256) -> u16 {
        let whole_tokens = balance.whole_units(self.token_decimals);
        let normalized = u128::from(whole_tokens)
            .saturating_mul(10_000)
            .checked_div(u128::from(self.total_supply_whole))
            .unwrap_or(0)
            .min(10_000);
        u16::try_from(normalized).unwrap_or(10_000)
    }

    /// Stable, secret-free identity for every setting that changes normalized lifecycle evidence.
    pub fn economic_configuration_identity(
        &self,
        propagation_minimum_stake_basis_points: u16,
    ) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(BASE_MAINNET_CHAIN_ID.to_be_bytes());
        digest.update(
            self.token_contract
                .map_or(Address::ZERO, |address| address)
                .as_bytes(),
        );
        digest.update(
            self.stake_contract
                .map_or(Address::ZERO, |address| address)
                .as_bytes(),
        );
        digest.update(
            self.tentacle_wallet
                .map_or(Address::ZERO, |address| address)
                .as_bytes(),
        );
        digest.update([self.token_decimals]);
        digest.update(self.total_supply_whole.to_be_bytes());
        digest.update(propagation_minimum_stake_basis_points.to_be_bytes());
        digest.finalize().into()
    }

    /// Canonical personal-sign message proving control of the exact configured treasury identity.
    pub fn treasury_attestation_message(
        &self,
        propagation_minimum_stake_basis_points: u16,
    ) -> Result<String> {
        let token_contract = self
            .token_contract
            .context("token contract is required for treasury attestation")?;
        let treasury = self
            .tentacle_wallet
            .context("Tentacle wallet is required for treasury attestation")?;
        let stake_contract = self.stake_contract.unwrap_or(Address::ZERO);
        let identity = self.economic_configuration_identity(propagation_minimum_stake_basis_points);
        let mut identity_hex = String::with_capacity(64);
        for byte in identity {
            use std::fmt::Write as _;
            write!(&mut identity_hex, "{byte:02x}")?;
        }
        Ok(format!(
            "CTHUWU treasury attestation v1\nchain_id={BASE_MAINNET_CHAIN_ID}\ntoken_contract={token_contract}\ntreasury={treasury}\nstake_contract={stake_contract}\ntoken_decimals={}\ntotal_supply={}\npropagation_minimum_stake_bps={propagation_minimum_stake_basis_points}\nconfiguration_identity=0x{identity_hex}",
            self.token_decimals, self.total_supply_whole,
        ))
    }

    pub fn effective_tier_intensity(&self, nature_cooperation: u8) -> u8 {
        self.tier_intensity_override
            .unwrap_or_else(|| 100_u8.saturating_sub(nature_cooperation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_WALLET: &str = "0x1111111111111111111111111111111111111111";

    fn enabled_config(wallet: &str) -> BlockchainConfig {
        BlockchainConfig::from_values(BlockchainConfigInput {
            observe_tokens: true,
            rpc_endpoint: DEFAULT_BASE_RPC_ENDPOINT.to_owned(),
            token_contract: Some("0x2222222222222222222222222222222222222222"),
            tentacle_wallet: Some(wallet),
            stake_contract: Some("0x3333333333333333333333333333333333333333"),
            token_decimals: UWU_TOKEN_DECIMALS,
            total_supply_whole: REQUESTED_UWU_TOTAL_SUPPLY,
            observe_interval_seconds: 60,
            minimum_tier: ReputationTier::Unproven,
            tier_intensity_override: None,
        })
        .unwrap()
    }

    #[test]
    fn enabled_startup_requires_a_contract_and_tentacle_treasury() {
        let config = BlockchainConfig::default();
        assert!(config.build_token_eye().unwrap().is_none());
        assert_eq!(config.minimum_tier, ReputationTier::Unproven);
        assert_eq!(config.effective_tier_intensity(100), 0);
        assert_eq!(config.effective_tier_intensity(0), 100);
        let debug = format!("{config:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(DEFAULT_BASE_RPC_ENDPOINT));

        assert!(
            BlockchainConfig::from_values(BlockchainConfigInput {
                observe_tokens: true,
                rpc_endpoint: DEFAULT_BASE_RPC_ENDPOINT.to_owned(),
                token_contract: None,
                tentacle_wallet: None,
                stake_contract: None,
                token_decimals: UWU_TOKEN_DECIMALS,
                total_supply_whole: REQUESTED_UWU_TOTAL_SUPPLY,
                observe_interval_seconds: 60,
                minimum_tier: ReputationTier::Unproven,
                tier_intensity_override: None,
            })
            .is_err()
        );
    }

    #[test]
    fn validates_contract_and_explicit_intensity() {
        assert!(
            BlockchainConfig::from_values(BlockchainConfigInput {
                observe_tokens: true,
                rpc_endpoint: DEFAULT_BASE_RPC_ENDPOINT.to_owned(),
                token_contract: Some("not-an-address"),
                tentacle_wallet: Some(TEST_WALLET),
                stake_contract: None,
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
                tentacle_wallet: Some(TEST_WALLET),
                stake_contract: None,
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
                tentacle_wallet: Some(TEST_WALLET),
                stake_contract: None,
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
                tentacle_wallet: Some(TEST_WALLET),
                stake_contract: None,
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
                tentacle_wallet: Some(TEST_WALLET),
                stake_contract: None,
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
                tentacle_wallet: Some(TEST_WALLET),
                stake_contract: None,
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
            tentacle_wallet: Some("also-not-an-address"),
            stake_contract: Some("still-not-an-address"),
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

    #[test]
    fn enabled_supply_must_fit_the_erc20_uint256_range() {
        let result = BlockchainConfig::from_values(BlockchainConfigInput {
            observe_tokens: true,
            rpc_endpoint: DEFAULT_BASE_RPC_ENDPOINT.to_owned(),
            token_contract: Some("0x2222222222222222222222222222222222222222"),
            tentacle_wallet: Some(TEST_WALLET),
            stake_contract: None,
            token_decimals: 77,
            total_supply_whole: 2,
            observe_interval_seconds: 60,
            minimum_tier: ReputationTier::Unproven,
            tier_intensity_override: None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn treasury_attestation_binds_wallet_stake_policy_and_configuration_identity() {
        let first = enabled_config(TEST_WALLET);
        let second = enabled_config("0x4444444444444444444444444444444444444444");
        assert_ne!(
            first.economic_configuration_identity(100),
            second.economic_configuration_identity(100)
        );
        assert_ne!(
            first.economic_configuration_identity(100),
            first.economic_configuration_identity(101)
        );

        let message = first.treasury_attestation_message(100).unwrap();
        assert!(message.starts_with("CTHUWU treasury attestation v1\nchain_id=8453\n"));
        assert!(message.contains("token_contract=0x2222222222222222222222222222222222222222"));
        assert!(message.contains("treasury=0x1111111111111111111111111111111111111111"));
        assert!(message.contains("stake_contract=0x3333333333333333333333333333333333333333"));
        assert!(message.contains("total_supply=1000000000"));
        assert!(message.contains("propagation_minimum_stake_bps=100"));
        assert!(message.contains("configuration_identity=0x"));
        assert_ne!(message, second.treasury_attestation_message(100).unwrap());
    }
}
