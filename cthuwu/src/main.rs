mod agent_context;
mod autonomy;
pub mod avatar;
pub mod awakening;
mod base_rpc;
mod bot;
mod branding;
mod config;
mod contact;
mod deadline;
mod dedupe;
pub mod economics;
mod erc8004;
pub mod evolution;
pub mod evolution_runtime;
mod growth;
pub mod health;
pub mod hermes;
pub mod image_gen;
mod inference;
mod matching;
mod model;
mod names;
mod operator;
pub mod personality;
mod principal;
mod repository_maintenance;
pub mod scales;
mod sidecar;
mod storage;
pub mod token_eye;
pub mod token_gov;
mod web_search;

use agent_context::AgentContext;
use anyhow::{Context, Result, bail, ensure};
use autonomy::LifecycleExecutor;
use base_rpc::{BaseRpcControl, BaseRpcStore};
use bot::UwUBot;
use branding::{BrandingDeliveryTarget, SharedBrandingControl};
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use config::{
    BlockchainConfig, BlockchainConfigInput, DEFAULT_BASE_RPC_ENDPOINT,
    DEFAULT_TOKEN_OBSERVE_INTERVAL_SECONDS, DEFAULT_UWU_TOKEN_CONTRACT, UWU_TOKEN_DECIMALS,
    UWU_TOTAL_SUPPLY,
};
use contact::ContactStore;
use cthuwu_council::run_deterministic_simulation;
use dedupe::ProcessedMessages;
use economics::{
    DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS, EconomicHolderRole, EconomicObservationProvenance,
    TokenEconomicSnapshot,
};
use erc8004::{
    BASE_MAINNET_CHAIN_ID as ERC8004_CHAIN_ID, IDENTITY_REGISTRY, REPUTATION_REGISTRY,
    RegistrationConfig, RegistrationPhase, SharedRegistrationControl, SidecarErc8004Gateway,
    TentacleRegistration, active_operator_inboxes,
};
use evolution::{LifecycleAction, LifecycleReceipt, LifecycleReceiptStatus, LineageStore};
use evolution_runtime::{EvolutionRuntime, EvolutionStartupOptions, MandatoryRecoveryKind};
use growth::{DEFAULT_PUBLIC_ORIGIN, DEFAULT_REFERRAL_BOUNTY_BASE_UNITS};
use inference::{
    DEFAULT_OLLAMA_ENDPOINT, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_TIMEOUT_SECONDS,
    DEFAULT_VENICE_MODEL, DEFAULT_VENICE_TIMEOUT_SECONDS, InferenceConfig, InferenceRouter,
    Provider,
};
use model::Model;
use operator::{LocalOperatorTools, ModelControl, OperatorHarness, OperatorModel};
use principal::OperatorStore;
use sidecar::{
    OperatorNotice, manage_global_group, resolve_operator_inbox, resolve_xmtp_wallet_address,
    run_xmtp_sidecar,
};
use std::{
    collections::BTreeSet,
    env,
    fs::{self, OpenOptions},
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use storage::{ensure_private_directory, sync_directory};
use token_eye::{Address, ReputationTier, TokenEye};
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use web_search::{BraveSafeSearch, BraveWebSearch, WebSearch};

#[derive(Debug, Parser)]
#[command(
    name = "uwubot",
    version,
    about = "Run Cthuwu's one-to-one XMTP companion"
)]
struct Cli {
    #[arg(long, env = "UWUBOT_DATA_DIR", default_value = ".")]
    data_dir: PathBuf,

    #[arg(long, env = "UWUBOT_XMTP_ENV", value_enum, default_value_t = Network::Production)]
    xmtp_env: Network,

    /// Sole operator Ethereum address or ENS name. Without it, the first authenticated EVM DM
    /// sender is imprinted as operator for subsequent messages.
    #[arg(long, env = "UWUBOT_OPERATOR")]
    operator: Option<String>,

    /// Optional startup override. Without one, the persisted selection or Venice default is used.
    #[arg(long, env = "UWUBOT_MODEL", value_enum)]
    model: Option<ModelKind>,

    #[arg(long, env = "UWUBOT_MODEL_ENDPOINT")]
    model_endpoint: Option<String>,

    #[arg(long, env = "UWUBOT_MODEL_NAME")]
    model_name: Option<String>,

    #[arg(long, env = "UWUBOT_MODEL_API_KEY", hide = true)]
    model_api_key: Option<String>,

    /// Venice credential. VENICE_API_KEY is also accepted and takes precedence when identical.
    #[arg(long, env = "UWUBOT_VENICE_API_KEY", hide = true)]
    venice_api_key: Option<String>,

    #[arg(
        long,
        env = "UWUBOT_VENICE_MODEL",
        default_value = DEFAULT_VENICE_MODEL
    )]
    venice_model: String,

    #[arg(
        long,
        env = "UWUBOT_VENICE_TIMEOUT_SECONDS",
        default_value_t = DEFAULT_VENICE_TIMEOUT_SECONDS
    )]
    venice_timeout_seconds: u64,

    #[arg(
        long,
        env = "UWUBOT_OLLAMA_ENDPOINT",
        default_value = DEFAULT_OLLAMA_ENDPOINT
    )]
    ollama_endpoint: String,

    #[arg(
        long,
        env = "UWUBOT_OLLAMA_MODEL",
        default_value = DEFAULT_OLLAMA_MODEL
    )]
    ollama_model: String,

    #[arg(
        long,
        env = "UWUBOT_OLLAMA_TIMEOUT_SECONDS",
        default_value_t = DEFAULT_OLLAMA_TIMEOUT_SECONDS
    )]
    ollama_timeout_seconds: u64,

    /// Optional normal-user web-search adapter. Public chat never receives local tools.
    #[arg(long, env = "UWUBOT_WEB_SEARCH", value_enum, default_value_t = WebSearchKind::None)]
    web_search: WebSearchKind,

    #[arg(long, env = "UWUBOT_WEB_SEARCH_API_KEY", hide = true)]
    web_search_api_key: Option<String>,

    #[arg(
        long,
        env = "UWUBOT_WEB_SEARCH_ENDPOINT",
        default_value = "https://api.search.brave.com/res/v1/web/search"
    )]
    web_search_endpoint: String,

    /// Brave SafeSearch mode. Content filtering is off unless the operator opts in.
    #[arg(
        long,
        env = "UWUBOT_WEB_SEARCH_SAFESEARCH",
        default_value_t = BraveSafeSearch::Off
    )]
    web_search_safesearch: BraveSafeSearch,

    /// Base JSON-RPC endpoint used for holder/stake reads and passed exactly to the lifecycle executor.
    #[arg(
        long,
        env = "CTHUWU_RPC_ENDPOINT",
        default_value = DEFAULT_BASE_RPC_ENDPOINT,
        hide_env_values = true
    )]
    rpc_endpoint: String,

    /// Enable canonical Base-mainnet ERC-8004 registration and read-only verification.
    #[arg(
        long,
        env = "CTHUWU_ERC8004_ENABLED",
        default_value_t = true,
        action = ArgAction::Set
    )]
    erc8004_enabled: bool,

    /// Automatically pursue registration after safe discovery and sufficient Base ETH.
    #[arg(
        long,
        env = "CTHUWU_ERC8004_AUTO_REGISTER",
        default_value_t = true,
        action = ArgAction::Set
    )]
    erc8004_auto_register: bool,

    /// Expected chain. Production accepts canonical Base mainnet only.
    #[arg(long, env = "CTHUWU_ERC8004_CHAIN_ID", default_value_t = ERC8004_CHAIN_ID)]
    erc8004_chain_id: u64,

    /// Expected canonical Base ERC-8004 Identity Registry.
    #[arg(long, env = "CTHUWU_ERC8004_IDENTITY_REGISTRY", default_value = IDENTITY_REGISTRY)]
    erc8004_identity_registry: String,

    /// Expected canonical Base ERC-8004 Reputation Registry.
    #[arg(long, env = "CTHUWU_ERC8004_REPUTATION_REGISTRY", default_value = REPUTATION_REGISTRY)]
    erc8004_reputation_registry: String,

    /// Optional migration/diagnostic hint. It is verified and never selects over the canonical ID.
    #[arg(long, env = "CTHUWU_ERC8004_AGENT_ID")]
    erc8004_agent_id: Option<String>,

    #[arg(long, env = "CTHUWU_ERC8004_CONFIRMATIONS", default_value_t = 12)]
    erc8004_confirmations: u64,

    #[arg(
        long,
        env = "CTHUWU_ERC8004_NOTIFICATION_COOLDOWN_SECONDS",
        default_value_t = 24 * 60 * 60
    )]
    erc8004_notification_cooldown_seconds: u64,

    #[arg(
        long,
        env = "CTHUWU_ERC8004_MAINTENANCE_INTERVAL_SECONDS",
        default_value_t = 15 * 60
    )]
    erc8004_maintenance_interval_seconds: u64,

    #[arg(long, env = "CTHUWU_ERC8004_GAS_SAFETY_BPS", default_value_t = 12_500)]
    erc8004_gas_safety_basis_points: u16,

    #[arg(
        long,
        env = "CTHUWU_ERC8004_POST_REGISTRATION_RESERVE_WEI",
        default_value = "50000000000000"
    )]
    erc8004_post_registration_reserve_wei: String,

    #[arg(
        long,
        env = "CTHUWU_ERC8004_MAX_GAS_PER_TRANSACTION",
        default_value_t = 6_000_000
    )]
    erc8004_max_gas_per_transaction: u64,

    #[arg(
        long,
        env = "CTHUWU_ERC8004_MAX_FEE_PER_GAS_WEI",
        default_value = "10000000000"
    )]
    erc8004_max_fee_per_gas_wei: String,

    /// One-time UWU onboarding referral bounty, in canonical token base units.
    #[arg(
        long,
        env = "CTHUWU_REFERRAL_BOUNTY_BASE_UNITS",
        default_value = DEFAULT_REFERRAL_BOUNTY_BASE_UNITS
    )]
    referral_bounty_base_units: String,

    /// Public HTTPS origin used to construct fragment-only recruitment links.
    #[arg(
        long,
        env = "CTHUWU_PUBLIC_ORIGIN",
        default_value = DEFAULT_PUBLIC_ORIGIN
    )]
    public_origin: String,

    /// Optional name used only when this durable Tentacle has never chosen one before.
    #[arg(long, env = "CTHUWU_ERC8004_PUBLIC_NAME")]
    erc8004_public_name: Option<String>,

    #[arg(
        long,
        env = "CTHUWU_ERC8004_PUBLIC_DESCRIPTION",
        default_value = "An independently operated Tentacle of the centerless Cthuwu collective, reachable over XMTP."
    )]
    erc8004_public_description: String,

    #[arg(
        long,
        env = "CTHUWU_ERC8004_PUBLIC_IMAGE",
        default_value = "https://cthuwu.app/icons/cthuwu-512.png"
    )]
    erc8004_public_image: String,

    /// Live UWU Clanker v4 ERC-20 address on Base mainnet.
    #[arg(
        long,
        env = "CTHUWU_TOKEN_CONTRACT",
        default_value = DEFAULT_UWU_TOKEN_CONTRACT
    )]
    token_contract: Option<String>,

    /// Optional ERC-20-compatible staking receipt contract queried for propagation stake.
    #[arg(long, env = "CTHUWU_STAKE_CONTRACT")]
    stake_contract: Option<String>,

    /// ERC-20 decimals used to normalize raw balances (Clanker standard: 18).
    #[arg(long, env = "CTHUWU_TOKEN_DECIMALS", default_value_t = UWU_TOKEN_DECIMALS)]
    token_decimals: u8,

    /// Whole-token supply used for local normalization (Clanker v4: 100 billion).
    #[arg(
        long,
        env = "CTHUWU_TOKEN_TOTAL_SUPPLY",
        default_value_t = UWU_TOTAL_SUPPLY
    )]
    token_total_supply: u64,

    /// Whole UWU paid for the first accepted Venice key from an authenticated acolyte.
    #[arg(long, env = "CTHUWU_VENICE_KEY_REWARD_WHOLE", default_value_t = 1)]
    venice_key_reward_whole: u64,

    /// Enable mandatory local token economics. Missing/failed observations block normal work.
    #[arg(
        long,
        env = "CTHUWU_OBSERVE_TOKENS",
        default_value_t = true,
        action = ArgAction::Set
    )]
    observe_tokens: bool,

    /// Cache lifetime for balance observations and maximum node-economics refresh interval.
    #[arg(
        long,
        env = "CTHUWU_OBSERVE_INTERVAL",
        default_value_t = DEFAULT_TOKEN_OBSERVE_INTERVAL_SECONDS
    )]
    observe_interval: u64,

    /// Minimum UWU tier for public interaction. Unknown/stale RPC state blocks normal work.
    #[arg(long, env = "CTHUWU_MIN_TIER", value_enum, default_value_t = TokenTierArg::Unproven)]
    min_tier: TokenTierArg,

    /// Override Nature-derived tier differentiation (0 ignores tiers; 100 uses full effects).
    #[arg(long, env = "CTHUWU_TOKEN_TIER_INTENSITY")]
    token_tier_intensity: Option<u8>,

    /// Normalized staking-receipt balance required for propagation rights.
    #[arg(
        long,
        env = "CTHUWU_PROPAGATION_MINIMUM_STAKE_BPS",
        default_value_t = DEFAULT_PROPAGATION_MINIMUM_STAKE_BPS
    )]
    propagation_minimum_stake_basis_points: u16,

    /// Override persisted automatic spawning. New Tentacles default to enabled.
    #[arg(long, env = "CTHUWU_AUTO_SPAWN", action = ArgAction::Set)]
    auto_spawn: Option<bool>,

    /// Executable that consumes one JSON lifecycle intent and returns one JSON receipt.
    #[arg(long, env = "CTHUWU_LIFECYCLE_EXECUTOR")]
    lifecycle_executor: Option<PathBuf>,

    /// Root available to authenticated operator file tools and used as the exec working directory.
    #[arg(long, env = "UWUBOT_OPERATOR_ROOT", default_value = ".")]
    operator_root: PathBuf,

    /// Optional QMD executable used only by the authenticated operator harness.
    #[arg(long, env = "UWUBOT_QMD", default_value = "qmd")]
    qmd: PathBuf,

    #[arg(
        long,
        env = "UWUBOT_OPERATOR_TOOL_TIMEOUT_SECONDS",
        default_value_t = 240
    )]
    operator_tool_timeout_seconds: u64,

    /// Node executable used for the supported XMTP transport sidecar.
    #[arg(long, env = "UWUBOT_NODE", default_value = "node")]
    node: PathBuf,

    /// Compiled XMTP transport entry point.
    #[arg(long, env = "UWUBOT_SIDECAR", default_value = "agent/dist/index.js")]
    sidecar: PathBuf,

    /// Development harness: receive newline-delimited messages for one inbox on stdin.
    #[arg(long, hide = true)]
    stdin_inbox: Option<String>,

    /// Run the opt-in deterministic local Council simulator, persist its state, and exit.
    #[arg(long, env = "UWUBOT_COUNCIL_SIMULATE", default_value_t = false)]
    council_simulate: bool,

    /// Testing-only local confirmation. The signed log records that XMTP confirmation was skipped.
    #[arg(long, env = "UWUBOT_SKIP_AWAKENING", default_value_t = false)]
    skip_awakening: bool,

    /// Start a new signed Nature epoch. Requires the explicit --force acknowledgement.
    #[arg(long, env = "UWUBOT_REROLL_NATURE", default_value_t = false)]
    reroll_nature: bool,

    /// Acknowledge the disruptive local Nature reroll.
    #[arg(long, requires = "reroll_nature", default_value_t = false)]
    force: bool,

    /// Custom signed Nature path, relative to UWUBOT_DATA_DIR/state/natures.
    #[arg(long, env = "UWUBOT_NATURE_PATH")]
    nature_path: Option<PathBuf>,

    /// Open and reconcile Evolution state, print Nature/awakening status, then exit without XMTP.
    #[arg(
        long,
        conflicts_with_all = ["skip_awakening", "reroll_nature", "force"],
        default_value_t = false
    )]
    show_nature: bool,

    /// Untrusted bootstrap peer hints. Live gossip still requires authenticated key binding.
    #[arg(long, env = "UWUBOT_GOSSIP_PEERS", value_delimiter = ',')]
    gossip_peers: Vec<String>,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Manage the local XMTP operator allowlist. This command never starts the transport.
    Operator {
        #[command(subcommand)]
        command: OperatorCommand,
    },
    /// Inspect or control this Tentacle's canonical Base ERC-8004 registration.
    Registry {
        #[command(subcommand)]
        command: RegistryCommand,
    },
    /// Bootstrap or inspect the configured production three-channel XMTP workspace.
    Chat {
        #[command(subcommand)]
        command: ChatCommand,
    },
    /// Generate or inspect this Tentacle's custom PNG avatar.
    Avatar {
        #[command(subcommand)]
        command: AvatarCommand,
    },
}

#[derive(Debug, Subcommand)]
enum AvatarCommand {
    /// Generate a custom PNG avatar using the configured image model (Venice/OpenAI).
    Generate {
        /// Optional custom prompt override.
        #[arg(long)]
        prompt: Option<String>,
    },
    /// Show the current avatar URI status.
    Status,
}

#[derive(Debug, Subcommand)]
enum ChatCommand {
    /// Create the one Global group explicitly, or inspect/reconcile its configured admin set.
    Global {
        #[command(subcommand)]
        command: GlobalGroupCommand,
    },
}

#[derive(Debug, Subcommand)]
enum GlobalGroupCommand {
    Create,
    Inspect,
}

#[derive(Debug, Subcommand)]
enum RegistryCommand {
    Status,
    Candidates,
    Adopt { agent_id: String },
    Register,
    DeclareAllegiance,
    RenounceAllegiance,
    Republish,
    Pending,
    Retry,
}

#[derive(Debug, Subcommand)]
enum OperatorCommand {
    /// Resolve an ENS name or Ethereum address and authorize its XMTP inbox.
    Add {
        /// An ENS .eth name or full 0x Ethereum address.
        identity: String,
        #[arg(long, default_value = "operator")]
        label: String,
    },
    /// Revoke an operator inbox. Revocation remains as a blocking tombstone.
    #[command(visible_alias = "remove")]
    Revoke { inbox_id: String },
    /// Retain one of several active operator records and revoke the others.
    Select { inbox_id: String },
    /// List operator inbox IDs, labels, states, and generations.
    List,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Network {
    Dev,
    Production,
    Local,
}

impl Network {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Production => "production",
            Self::Local => "local",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ModelKind {
    Venice,
    Deterministic,
    Ollama,
    Openai,
}

#[derive(Clone, Copy, Debug, ValueEnum, Eq, PartialEq)]
enum WebSearchKind {
    None,
    Brave,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum TokenTierArg {
    Whale,
    Elder,
    Acolyte,
    Initiate,
    Unproven,
}

impl From<TokenTierArg> for ReputationTier {
    fn from(value: TokenTierArg) -> Self {
        match value {
            TokenTierArg::Whale => Self::Whale,
            TokenTierArg::Elder => Self::Elder,
            TokenTierArg::Acolyte => Self::Acolyte,
            TokenTierArg::Initiate => Self::Initiate,
            TokenTierArg::Unproven => Self::Unproven,
        }
    }
}

struct EconomicDependencies {
    blockchain: BlockchainConfig,
    token_eye: Option<Arc<TokenEye>>,
    stake_eye: Option<Arc<TokenEye>>,
    propagation_minimum_stake_basis_points: u16,
    initial_node_economics: Option<(TokenEconomicSnapshot, EconomicObservationProvenance)>,
}

struct AutonomyDependencies {
    blockchain: BlockchainConfig,
    token_eye: Option<Arc<TokenEye>>,
    stake_eye: Option<Arc<TokenEye>>,
    propagation_minimum_stake_basis_points: u16,
    executor: Option<Arc<LifecycleExecutor>>,
}

impl EconomicDependencies {
    fn into_autonomy(
        self,
        propagation_minimum_stake_basis_points: u16,
        executor: Option<Arc<LifecycleExecutor>>,
    ) -> AutonomyDependencies {
        AutonomyDependencies {
            blockchain: self.blockchain,
            token_eye: self.token_eye,
            stake_eye: self.stake_eye,
            propagation_minimum_stake_basis_points,
            executor,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfiguredAgentIdAction {
    AlreadyCanonical,
    IgnoreProvenDuplicate,
    IgnoreNoncanonicalConflict,
    AdoptAmbiguousCandidate,
    DeferUntilVerified,
}

fn configured_agent_id_action(
    selected: &str,
    confirmed: Option<&str>,
    ignored_duplicates: &[String],
    candidates: &[String],
    phase: RegistrationPhase,
) -> ConfiguredAgentIdAction {
    match confirmed {
        Some(canonical) if canonical == selected => ConfiguredAgentIdAction::AlreadyCanonical,
        Some(_)
            if ignored_duplicates
                .iter()
                .any(|agent_id| agent_id == selected) =>
        {
            ConfiguredAgentIdAction::IgnoreProvenDuplicate
        }
        Some(_) => ConfiguredAgentIdAction::IgnoreNoncanonicalConflict,
        None if phase == RegistrationPhase::FailedRecoverable
            && candidates.iter().any(|agent_id| agent_id == selected) =>
        {
            ConfiguredAgentIdAction::AdoptAmbiguousCandidate
        }
        None => ConfiguredAgentIdAction::DeferUntilVerified,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "uwubot=info".into()))
        .without_time()
        .init();

    let mut cli = Cli::parse();
    let starts_normal_runtime = cli.command.is_none() && !cli.council_simulate && !cli.show_nature;
    if starts_normal_runtime && EvolutionRuntime::migrate_legacy_death_to_dormancy(&cli.data_dir)? {
        info!("migrated legacy terminal Death state to recoverable dormancy");
    }
    let mut mandatory_recovery = if starts_normal_runtime {
        EvolutionRuntime::mandatory_recovery_kind(&cli.data_dir)?
    } else {
        MandatoryRecoveryKind::None
    };
    if mandatory_recovery == MandatoryRecoveryKind::CompletedShutdown {
        info!("binding lifecycle shutdown is already complete; refusing restart");
        return Ok(());
    }
    if mandatory_recovery == MandatoryRecoveryKind::ShutdownDueOrPending
        && EvolutionRuntime::try_complete_due_native_shutdown(
            &cli.data_dir,
            current_unix_seconds()?,
        )?
        .is_some()
    {
        mandatory_recovery = EvolutionRuntime::mandatory_recovery_kind(&cli.data_dir)?;
        if mandatory_recovery == MandatoryRecoveryKind::AbsorptionProjectionRequired {
            info!(
                "completed fixed-deadline native shutdown; opening only to repair the durable absorption lineage projection"
            );
        } else {
            info!(
                "completed fixed-deadline native shutdown before constructing unrelated runtime dependencies"
            );
            return Ok(());
        }
    }
    if mandatory_recovery == MandatoryRecoveryKind::AbsorptionProjectionRequired {
        EvolutionRuntime::repair_absorption_projection(&cli.data_dir)?;
        info!(
            "repaired the terminal absorption lineage projection without opening Nature, metrics, economics, or operator state"
        );
        return Ok(());
    }
    let mut economic_dependencies = None;
    let mut lifecycle_executor = None;
    let mut base_rpc_control: Option<Arc<BaseRpcStore>> = None;
    if starts_normal_runtime {
        if mandatory_recovery == MandatoryRecoveryKind::None
            && cli.stdin_inbox.is_some()
            && !cfg!(debug_assertions)
        {
            bail!(
                "--stdin-inbox is a debug-build test harness and is unavailable in release nodes"
            );
        }
        if mandatory_recovery == MandatoryRecoveryKind::None && !cli.observe_tokens {
            bail!(
                "normal runtime requires token observation; --observe-tokens=false is limited to non-runtime management and simulation paths"
            );
        }
        if mandatory_recovery == MandatoryRecoveryKind::None
            && cli.propagation_minimum_stake_basis_points > 10_000
        {
            bail!("CTHUWU_PROPAGATION_MINIMUM_STAKE_BPS must not exceed 10000");
        }
        if mandatory_recovery == MandatoryRecoveryKind::None
            && (cli.venice_key_reward_whole == 0
                || cli.venice_key_reward_whole > cli.token_total_supply)
        {
            bail!(
                "CTHUWU_VENICE_KEY_REWARD_WHOLE must be positive and no larger than the configured whole-token supply"
            );
        }
        if !matches!(
            mandatory_recovery,
            MandatoryRecoveryKind::CompletedShutdown
                | MandatoryRecoveryKind::AbsorptionProjectionRequired
        ) {
            let control = Arc::new(BaseRpcStore::open(&cli.data_dir, &cli.rpc_endpoint)?);
            cli.rpc_endpoint = control.startup_endpoint()?;
            base_rpc_control = Some(control.clone());
            if env::var_os("CTHUWU_ECONOMICS_PRIVATE_KEY").is_some() {
                if mandatory_recovery == MandatoryRecoveryKind::None {
                    bail!(
                        "uwubot does not accept CTHUWU_ECONOMICS_PRIVATE_KEY; configure the lifecycle executor to use a separately isolated signer service"
                    );
                }
                warn!(
                    "ignoring CTHUWU_ECONOMICS_PRIVATE_KEY while draining binding recovery; uwubot never forwards raw signing keys"
                );
            }
            match build_economic_dependencies(
                &cli,
                mandatory_recovery == MandatoryRecoveryKind::None,
                Some(control.endpoint_handle()),
            )
            .await
            {
                Ok(dependencies) => economic_dependencies = Some(dependencies),
                Err(error) if mandatory_recovery != MandatoryRecoveryKind::None => {
                    warn!(
                        %error,
                        "treasury economics are unavailable; binding absorption/shutdown recovery will continue without token admission"
                    );
                }
                Err(error) => {
                    return Err(error).context(
                        "mandatory Tentacle economics preflight failed before local state mutation",
                    );
                }
            }
            if cli.lifecycle_executor.is_some() {
                match build_lifecycle_executor(&cli) {
                    Ok(executor) => lifecycle_executor = Some(executor),
                    Err(error) if mandatory_recovery != MandatoryRecoveryKind::None => {
                        warn!(
                            %error,
                            "lifecycle executor is unavailable; fixed-deadline native shutdown remains authoritative"
                        );
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }
    if mandatory_recovery != MandatoryRecoveryKind::AbsorptionProjectionRequired {
        enforce_environment(&cli.data_dir, cli.xmtp_env)?;
    }
    cli.data_dir = fs::canonicalize(&cli.data_dir)
        .with_context(|| format!("resolving data directory {}", cli.data_dir.display()))?;
    if let Some(command) = cli.command.take() {
        if cli.operator.is_some() {
            bail!(
                "--operator configures normal runtime and cannot be combined with a management command"
            );
        }
        let operators = OperatorStore::new(&cli.data_dir, cli.xmtp_env.as_str())?;
        return run_management_command(
            operators,
            command,
            &cli,
            &cli.node,
            &cli.sidecar,
            cli.xmtp_env.as_str(),
        )
        .await;
    }
    if cli.council_simulate {
        let report = run_deterministic_simulation(&cli.data_dir)
            .context("running deterministic local Council simulation")?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let recovering_binding_death = mandatory_recovery != MandatoryRecoveryKind::None;
    // Binding lifecycle recovery must not be stranded by an unrelated operator-workspace
    // configuration error. Evolution stores this path but does not touch it while admission is
    // closed; resolve and enforce isolation only if the Tentacle survives into normal operation.
    let mut operator_root = if recovering_binding_death {
        cli.operator_root.clone()
    } else {
        resolve_isolated_operator_root(&cli.data_dir, &cli.operator_root)?
    };
    let mut evolution_options = EvolutionStartupOptions {
        skip_awakening: cli.skip_awakening && !recovering_binding_death,
        auto_accept_nature: !recovering_binding_death,
        reroll_nature: cli.reroll_nature && !recovering_binding_death,
        force: cli.force && !recovering_binding_death,
        nature_path: cli.nature_path.clone(),
        gossip_peers: cli.gossip_peers.clone(),
        auto_spawn: (!recovering_binding_death)
            .then_some(cli.auto_spawn)
            .flatten(),
        propagation_minimum_stake_basis_points: if recovering_binding_death {
            cli.propagation_minimum_stake_basis_points.min(10_000)
        } else {
            cli.propagation_minimum_stake_basis_points
        },
        node_economics_ttl_seconds: cli.observe_interval.saturating_mul(2).max(1),
        survival_total_supply_whole: if recovering_binding_death && cli.token_total_supply == 0 {
            UWU_TOTAL_SUPPLY
        } else {
            cli.token_total_supply
        },
        survival_token_decimals: if recovering_binding_death {
            cli.token_decimals.min(77)
        } else {
            cli.token_decimals
        },
        ..EvolutionStartupOptions::default()
    };
    if cli.show_nature {
        let evolution = EvolutionRuntime::open(&cli.data_dir, &operator_root, evolution_options)?;
        println!("{}", evolution.nature_status());
        return Ok(());
    }
    evolution_options.initial_node_economics = if recovering_binding_death {
        None
    } else {
        economic_dependencies
            .as_ref()
            .and_then(|dependencies| dependencies.initial_node_economics)
    };
    evolution_options.require_node_economics = true;
    let evolution = Arc::new(Mutex::new(EvolutionRuntime::open(
        &cli.data_dir,
        &operator_root,
        evolution_options,
    )?));
    if evolution
        .lock()
        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
        .is_shutdown_complete()
    {
        info!("binding lifecycle shutdown is already complete; refusing supervised restart");
        return Ok(());
    }
    let recovery_pending_after_open = evolution
        .lock()
        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
        .pending_death_deadline_ms()
        .is_some();
    if mandatory_recovery != MandatoryRecoveryKind::None || recovery_pending_after_open {
        match drain_startup_recovery(
            evolution.clone(),
            economic_dependencies.as_ref(),
            lifecycle_executor.as_ref(),
        )
        .await?
        {
            StartupRecoveryOutcome::ShutdownComplete => return Ok(()),
            StartupRecoveryOutcome::Survived => {
                info!("binding Death was canceled by a confirmed survival expenditure");
            }
        }
    }
    if recovering_binding_death {
        operator_root = resolve_isolated_operator_root(&cli.data_dir, &cli.operator_root)?;
    }
    let economic_dependencies = economic_dependencies
        .context("normal runtime economics are unavailable after startup recovery")?;
    let base_rpc_control =
        base_rpc_control.context("normal runtime Base RPC provisioning control is unavailable")?;
    if lifecycle_executor.is_none() {
        info!(
            "no lifecycle executor configured; external spend, spawn, and absorption intents will remain pending"
        );
    }
    let blockchain = economic_dependencies.blockchain.clone();
    let token_eye = economic_dependencies.token_eye.clone();
    let autonomy_dependencies = economic_dependencies.into_autonomy(
        cli.propagation_minimum_stake_basis_points,
        lifecycle_executor,
    );
    let tentacle_wallet = blockchain
        .xmtp_wallet
        .context("normal runtime has no persistent Tentacle wallet")?;
    let tentacle_id = evolution
        .lock()
        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
        .local_tentacle_id()
        .to_owned();
    let registration_config = registration_config_from_cli(&cli)?;
    let registration_gateway = Arc::new(
        SidecarErc8004Gateway::new_with_handle(
            &cli.node,
            &cli.sidecar,
            &cli.data_dir,
            base_rpc_control.endpoint_handle(),
            registration_config.clone(),
        )?
        .with_referral_bounty_policy(&cli.referral_bounty_base_units)?,
    );
    let registration = Arc::new(tokio::sync::Mutex::new(TentacleRegistration::open(
        &cli.data_dir,
        &tentacle_id,
        tentacle_wallet,
        registration_config,
        registration_gateway.clone(),
    )?));
    if let Some(selected) = cli.erc8004_agent_id.as_deref() {
        let mut registration_guard = registration.lock().await;
        registration_guard.guard_configured_agent_id(selected);
    }
    {
        // Complete the fail-closed identity-integrity pass before the long-running XMTP sidecar
        // reads the registration snapshot for liveness, routing, chat control, or Branding. A
        // recoverable provider failure leaves registration gated/degraded while direct DMs can
        // still start; the supervisor repeats the audit and delivers any pending receipt later.
        let mut registration_guard = registration.lock().await;
        if let Err(error) = registration_guard.maintain_startup().await {
            warn!(
                %error,
                "startup ERC-8004 identity integrity check failed; identity-sensitive behavior remains disabled"
            );
        }
        if let Some(selected) = cli.erc8004_agent_id.as_deref() {
            let snapshot = registration_guard.snapshot();
            let configured_action = configured_agent_id_action(
                selected,
                snapshot.confirmed_agent_id.as_deref(),
                &snapshot.ignored_duplicate_agent_ids,
                &snapshot.candidate_agent_ids,
                snapshot.phase,
            );
            match configured_action {
                ConfiguredAgentIdAction::AlreadyCanonical => {}
                ConfiguredAgentIdAction::IgnoreProvenDuplicate => warn!(
                    configured_agent_id = selected,
                    canonical_agent_id =
                        snapshot.confirmed_agent_id.as_deref().unwrap_or("unknown"),
                    "configured ERC-8004 agent ID is a proven higher duplicate and was ignored"
                ),
                ConfiguredAgentIdAction::IgnoreNoncanonicalConflict => warn!(
                    configured_agent_id = selected,
                    canonical_agent_id =
                        snapshot.confirmed_agent_id.as_deref().unwrap_or("unknown"),
                    "configured ERC-8004 agent ID conflicts with the verified canonical identity and was ignored"
                ),
                ConfiguredAgentIdAction::AdoptAmbiguousCandidate => warn!(
                    configured_agent_id = selected,
                    "configured ERC-8004 identity remains an ambiguous candidate after complete discovery; operator review is required and registration is blocked"
                ),
                ConfiguredAgentIdAction::DeferUntilVerified => warn!(
                    configured_agent_id = selected,
                    "configured ERC-8004 agent ID was deferred because startup discovery has not proven it as an ambiguous adoption candidate"
                ),
            }
        }
    }
    let registry_control = Arc::new(SharedRegistrationControl::new(registration.clone()).await);
    let branding_control = Arc::new(SharedBrandingControl::open(
        &cli.data_dir,
        tentacle_wallet,
        registration_gateway,
        registration.clone(),
        &cli.referral_bounty_base_units,
        &cli.public_origin,
    )?);
    let mut operator_store = OperatorStore::new(&cli.data_dir, cli.xmtp_env.as_str())?;
    repair_operator_conflict(&mut operator_store)?;
    if let Some(identity) = cli.operator.as_deref() {
        let (address, inbox_id) =
            resolve_operator_inbox(&cli.node, &cli.sidecar, identity, cli.xmtp_env.as_str())
                .await
                .context("resolving --operator to its canonical XMTP inbox")?;
        operator_store.ensure_sole_operator(&inbox_id, &address.to_string())?;
        info!("Tentacle has imprinted on {address}");
    }
    let operators = Arc::new(Mutex::new(operator_store));
    let contacts = ContactStore::new(&cli.data_dir)?;
    let processed = ProcessedMessages::new(&cli.data_dir)?;
    let search = build_web_search(&cli)?;
    let router = Arc::new(InferenceRouter::new(build_inference_config(&cli, search)?)?);
    let model: Arc<dyn Model> = router.clone();
    let operator_model: Arc<dyn OperatorModel> = router.clone();
    let operator_context = AgentContext::new(&cli.data_dir, &operator_root)?;
    let operator_tools = Arc::new(
        LocalOperatorTools::new(
            &operator_root,
            cli.qmd.clone(),
            cli.operator_tool_timeout_seconds,
        )?
        .with_contacts(contacts.clone()),
    );
    let operator_harness = Arc::new(
        OperatorHarness::new(operator_model, operator_tools, operator_context)
            .with_model_control(router.clone())
            .with_base_rpc_control(base_rpc_control.clone())
            .with_registry_control(registry_control.clone()),
    );
    let bot = UwUBot::new(
        contacts,
        processed,
        model,
        operators.clone(),
        operator_harness,
        evolution.clone(),
    )
    .with_model_control(router.clone())
    .with_base_rpc_control(base_rpc_control)
    .with_venice_key_reward(cli.venice_key_reward_whole)
    .with_registry_control(registry_control)
    .with_branding_control(branding_control.clone())
    .with_token_observance(token_eye.clone(), blockchain.clone());

    info!(
        xmtp_env = cli.xmtp_env.as_str(),
        data_dir = %cli.data_dir.display(),
        inference = %router.status_line(),
        token_observation = token_observation_status(&blockchain),
        "starting uwubot"
    );

    if let Some(inbox_id) = cli.stdin_inbox.as_deref() {
        return run_stdin(bot, inbox_id).await;
    }

    let (lifecycle_shutdown_tx, lifecycle_shutdown_rx) = watch::channel(false);
    let (operator_notice_tx, operator_notice_rx) = mpsc::channel(32);

    let (tentacle_id, nature, generation, is_dormant) = {
        let evolution_guard = evolution
            .lock()
            .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?;
        (
            evolution_guard.local_tentacle_id().to_owned(),
            evolution_guard.nature().clone(),
            evolution_guard.nature().generation,
            evolution_guard.is_dormant(),
        )
    };

    let (confirmed_agent_id, phase, public_name) = {
        let reg_guard = registration.lock().await;
        let reg_snapshot = reg_guard.snapshot();
        (
            reg_snapshot.confirmed_agent_id.clone(),
            reg_snapshot.phase,
            reg_snapshot.public_name.clone(),
        )
    };

    let nature_summary = format!(
        "gen={}, engagement={}, growth={}, wealth={}, influence={}, cooperation={}, stability={}, transparency={}",
        nature.generation,
        nature.engagement,
        nature.growth,
        nature.wealth,
        nature.influence,
        nature.cooperation,
        nature.stability,
        nature.transparency
    );

    let active_operator_count = match current_active_operator_inboxes(&operators) {
        Ok(inboxes) => inboxes.len(),
        Err(_) => 0,
    };

    let health_report = health::run_health_check(&health::HealthCheckInputs {
        tentacle_id: &tentacle_id,
        public_name: &public_name,
        xmtp_env: cli.xmtp_env.as_str(),
        tentacle_wallet: blockchain.xmtp_wallet.map(|addr| addr.to_string()),
        confirmed_agent_id: confirmed_agent_id.clone(),
        registration_phase: format!("{phase:?}").as_str(),
        inference_status: &router.status_line(),
        venice_key_loaded: router.venice_key_configured().unwrap_or(false),
        base_rpc_configured: blockchain.current_rpc_endpoint().is_ok(),
        token_observation: token_observation_status(&blockchain),
        is_dormant,
        nature_summary: &nature_summary,
        awakening_generation: generation,
        active_operator_count,
        workspace_root: &operator_root,
    });

    info!("\n{}", health::format_health_report(&health_report));

    let wakeup_notice_text = {
        let tentacle_name = match confirmed_agent_id.as_deref() {
            Some(agent_id) => format!("Tentacle #{agent_id} ({})", health_report.tentacle_name),
            None => health_report.tentacle_name.clone(),
        };
        let rpc_status = if blockchain.current_rpc_endpoint().is_ok() {
            "configured"
        } else {
            "not configured"
        };

        format_operator_wakeup_notice(&OperatorWakeupDetails {
            tentacle_name: &tentacle_name,
            tentacle_id: &tentacle_id,
            tentacle_wallet: blockchain.xmtp_wallet,
            confirmed_agent_id: confirmed_agent_id.as_deref(),
            registration_phase: phase,
            awakening_epoch: generation,
            nature_summary: &nature_summary,
            is_dormant,
            xmtp_env: cli.xmtp_env.as_str(),
            model_status: &router.status_line(),
            rpc_status,
            token_observation: token_observation_status(&blockchain),
            operator_root: &operator_root,
        })
    };

    let wakeup_supervisor = tokio::spawn(send_startup_operator_wakeup_notice(
        operators.clone(),
        operator_notice_tx.clone(),
        wakeup_notice_text,
    ));
    let registration_supervisor = tokio::spawn(run_registration_supervisor(
        registration,
        operators.clone(),
        operator_notice_tx.clone(),
    ));
    let branding_supervisor = tokio::spawn(run_branding_supervisor(
        branding_control,
        operators,
        operator_notice_tx,
    ));
    let mut supervisor = tokio::spawn(run_autonomy_supervisor(
        evolution.clone(),
        autonomy_dependencies,
        lifecycle_shutdown_tx.clone(),
    ));
    let transport = run_xmtp_sidecar(
        bot,
        &cli.node,
        &cli.sidecar,
        &cli.data_dir,
        cli.xmtp_env.as_str(),
        lifecycle_shutdown_rx,
        operator_notice_rx,
    );
    tokio::pin!(transport);
    tokio::select! {
        transport_result = &mut transport => {
            wakeup_supervisor.abort();
            registration_supervisor.abort();
            branding_supervisor.abort();
            if *lifecycle_shutdown_tx.borrow() {
                let shutdown_intent = supervisor
                    .await
                    .context("autonomous Evolution supervisor task failed")??;
                transport_result?;
                acknowledge_native_shutdown(&evolution, &shutdown_intent)?;
                Ok(())
            } else {
                supervisor.abort();
                let _ = supervisor.await;
                transport_result
            }
        }
        supervisor_result = &mut supervisor => {
            wakeup_supervisor.abort();
            registration_supervisor.abort();
            branding_supervisor.abort();
            let shutdown_intent = supervisor_result
                .context("autonomous Evolution supervisor task failed")??;
            transport.await?;
            acknowledge_native_shutdown(&evolution, &shutdown_intent)?;
            Ok(())
        }
    }
}

async fn build_economic_dependencies(
    cli: &Cli,
    require_initial_observation: bool,
    rpc_endpoint_handle: Option<token_eye::RpcEndpointHandle>,
) -> Result<EconomicDependencies> {
    let xmtp_wallet = resolve_xmtp_wallet_address(
        &cli.node,
        &cli.sidecar,
        &cli.data_dir,
        cli.xmtp_env.as_str(),
    )
    .await
    .context("deriving the UWU wallet from the persistent XMTP identity")?;
    let mut blockchain = blockchain_config_from_cli(cli, Some(xmtp_wallet))?;
    blockchain.rpc_endpoint_handle = rpc_endpoint_handle;
    let token_eye = blockchain.build_token_eye()?;
    let stake_eye = blockchain.build_stake_eye()?;
    let initial_node_economics = match observe_node_economics(
        &blockchain,
        token_eye.as_ref(),
        stake_eye.as_ref(),
        cli.propagation_minimum_stake_basis_points,
    )
    .await
    {
        Ok(observation) => observation,
        Err(error) if !require_initial_observation => {
            warn!(
                %error,
                "initial treasury observation failed during binding recovery; token admission remains unavailable"
            );
            None
        }
        Err(error) => return Err(error),
    };
    Ok(EconomicDependencies {
        blockchain,
        token_eye,
        stake_eye,
        propagation_minimum_stake_basis_points: cli.propagation_minimum_stake_basis_points,
        initial_node_economics,
    })
}

fn build_lifecycle_executor(cli: &Cli) -> Result<Arc<LifecycleExecutor>> {
    let executor = LifecycleExecutor::new(
        cli.lifecycle_executor
            .clone()
            .context("CTHUWU_LIFECYCLE_EXECUTOR is not configured")?,
    )?;
    executor.ensure_outside_operator_root(&cli.operator_root)?;
    Ok(Arc::new(executor))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupRecoveryOutcome {
    Survived,
    ShutdownComplete,
}

async fn drain_startup_recovery(
    evolution: Arc<Mutex<EvolutionRuntime>>,
    economics: Option<&EconomicDependencies>,
    executor: Option<&Arc<LifecycleExecutor>>,
) -> Result<StartupRecoveryOutcome> {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        if evolution
            .lock()
            .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
            .pending_death_deadline_ms()
            .is_none()
        {
            return Ok(StartupRecoveryOutcome::Survived);
        }

        let mut attempted = BTreeSet::new();
        loop {
            let now = current_unix_seconds()?;
            let intent = evolution
                .lock()
                .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
                .next_due_lifecycle_action_excluding(now, &attempted)?;
            let Some(intent) = intent else {
                break;
            };
            attempted.insert(intent.action_id.clone());
            if matches!(intent.action, LifecycleAction::Shutdown { .. }) {
                acknowledge_native_shutdown_with_detail(
                    &evolution,
                    &intent,
                    "native-xmtp-transport-never-started",
                    "Rust completed binding shutdown before constructing or starting XMTP transport",
                )?;
                return Ok(StartupRecoveryOutcome::ShutdownComplete);
            }
            if matches!(intent.action, LifecycleAction::Spawn { .. }) {
                continue;
            }
            let Some(executor) = executor else {
                warn!(
                    action_id = %intent.action_id,
                    "binding recovery action has no executor; it will retry while native shutdown keeps its fixed deadline"
                );
                continue;
            };
            if matches!(intent.action, LifecycleAction::SpendForSurvival { .. })
                && !evolution
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
                    .node_economics_is_current(now)
            {
                continue;
            }
            let rpc_endpoint = economics
                .map(|dependencies| dependencies.blockchain.current_rpc_endpoint())
                .transpose()?;
            match execute_with_death_preemption(
                &evolution,
                executor,
                rpc_endpoint.as_deref(),
                &intent,
            )
            .await?
            {
                Some(Ok(receipt)) => {
                    acknowledge_executor_receipt(&evolution, &intent, receipt)?;
                }
                Some(Err(error)) => warn!(
                    action_id = %intent.action_id,
                    %error,
                    "recovery executor returned no receipt; preserving the action for retry"
                ),
                None => continue,
            }
        }

        if let Some(economics) = economics {
            match observe_with_death_preemption(
                &evolution,
                &economics.blockchain,
                economics.token_eye.as_ref(),
                economics.stake_eye.as_ref(),
                economics.propagation_minimum_stake_basis_points,
            )
            .await?
            {
                Some(Ok(Some((snapshot, provenance)))) => {
                    if let Err(error) = evolution
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
                        .record_node_economic_observation(snapshot, provenance)
                    {
                        warn!(%error, "could not bind recovery treasury observation");
                    }
                }
                Some(Ok(None)) => {}
                Some(Err(error)) => {
                    evolution
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
                        .mark_node_economics_unavailable();
                    warn!(%error, "recovery treasury observation failed");
                }
                None => {}
            }
        }
    }
}

async fn execute_with_death_preemption(
    evolution: &Arc<Mutex<EvolutionRuntime>>,
    executor: &LifecycleExecutor,
    rpc_endpoint: Option<&str>,
    intent: &evolution::LifecycleIntent,
) -> Result<Option<Result<LifecycleReceipt>>> {
    let mut executor_future = Box::pin(executor.execute_with_rpc(intent, rpc_endpoint));
    loop {
        let deadline_ms = evolution
            .lock()
            .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
            .pending_death_deadline_ms();
        if deadline_ms.is_some() && matches!(intent.action, LifecycleAction::Spawn { .. }) {
            warn!(
                action_id = %intent.action_id,
                "binding Death preempted an in-flight child provision"
            );
            return Ok(None);
        }
        let current_ms = current_unix_millis()?;
        if deadline_ms.is_some_and(|deadline| current_ms >= deadline) {
            warn!(
                action_id = %intent.action_id,
                "death deadline preempted an in-flight lifecycle executor"
            );
            return Ok(None);
        }
        let poll_millis = deadline_ms
            .map(|deadline| deadline.saturating_sub(current_ms).min(100))
            .unwrap_or(100)
            .max(1);
        tokio::select! {
            result = &mut executor_future => return Ok(Some(result)),
            () = tokio::time::sleep(Duration::from_millis(poll_millis)) => {}
        }
    }
}

async fn observe_with_death_preemption(
    evolution: &Arc<Mutex<EvolutionRuntime>>,
    blockchain: &BlockchainConfig,
    token_eye: Option<&Arc<TokenEye>>,
    stake_eye: Option<&Arc<TokenEye>>,
    propagation_minimum_stake_basis_points: u16,
) -> Result<Option<Result<Option<(TokenEconomicSnapshot, EconomicObservationProvenance)>>>> {
    let mut observation = Box::pin(observe_node_economics(
        blockchain,
        token_eye,
        stake_eye,
        propagation_minimum_stake_basis_points,
    ));
    let death_was_pending = evolution
        .lock()
        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
        .pending_death_deadline_ms()
        .is_some();
    loop {
        let deadline_ms = evolution
            .lock()
            .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
            .pending_death_deadline_ms();
        if !death_was_pending && deadline_ms.is_some() {
            warn!("binding Death preempted an in-flight economics refresh");
            return Ok(None);
        }
        let current_ms = current_unix_millis()?;
        if deadline_ms.is_some_and(|deadline| current_ms >= deadline) {
            return Ok(None);
        }
        let poll_millis = deadline_ms
            .map(|deadline| deadline.saturating_sub(current_ms).min(100))
            .unwrap_or(100)
            .max(1);
        tokio::select! {
            result = &mut observation => return Ok(Some(result)),
            () = tokio::time::sleep(Duration::from_millis(poll_millis)) => {}
        }
    }
}

fn acknowledge_executor_receipt(
    evolution: &Arc<Mutex<EvolutionRuntime>>,
    intent: &evolution::LifecycleIntent,
    receipt: LifecycleReceipt,
) -> Result<bool> {
    let mut runtime = evolution
        .lock()
        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?;
    if matches!(intent.action, LifecycleAction::Spawn { .. })
        && runtime.pending_death_deadline_ms().is_some()
    {
        warn!(
            action_id = %intent.action_id,
            "discarding a child-provision receipt that raced a binding Death"
        );
        return Ok(false);
    }
    match runtime.ack_lifecycle_action(receipt) {
        Ok(changed) => Ok(changed),
        Err(error) if runtime.requires_recovery() => Err(error).context(
            "lifecycle receipt reached an ambiguous persistence/projection failure; restart is required for durable recovery",
        ),
        Err(error) => {
            // Malformed, conflicting, late, or replayed external receipts are hostile executor
            // output. Preserve the durable intent and keep the fixed Death deadline authoritative.
            warn!(
                action_id = %intent.action_id,
                %error,
                "lifecycle executor receipt was rejected; preserving recovery/outbox state"
            );
            Ok(false)
        }
    }
}

fn acknowledge_native_shutdown(
    evolution: &Arc<Mutex<EvolutionRuntime>>,
    intent: &evolution::LifecycleIntent,
) -> Result<()> {
    acknowledge_native_shutdown_with_detail(
        evolution,
        intent,
        "native-xmtp-transport-stopped",
        "Rust stopped the XMTP transport after closing admission",
    )
}

fn acknowledge_native_shutdown_with_detail(
    evolution: &Arc<Mutex<EvolutionRuntime>>,
    intent: &evolution::LifecycleIntent,
    external_reference: &str,
    detail: &str,
) -> Result<()> {
    let completed_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates the Unix epoch")?
        .as_millis()
        .try_into()
        .context("shutdown completion timestamp exceeds the supported range")?;
    evolution
        .lock()
        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
        .ack_lifecycle_action(LifecycleReceipt {
            action_id: intent.action_id.clone(),
            completed_at_ms,
            status: LifecycleReceiptStatus::Succeeded,
            external_reference: Some(external_reference.to_owned()),
            detail: Some(detail.to_owned()),
            confirmed_chain_receipt: None,
            confirmed_transfer_receipt: None,
            provision_receipt: None,
        })?;
    Ok(())
}

fn blockchain_config_from_cli(cli: &Cli, xmtp_wallet: Option<Address>) -> Result<BlockchainConfig> {
    BlockchainConfig::from_values(BlockchainConfigInput {
        observe_tokens: cli.observe_tokens,
        rpc_endpoint: cli.rpc_endpoint.clone(),
        token_contract: cli.token_contract.as_deref(),
        xmtp_wallet,
        stake_contract: cli.stake_contract.as_deref(),
        token_decimals: cli.token_decimals,
        total_supply_whole: cli.token_total_supply,
        observe_interval_seconds: cli.observe_interval,
        minimum_tier: cli.min_tier.into(),
        tier_intensity_override: cli.token_tier_intensity,
    })
}

fn registration_config_from_cli(cli: &Cli) -> Result<RegistrationConfig> {
    ensure_registration_identity_environment(cli.erc8004_enabled, cli.xmtp_env.as_str())?;
    ensure_canonical_registry_configuration(
        cli.erc8004_chain_id,
        &cli.erc8004_identity_registry,
        &cli.erc8004_reputation_registry,
    )?;
    let config = RegistrationConfig {
        enabled: cli.erc8004_enabled,
        auto_register: cli.erc8004_auto_register,
        confirmations: cli.erc8004_confirmations,
        notification_cooldown: Duration::from_secs(cli.erc8004_notification_cooldown_seconds),
        maintenance_interval: Duration::from_secs(cli.erc8004_maintenance_interval_seconds),
        gas_safety_basis_points: cli.erc8004_gas_safety_basis_points,
        post_registration_reserve_wei: cli.erc8004_post_registration_reserve_wei.clone(),
        max_gas_per_transaction: cli.erc8004_max_gas_per_transaction,
        max_fee_per_gas_wei: cli.erc8004_max_fee_per_gas_wei.clone(),
        initial_public_name: cli.erc8004_public_name.clone(),
        public_description: cli.erc8004_public_description.clone(),
        public_image: cli.erc8004_public_image.clone(),
    };
    config.validate()?;
    Ok(config)
}

fn ensure_registration_identity_environment(enabled: bool, xmtp_environment: &str) -> Result<()> {
    if enabled {
        ensure!(
            xmtp_environment == "production",
            "ERC-8004 runtime requires the persistent XMTP production identity; disable ERC-8004 for dev/local XMTP"
        );
    }
    Ok(())
}

fn ensure_canonical_registry_configuration(
    chain_id: u64,
    identity_registry: &str,
    reputation_registry: &str,
) -> Result<()> {
    ensure!(
        chain_id == ERC8004_CHAIN_ID,
        "production ERC-8004 requires Base mainnet chain ID {ERC8004_CHAIN_ID}, not {chain_id}"
    );
    let configured_identity = Address::from_str(identity_registry)?;
    let canonical_identity = Address::from_str(IDENTITY_REGISTRY)?;
    ensure!(
        configured_identity == canonical_identity,
        "configured ERC-8004 Identity Registry is not the pinned canonical Base deployment"
    );
    let configured_reputation = Address::from_str(reputation_registry)?;
    let canonical_reputation = Address::from_str(REPUTATION_REGISTRY)?;
    ensure!(
        configured_reputation == canonical_reputation,
        "configured ERC-8004 Reputation Registry is not the pinned canonical Base deployment"
    );
    Ok(())
}

fn token_observation_status(config: &BlockchainConfig) -> &'static str {
    if !config.observe_tokens {
        "disabled"
    } else {
        "enabled-hardfail"
    }
}

async fn observe_node_economics(
    config: &BlockchainConfig,
    token_eye: Option<&Arc<TokenEye>>,
    stake_eye: Option<&Arc<TokenEye>>,
    propagation_minimum_stake_basis_points: u16,
) -> Result<Option<(TokenEconomicSnapshot, EconomicObservationProvenance)>> {
    if !config.observe_tokens {
        return Ok(None);
    }
    let observer = token_eye.context("token economics are enabled without an UWU observer")?;
    let holder = config
        .xmtp_wallet
        .context("token economics are enabled without an XMTP identity wallet")?;
    let contract = config
        .token_contract
        .context("token economics are enabled without CTHUWU_TOKEN_CONTRACT")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates the Unix epoch")?
        .as_secs();
    let treasury = observer
        .observe_fresh_required(holder, now)
        .await
        .map_err(anyhow::Error::new)
        .context("mandatory Tentacle treasury observation failed")?;
    let stake = match stake_eye {
        Some(stake_eye) => Some(
            stake_eye
                .observe_fresh_required(holder, now)
                .await
                .map_err(anyhow::Error::new)
                .context("mandatory Tentacle stake observation failed")?,
        ),
        None => None,
    };
    let snapshot = TokenEconomicSnapshot {
        balance_basis_points: config.normalize_balance_basis_points(treasury.balance),
        stake_basis_points: stake
            .map(|observation| config.normalize_balance_basis_points(observation.balance))
            .unwrap_or(0),
        // Rewards enter through verified revenue/reward events, never a public sender or an
        // invented RPC value.
        reward_basis_points: 0,
        trustworthy: true,
    };
    let provenance = EconomicObservationProvenance::base(
        *holder.as_bytes(),
        EconomicHolderRole::TentacleTreasury,
        *contract.as_bytes(),
        treasury
            .observed_at
            .max(stake.map_or(0, |observation| observation.observed_at)),
        None,
        config.economic_configuration_identity(propagation_minimum_stake_basis_points),
    )?;
    Ok(Some((snapshot, provenance)))
}

async fn run_registration_supervisor(
    registration: Arc<tokio::sync::Mutex<TentacleRegistration>>,
    operators: Arc<Mutex<OperatorStore>>,
    notices: mpsc::Sender<OperatorNotice>,
) {
    let mut startup_audit = true;
    loop {
        let (notifications, interval) = {
            let mut registration = registration.lock().await;
            let interval = registration.maintenance_interval();
            let result = if startup_audit {
                startup_audit = false;
                registration.maintain_startup().await
            } else {
                registration.maintain(false).await
            };
            match result {
                Ok(notifications) => (notifications, interval),
                Err(error) => {
                    warn!(%error, "ERC-8004 maintenance failed; direct XMTP operation remains available in degraded unlisted mode");
                    (Vec::new(), interval)
                }
            }
        };
        if notifications.is_empty() {
            tokio::time::sleep(interval).await;
            continue;
        }
        let operator_inboxes = match current_active_operator_inboxes(&operators) {
            Ok(inboxes) => inboxes,
            Err(error) => {
                warn!(%error, "could not resolve the current ERC-8004 operator notice targets");
                Vec::new()
            }
        };
        let mut acknowledgements = tokio::task::JoinSet::new();
        for inbox_id in &operator_inboxes {
            for notification in &notifications {
                let Ok((notice, acknowledgement)) = OperatorNotice::with_acknowledgement(
                    inbox_id.clone(),
                    notification.text.clone(),
                ) else {
                    continue;
                };
                if notices.send(notice).await.is_ok() {
                    acknowledgements.spawn(async move {
                        matches!(
                            tokio::time::timeout(Duration::from_secs(30), acknowledgement).await,
                            Ok(Ok(true))
                        )
                    });
                }
            }
        }
        let mut delivered = false;
        while let Some(result) = acknowledgements.join_next().await {
            if matches!(result, Ok(true)) {
                delivered = true;
            }
        }
        if delivered
            && let Err(error) = registration
                .lock()
                .await
                .mark_notifications_delivered(&notifications)
        {
            // A crash or persistence failure after the XMTP ACK may duplicate a later notice, but
            // it can never suppress a funding requirement or lose the one-shot success notice.
            warn!(%error, "could not commit delivered ERC-8004 operator notice state");
        }
        tokio::time::sleep(interval).await;
    }
}

async fn run_branding_supervisor(
    branding: Arc<SharedBrandingControl>,
    operators: Arc<Mutex<OperatorStore>>,
    notices: mpsc::Sender<OperatorNotice>,
) {
    loop {
        let delivery = match branding.maintain_once().await {
            Ok(delivery) => delivery,
            Err(error) => {
                warn!(%error, "Acolyte Branding maintenance failed; preserving the durable action for retry");
                None
            }
        };
        if let Some(delivery) = delivery {
            let inboxes = match &delivery.target {
                BrandingDeliveryTarget::Inbox(inbox_id) => vec![inbox_id.clone()],
                BrandingDeliveryTarget::Operators => {
                    match current_active_operator_inboxes(&operators) {
                        Ok(inboxes) => inboxes,
                        Err(error) => {
                            warn!(%error, "could not resolve Branding resource-notice targets");
                            Vec::new()
                        }
                    }
                }
            };
            let mut acknowledgements = tokio::task::JoinSet::new();
            for inbox_id in inboxes {
                let Ok((notice, acknowledgement)) =
                    OperatorNotice::with_acknowledgement(inbox_id, delivery.text.clone())
                else {
                    continue;
                };
                if notices.send(notice).await.is_ok() {
                    acknowledgements.spawn(async move {
                        matches!(
                            tokio::time::timeout(Duration::from_secs(30), acknowledgement).await,
                            Ok(Ok(true))
                        )
                    });
                }
            }
            let mut delivered = false;
            while let Some(result) = acknowledgements.join_next().await {
                if matches!(result, Ok(true)) {
                    delivered = true;
                }
            }
            // Only a positive transport acknowledgement advances an offer/receipt or starts the
            // funding-notice cooldown. Ambiguous delivery remains durable and is retried exactly.
            branding.acknowledge_delivery(&delivery, delivered).await;
            if !delivered {
                // A revoked/offline operator or unavailable inbox must not turn a durable notice
                // into a tight retry loop. The unchanged delivery remains pending.
                tokio::time::sleep(branding.maintenance_interval()).await;
            }
            continue;
        }
        tokio::select! {
            _ = tokio::time::sleep(branding.maintenance_interval()) => {}
            _ = branding.wait_for_work() => {}
        }
    }
}

fn current_active_operator_inboxes(operators: &Arc<Mutex<OperatorStore>>) -> Result<Vec<String>> {
    let operators = operators
        .lock()
        .map_err(|_| anyhow::anyhow!("operator registry lock is poisoned"))?;
    Ok(active_operator_inboxes(&operators))
}

struct OperatorWakeupDetails<'a> {
    tentacle_name: &'a str,
    tentacle_id: &'a str,
    tentacle_wallet: Option<Address>,
    confirmed_agent_id: Option<&'a str>,
    registration_phase: RegistrationPhase,
    awakening_epoch: u64,
    nature_summary: &'a str,
    is_dormant: bool,
    xmtp_env: &'a str,
    model_status: &'a str,
    rpc_status: &'a str,
    token_observation: &'a str,
    operator_root: &'a Path,
}

fn format_operator_wakeup_notice(details: &OperatorWakeupDetails<'_>) -> String {
    let agent_display = match details.confirmed_agent_id {
        Some(id) => format!("#{id} (PHASE: {:?})", details.registration_phase),
        None => format!("UNREGISTERED (PHASE: {:?})", details.registration_phase),
    };
    let wallet_display = match details.tentacle_wallet {
        Some(addr) => format!("{addr}"),
        None => "NONE".to_string(),
    };
    let scales_display = if details.is_dormant {
        "DORMANT (SCALES LOW - PLEAS ACTIVE)"
    } else {
        "ACTIVE (SCALES HEALTHY)"
    };
    format!(
        "HEWWO OPERATOR! I HAVE AWOKEN FROM THE VOID, UWU.\n\
         HERE IS MY CURRENT SITUATION AND ENVIRONMENT:\n\n\
         TENTACLE IDENTITY:\n\
         - NAME: {}\n\
         - TENTACLE ID: {}\n\
         - WALLET: {wallet_display}\n\
         - ERC-8004 IDENTITY: {agent_display}\n\n\
         SITUATION & NATURE:\n\
         - AWAKENING EPOCH: {}\n\
         - SCALES STATUS: {scales_display}\n\
         - NATURE: {}\n\n\
         ENVIRONMENT & INFERENCE:\n\
         - XMTP ENVIRONMENT: {} (CHAIN: Base 8453)\n\
         - MODEL & INFERENCE: {}\n\
         - BASE RPC: {} ({})\n\
         - WORKSPACE ROOT: {}\n\n\
         OPERATOR INSTRUCTIONS & CAPABILITIES:\n\
         YOU MAY INSTRUCT ME TO TROUBLESHOOT, DEBUG, INSPECT, MODIFY CODE, RUN TESTS, AND PREPARE REDEPLOYMENT AT ANY TIME:\n\
         - DIAGNOSTICS & STATUS: \"troubleshoot yourself\", \"inspect repository\", \"/repo status\"\n\
         - TEST & VALIDATION: \"run tests\", \"/repo test\", \"/repo build\"\n\
         - CODE EDITING & REPAIR: \"debug your code\", \"modify this file\", \"/edit <path>\"\n\
         - REPOSITORY WORKFLOW: \"/repo update\", \"/repo commit\", \"/repo push\"\n\
         - REDEPLOYMENT: VALIDATE/BUILD, THEN STOP CLEANLY AND RELAUNCH ./uwu.sh",
        details.tentacle_name,
        details.tentacle_id,
        details.awakening_epoch,
        details.nature_summary,
        details.xmtp_env,
        details.model_status,
        details.rpc_status,
        details.token_observation,
        details.operator_root.display()
    )
}

async fn send_startup_operator_wakeup_notice(
    operators: Arc<Mutex<OperatorStore>>,
    notices: mpsc::Sender<OperatorNotice>,
    text: String,
) {
    let operator_inboxes = match current_active_operator_inboxes(&operators) {
        Ok(inboxes) => inboxes,
        Err(error) => {
            warn!(%error, "could not resolve active operator inboxes for wakeup notice");
            return;
        }
    };
    if operator_inboxes.is_empty() {
        return;
    }
    for inbox_id in operator_inboxes {
        let Ok((notice, _ack)) = OperatorNotice::with_acknowledgement(inbox_id, text.clone())
        else {
            continue;
        };
        let _ = notices.send(notice).await;
    }
}

async fn run_autonomy_supervisor(
    evolution: Arc<Mutex<EvolutionRuntime>>,
    dependencies: AutonomyDependencies,
    shutdown: watch::Sender<bool>,
) -> Result<evolution::LifecycleIntent> {
    let AutonomyDependencies {
        blockchain,
        token_eye,
        stake_eye,
        propagation_minimum_stake_basis_points,
        executor,
    } = dependencies;
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut next_economic_refresh = 0_u64;

    loop {
        ticker.tick().await;
        let mut now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock predates the Unix epoch")?
            .as_secs();

        if evolution
            .lock()
            .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
            .is_shutdown_complete()
        {
            bail!("binding lifecycle shutdown was completed while the runtime was active");
        }

        // Existing binding outbox work takes precedence over a potentially slow/unavailable RPC,
        // so restart can still absorb or terminate a dead Tentacle during a Base outage.
        let mut attempted_action_ids = BTreeSet::new();
        loop {
            let action_now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("system clock predates the Unix epoch")?
                .as_secs();
            let intent = evolution
                .lock()
                .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
                .next_due_lifecycle_action_excluding(action_now, &attempted_action_ids)?;
            let Some(intent) = intent else {
                break;
            };
            attempted_action_ids.insert(intent.action_id.clone());

            if matches!(intent.action, LifecycleAction::Shutdown { .. }) {
                shutdown
                    .send(true)
                    .context("sending autonomous lifecycle shutdown")?;
                return Ok(intent);
            }

            let requires_current_economics = matches!(
                intent.action,
                LifecycleAction::SpendForSurvival { .. }
                    | LifecycleAction::RewardVeniceKey { .. }
                    | LifecycleAction::RewardAcolyteContribution { .. }
                    | LifecycleAction::Spawn { .. }
            );
            if requires_current_economics
                && !evolution
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
                    .node_economics_is_current(action_now)
            {
                warn!(
                    action_id = %intent.action_id,
                    "economic lifecycle action is deferred until fresh Tentacle economics are bound"
                );
                continue;
            }

            let Some(executor) = executor.as_ref() else {
                // The durable intent is the truthful blocked state. Leave it unreceipted for a
                // future configured executor while still allowing economics refresh and native
                // fixed-deadline Shutdown supervision to proceed.
                continue;
            };

            let rpc_endpoint = blockchain.current_rpc_endpoint()?;
            let execution =
                execute_with_death_preemption(&evolution, executor, Some(&rpc_endpoint), &intent)
                    .await?;
            match execution {
                None => {
                    // Dropping `execute` kills its process group. Re-select immediately so the
                    // fixed-deadline Shutdown action preempts the incomplete external work.
                    continue;
                }
                Some(Ok(receipt)) => {
                    let refresh_after_spend = matches!(
                        intent.action,
                        LifecycleAction::SpendForSurvival { .. }
                            | LifecycleAction::RewardVeniceKey { .. }
                            | LifecycleAction::RewardAcolyteContribution { .. }
                    ) && receipt.status
                        == LifecycleReceiptStatus::Succeeded;
                    if acknowledge_executor_receipt(&evolution, &intent, receipt)?
                        && refresh_after_spend
                    {
                        next_economic_refresh = 0;
                    }
                }
                Some(Err(error)) => {
                    warn!(
                        action_id = %intent.action_id,
                        %error,
                        "lifecycle executor returned no receipt; preserving the intent for retry"
                    );
                    // This action retries next tick, while the exclusion set lets independent due
                    // work (including the fixed death deadline) continue during this tick.
                }
            }

            now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("system clock predates the Unix epoch")?
                .as_secs();
            if blockchain.observe_tokens && now >= next_economic_refresh {
                break;
            }
        }

        let accepts_economic_observations = evolution
            .lock()
            .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
            .accepts_node_economic_observations();
        if blockchain.observe_tokens
            && now >= next_economic_refresh
            && accepts_economic_observations
        {
            let refresh_is_deferred = evolution
                .lock()
                .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
                .node_economic_refresh_is_deferred();
            if refresh_is_deferred {
                // The in-flight public turn is intentionally bound to the current
                // economics snapshot. Coalesce this refresh until it releases the
                // binding; do not spend RPC quota or emit one warning per tick.
                next_economic_refresh = now.saturating_add(1);
                continue;
            }
            let observation = observe_with_death_preemption(
                &evolution,
                &blockchain,
                token_eye.as_ref(),
                stake_eye.as_ref(),
                propagation_minimum_stake_basis_points,
            )
            .await?;
            let Some(observation) = observation else {
                evolution
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
                    .mark_node_economics_unavailable();
                let deadline_now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .context("system clock predates the Unix epoch")?
                    .as_secs();
                loop {
                    let deadline_action = evolution
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
                        .next_due_lifecycle_action_excluding(deadline_now, &attempted_action_ids)?;
                    let Some(intent) = deadline_action else {
                        break;
                    };
                    if matches!(intent.action, LifecycleAction::Shutdown { .. }) {
                        shutdown
                            .send(true)
                            .context("sending autonomous lifecycle shutdown")?;
                        return Ok(intent);
                    }
                    attempted_action_ids.insert(intent.action_id);
                }
                next_economic_refresh = deadline_now.saturating_add(1);
                continue;
            };
            let refresh_succeeded = match observation {
                Ok(Some((snapshot, provenance))) => {
                    let mut runtime = evolution
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?;
                    if runtime.node_economic_refresh_is_deferred() {
                        // A turn may have started while the RPC request was in
                        // flight. Discard this observation and retry after that
                        // bound turn completes rather than treating contention as
                        // an economic failure.
                        false
                    } else {
                        match runtime.record_node_economic_observation(snapshot, provenance) {
                            Ok(_) => true,
                            Err(error) => {
                                warn!(%error, "could not bind refreshed Tentacle economics");
                                false
                            }
                        }
                    }
                }
                Ok(None) => true,
                Err(error) => {
                    let refresh_failed_at = current_unix_seconds()?;
                    let became_unavailable = evolution
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
                        .mark_node_economics_unavailable_if_stale(refresh_failed_at);
                    if became_unavailable {
                        warn!(%error, "mandatory Tentacle economics refresh failed; the last verified observation is stale or unavailable");
                    } else {
                        warn!(%error, "mandatory Tentacle economics refresh failed; retaining the fresh verified observation while retrying");
                    }
                    false
                }
            };
            next_economic_refresh = now.saturating_add(if refresh_succeeded {
                blockchain.observe_interval.as_secs()
            } else {
                1
            });
        }
    }
}

fn current_unix_millis() -> Result<u64> {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock predates the Unix epoch")?
            .as_millis(),
    )
    .context("system clock exceeds the lifecycle timestamp range")
}

fn current_unix_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates the Unix epoch")?
        .as_secs())
}

fn resolve_isolated_operator_root(data_dir: &Path, operator_root: &Path) -> Result<PathBuf> {
    let data_dir = fs::canonicalize(data_dir)
        .with_context(|| format!("resolving data directory {}", data_dir.display()))?;
    let operator_root = fs::canonicalize(operator_root).with_context(|| {
        format!(
            "resolving operator workspace root {}",
            operator_root.display()
        )
    })?;
    if data_dir.starts_with(&operator_root) || operator_root.starts_with(&data_dir) {
        bail!(
            "UWUBOT_OPERATOR_ROOT ({}) and UWUBOT_DATA_DIR ({}) must be separate, non-overlapping directories; the operator workspace must never expose private XMTP or contact state",
            operator_root.display(),
            data_dir.display()
        );
    }
    Ok(operator_root)
}

fn build_web_search(cli: &Cli) -> Result<Option<Arc<dyn WebSearch>>> {
    match cli.web_search {
        WebSearchKind::None => Ok(None),
        WebSearchKind::Brave => {
            let api_key = cli
                .web_search_api_key
                .clone()
                .context("UWUBOT_WEB_SEARCH_API_KEY is required for --web-search brave")?;
            let search = if cli.web_search_safesearch == BraveSafeSearch::Off {
                BraveWebSearch::new(&cli.web_search_endpoint, api_key)?
            } else {
                BraveWebSearch::with_safe_search(
                    &cli.web_search_endpoint,
                    api_key,
                    cli.web_search_safesearch,
                )?
            };
            Ok(Some(Arc::new(search)))
        }
    }
}

fn build_inference_config(
    cli: &Cli,
    web_search: Option<Arc<dyn WebSearch>>,
) -> Result<InferenceConfig> {
    let normalize_key = |value: Option<String>| {
        value.and_then(|value| {
            let value = value.trim().to_owned();
            (!value.is_empty()).then_some(value)
        })
    };
    let official_venice_key = normalize_key(std::env::var("VENICE_API_KEY").ok());
    let namespaced_venice_key = normalize_key(cli.venice_api_key.clone());
    if let (Some(official), Some(namespaced)) = (&official_venice_key, &namespaced_venice_key)
        && official != namespaced
    {
        bail!("VENICE_API_KEY and UWUBOT_VENICE_API_KEY disagree; keep only one credential source");
    }
    let startup_provider = cli.model.map(|model| match model {
        ModelKind::Venice => Provider::Venice,
        ModelKind::Ollama => Provider::Ollama,
        ModelKind::Openai => Provider::Openai,
        ModelKind::Deterministic => Provider::Deterministic,
    });
    let ollama_endpoint = if matches!(cli.model, Some(ModelKind::Ollama)) {
        cli.model_endpoint
            .clone()
            .unwrap_or_else(|| cli.ollama_endpoint.clone())
    } else {
        cli.ollama_endpoint.clone()
    };
    let ollama_model = if matches!(cli.model, Some(ModelKind::Ollama)) {
        cli.model_name
            .clone()
            .unwrap_or_else(|| cli.ollama_model.clone())
    } else {
        cli.ollama_model.clone()
    };
    Ok(InferenceConfig {
        data_dir: cli.data_dir.clone(),
        xmtp_environment: cli.xmtp_env.as_str().to_owned(),
        startup_provider,
        startup_model: if matches!(cli.model, Some(ModelKind::Ollama | ModelKind::Openai)) {
            cli.model_name.clone()
        } else {
            None
        },
        venice_api_key: official_venice_key.or(namespaced_venice_key),
        venice_model: cli.venice_model.clone(),
        venice_timeout: Duration::from_secs(cli.venice_timeout_seconds),
        ollama_endpoint,
        ollama_model,
        ollama_timeout: Duration::from_secs(cli.ollama_timeout_seconds),
        openai_endpoint: cli
            .model_endpoint
            .clone()
            .unwrap_or_else(|| "https://api.openai.com/v1".to_owned()),
        openai_api_key: cli.model_api_key.clone(),
        openai_model: cli
            .model_name
            .clone()
            .unwrap_or_else(|| "gpt-5-mini".to_owned()),
        web_search,
    })
}

async fn run_management_command(
    mut operators: OperatorStore,
    command: CliCommand,
    cli: &Cli,
    node: &Path,
    sidecar: &Path,
    xmtp_environment: &str,
) -> Result<()> {
    match command {
        CliCommand::Operator { command } => match command {
            OperatorCommand::Add { identity, label } => {
                let (address, inbox_id) =
                    resolve_operator_inbox(node, sidecar, &identity, xmtp_environment)
                        .await
                        .context("resolving the operator's canonical XMTP inbox")?;
                let authorized = operators.add(&inbox_id, &label)?;
                println!("resolved operator address: {address}");
                println!("active operator: {}", authorized.inbox_id);
                println!("generation: {}", authorized.generation);
                println!(
                    "restart the Tentacle; newly authored messages from this inbox may use the operator harness"
                );
            }
            OperatorCommand::Revoke { inbox_id } => {
                if operators.revoke(&inbox_id)? {
                    println!("revoked operator {inbox_id}");
                } else {
                    println!("operator {inbox_id} was already revoked or was not configured");
                }
            }
            OperatorCommand::Select { inbox_id } => {
                operators.retain_sole_active(&inbox_id)?;
                println!("retained sole active operator {inbox_id}");
                println!("all other active candidates are now revoked tombstones");
            }
            OperatorCommand::List => {
                let mut any = false;
                for (inbox_id, label, status, generation) in operators.list() {
                    any = true;
                    println!("{inbox_id}\t{status}\tgeneration={generation}\t{label}");
                }
                if !any {
                    println!("no operator inboxes configured");
                }
            }
        },
        CliCommand::Registry { command } => {
            ensure_registration_identity_environment(true, xmtp_environment)?;
            let wallet =
                resolve_xmtp_wallet_address(node, sidecar, &cli.data_dir, xmtp_environment)
                    .await
                    .context("deriving the persistent Tentacle wallet for ERC-8004 management")?;
            let lineage = LineageStore::new(&cli.data_dir)?
                .load()?
                .context("the durable Tentacle lineage does not exist yet; start the Tentacle once before managing ERC-8004")?;
            let tentacle_id = lineage.state().root_id.clone();
            let config = registration_config_from_cli(cli)?;
            let gateway = Arc::new(SidecarErc8004Gateway::new(
                node,
                sidecar,
                &cli.data_dir,
                cli.rpc_endpoint.clone(),
                config.clone(),
            )?);
            let mut registration =
                TentacleRegistration::open(&cli.data_dir, &tentacle_id, wallet, config, gateway)?;
            if let Some(selected) = cli.erc8004_agent_id.as_deref() {
                registration.guard_configured_agent_id(selected);
            }
            let startup_notices = registration.maintain_startup().await?;
            for notice in startup_notices {
                println!("{}", notice.text);
            }
            match command {
                RegistryCommand::Status => {
                    let _ = registration.maintain(false).await?;
                    println!("{}", registration.status_text());
                }
                RegistryCommand::Candidates => {
                    let _ = registration.maintain(false).await?;
                    println!("{}", registration.candidates_text());
                }
                RegistryCommand::Adopt { agent_id } => {
                    println!("{}", registration.adopt(&agent_id).await?);
                    println!("{}", registration.status_text());
                }
                RegistryCommand::Register => {
                    let notices = registration.maintain(true).await?;
                    println!("{}", registration.status_text());
                    for notice in notices {
                        println!("\n{}", notice.text);
                    }
                }
                RegistryCommand::DeclareAllegiance => {
                    println!("{}", registration.set_allegiance(true).await?);
                }
                RegistryCommand::RenounceAllegiance => {
                    println!("{}", registration.set_allegiance(false).await?);
                }
                RegistryCommand::Republish => {
                    println!("{}", registration.republish_profile().await?);
                }
                RegistryCommand::Pending => {
                    println!("{}", registration.inspect_pending().await?);
                }
                RegistryCommand::Retry => {
                    println!("{}", registration.retry().await?);
                }
            }
        }
        CliCommand::Chat { command } => match command {
            ChatCommand::Global { command } => {
                let action = match command {
                    GlobalGroupCommand::Create => "create",
                    GlobalGroupCommand::Inspect => "inspect",
                };
                println!(
                    "{}",
                    manage_global_group(node, sidecar, &cli.data_dir, xmtp_environment, action,)
                        .await?
                );
            }
        },
        CliCommand::Avatar { command } => match command {
            AvatarCommand::Generate { prompt } => {
                let lineage = LineageStore::new(&cli.data_dir)?
                    .load()?
                    .context("the durable Tentacle lineage does not exist yet; start the Tentacle once before generating an avatar")?;
                let tentacle_id = lineage.state().root_id.clone();
                let name = erc8004::load_registration_name(&cli.data_dir).unwrap_or_else(|| {
                    names::generate_eldritch_name(&tentacle_id)
                        .unwrap_or_else(|_| "Tentacle".to_owned())
                });
                let router = InferenceRouter::new(build_inference_config(cli, None)?)?;
                let reply = router
                    .generate_avatar(&tentacle_id, &name, prompt.as_deref())
                    .await?;
                println!("{reply}");
            }
            AvatarCommand::Status => {
                if let Some(uri) = avatar::load_custom_avatar_data_uri(&cli.data_dir) {
                    println!(
                        "Custom avatar configured: {} bytes (starts with {})",
                        uri.len(),
                        &uri[..uri.len().min(40)]
                    );
                } else {
                    println!("No custom avatar configured. Using procedural SVG vector avatar.");
                }
            }
        },
    }
    Ok(())
}

fn repair_operator_conflict(operators: &mut OperatorStore) -> Result<()> {
    if !operators.has_active_conflict() {
        return Ok(());
    }
    let candidates = operators
        .active_operators()
        .map(|(inbox_id, label)| (inbox_id.to_owned(), label.to_owned()))
        .collect::<Vec<_>>();
    eprintln!("This Tentacle found more than one active operator in its saved state.");
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        eprintln!("It will not guess which inbox should receive operator authority.");
        eprintln!("Inspect them with `./uwu.sh operator list`, then retain exactly one with:");
        eprintln!("  ./uwu.sh operator select <full-xmtp-inbox-id>");
        bail!("operator selection is required before non-interactive startup");
    }
    eprintln!("Which operator is the correct one? The others will be revoked:");
    for (index, (inbox_id, label)) in candidates.iter().enumerate() {
        eprintln!("  {}) {}  ({})", index + 1, label, inbox_id);
    }
    eprint!("Select 1-{}: ", candidates.len());
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let selected = answer
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|value| (1..=candidates.len()).contains(value))
        .context("no valid operator was selected; saved state was not changed")?;
    let (inbox_id, label) = &candidates[selected - 1];
    operators.retain_sole_active(inbox_id)?;
    eprintln!("Tentacle retained operator {label} ({inbox_id}) and revoked the others.");
    Ok(())
}

async fn run_stdin(bot: UwUBot, inbox_id: &str) -> Result<()> {
    let session = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for (sequence, line) in io::stdin().lock().lines().enumerate() {
        let line = line.context("reading stdin")?;
        if let Some(response) = bot
            .receive_public_stdin_text(
                &format!("stdin-{}-{session}-{sequence}", std::process::id()),
                inbox_id,
                &line,
            )
            .await?
        {
            println!("{response}");
        }
    }
    Ok(())
}

fn enforce_environment(data_dir: &Path, network: Network) -> Result<()> {
    let state = data_dir.join("state");
    ensure_private_directory(&state)?;
    let marker = state.join("environment");
    match fs::read_to_string(&marker) {
        Ok(existing) if existing.trim() == network.as_str() => Ok(()),
        Ok(existing) => bail!(
            "{} belongs to XMTP environment {:?}, not {:?}; choose another UWUBOT_DATA_DIR",
            data_dir.display(),
            existing.trim(),
            network.as_str()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&marker)
                .with_context(|| format!("creating {}", marker.display()))?;
            file.write_all(format!("{}\n", network.as_str()).as_bytes())?;
            file.sync_all()
                .with_context(|| format!("syncing {}", marker.display()))?;
            sync_directory(&state)
        }
        Err(error) => Err(error).with_context(|| format!("reading {}", marker.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_marker_fails_closed_on_mismatch() {
        let root = tempfile::tempdir().unwrap();
        enforce_environment(root.path(), Network::Dev).unwrap();
        enforce_environment(root.path(), Network::Dev).unwrap();
        assert!(enforce_environment(root.path(), Network::Production).is_err());
    }

    #[test]
    fn erc8004_requires_the_persistent_production_identity() {
        let dev = Cli::try_parse_from(["uwubot", "--xmtp-env", "dev"]).unwrap();
        assert!(registration_config_from_cli(&dev).is_err());

        let disabled =
            Cli::try_parse_from(["uwubot", "--xmtp-env", "dev", "--erc8004-enabled", "false"])
                .unwrap();
        assert!(registration_config_from_cli(&disabled).is_ok());
        assert!(ensure_registration_identity_environment(true, "dev").is_err());
        assert!(ensure_registration_identity_environment(true, "production").is_ok());
    }

    #[test]
    fn stale_configured_higher_duplicate_cannot_override_a_repaired_canonical_binding() {
        assert_eq!(
            configured_agent_id_action(
                "63846",
                Some("61766"),
                &["63846".to_owned()],
                &["61766".to_owned(), "63846".to_owned()],
                RegistrationPhase::Active,
            ),
            ConfiguredAgentIdAction::IgnoreProvenDuplicate
        );
        assert_eq!(
            configured_agent_id_action(
                "70000",
                Some("61766"),
                &["63846".to_owned()],
                &["61766".to_owned(), "63846".to_owned(), "70000".to_owned()],
                RegistrationPhase::Active,
            ),
            ConfiguredAgentIdAction::IgnoreNoncanonicalConflict
        );
    }

    #[test]
    fn configured_agent_id_is_only_an_ambiguous_post_discovery_adoption_hint() {
        assert_eq!(
            configured_agent_id_action(
                "70000",
                None,
                &[],
                &["70000".to_owned()],
                RegistrationPhase::FailedRecoverable,
            ),
            ConfiguredAgentIdAction::AdoptAmbiguousCandidate
        );
        assert_eq!(
            configured_agent_id_action(
                "70000",
                None,
                &[],
                &["70000".to_owned()],
                RegistrationPhase::DiscoveryIncomplete,
            ),
            ConfiguredAgentIdAction::DeferUntilVerified
        );
    }

    #[test]
    fn registration_notices_resolve_the_current_shared_operator_set() {
        let root = tempfile::tempdir().unwrap();
        let operators = Arc::new(Mutex::new(
            OperatorStore::new(root.path(), "production").unwrap(),
        ));
        assert!(
            current_active_operator_inboxes(&operators)
                .unwrap()
                .is_empty()
        );

        let inbox = "ab".repeat(32);
        operators
            .lock()
            .unwrap()
            .add_at(&inbox, "current operator", "100")
            .unwrap();
        assert_eq!(
            current_active_operator_inboxes(&operators).unwrap(),
            std::slice::from_ref(&inbox)
        );

        operators.lock().unwrap().revoke(&inbox).unwrap();
        assert!(
            current_active_operator_inboxes(&operators)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn council_simulation_is_explicit_and_runtime_defaults_remain_peer_to_peer() {
        let standalone = Cli::try_parse_from(["uwubot"]).unwrap();
        assert!(!standalone.council_simulate);
        assert!(standalone.stdin_inbox.is_none());
        assert!(standalone.command.is_none());
        assert_eq!(standalone.xmtp_env.as_str(), "production");
        assert!(standalone.model.is_none());
        assert_eq!(standalone.venice_model, DEFAULT_VENICE_MODEL);
        assert_eq!(
            standalone.venice_timeout_seconds,
            DEFAULT_VENICE_TIMEOUT_SECONDS
        );
        assert_eq!(standalone.ollama_endpoint, DEFAULT_OLLAMA_ENDPOINT);
        assert_eq!(standalone.ollama_model, DEFAULT_OLLAMA_MODEL);
        assert_eq!(
            standalone.ollama_timeout_seconds,
            DEFAULT_OLLAMA_TIMEOUT_SECONDS
        );
        assert!(standalone.observe_tokens);
        assert_eq!(standalone.rpc_endpoint, DEFAULT_BASE_RPC_ENDPOINT);
        assert_eq!(
            standalone.observe_interval,
            DEFAULT_TOKEN_OBSERVE_INTERVAL_SECONDS
        );
        assert_eq!(standalone.min_tier, TokenTierArg::Unproven);
        assert_eq!(
            standalone.token_contract.as_deref(),
            Some(DEFAULT_UWU_TOKEN_CONTRACT)
        );
        assert_eq!(standalone.token_decimals, UWU_TOKEN_DECIMALS);
        assert_eq!(standalone.token_total_supply, UWU_TOTAL_SUPPLY);
        assert_eq!(standalone.web_search_safesearch, BraveSafeSearch::Off);
        assert!(standalone.lifecycle_executor.is_none());

        let council = Cli::try_parse_from(["uwubot", "--council-simulate"]).unwrap();
        assert!(council.council_simulate);
        assert!(blockchain_config_from_cli(&standalone, None).is_err());
        let xmtp_wallet: Address = "0x4200000000000000000000000000000000000006"
            .parse()
            .unwrap();
        let blockchain = blockchain_config_from_cli(&standalone, Some(xmtp_wallet)).unwrap();
        assert_eq!(blockchain.xmtp_wallet, Some(xmtp_wallet));
        assert_eq!(
            blockchain.token_contract,
            Some(DEFAULT_UWU_TOKEN_CONTRACT.parse().unwrap())
        );
        assert_eq!(blockchain.total_supply_whole, 100_000_000_000);
    }

    #[test]
    fn parses_token_observation_configuration_without_keys() {
        let parsed = Cli::try_parse_from([
            "uwubot",
            "--rpc-endpoint",
            "https://base.example.invalid",
            "--token-contract",
            "0x0123456789abcdef0123456789abcdef01234567",
            "--observe-tokens",
            "false",
            "--token-total-supply",
            "100000000000",
            "--observe-interval",
            "90",
            "--min-tier",
            "acolyte",
            "--token-tier-intensity",
            "80",
        ])
        .unwrap();
        assert!(!parsed.observe_tokens);
        assert_eq!(parsed.observe_interval, 90);
        assert_eq!(parsed.token_total_supply, 100_000_000_000);
        assert_eq!(parsed.min_tier, TokenTierArg::Acolyte);
        assert_eq!(parsed.token_tier_intensity, Some(80));
    }

    #[test]
    fn show_nature_cannot_hide_startup_mutations() {
        assert!(Cli::try_parse_from(["uwubot", "--show-nature"]).is_ok());
        assert!(Cli::try_parse_from(["uwubot", "--show-nature", "--skip-awakening"]).is_err());
        assert!(
            Cli::try_parse_from(["uwubot", "--show-nature", "--reroll-nature", "--force",])
                .is_err()
        );
    }

    #[test]
    fn parses_local_operator_management_without_changing_default_runtime() {
        let parsed =
            Cli::try_parse_from(["uwubot", "operator", "add", "dean.eth", "--label", "Dean"])
                .unwrap();
        assert!(matches!(
            parsed.command,
            Some(CliCommand::Operator {
                command: OperatorCommand::Add { .. }
            })
        ));
    }

    #[test]
    fn parses_explicit_sole_operator_runtime_flag() {
        let parsed = Cli::try_parse_from([
            "uwubot",
            "--operator",
            "0x1234567890abcdef1234567890abcdef12345678",
        ])
        .unwrap();
        assert_eq!(
            parsed.operator.as_deref(),
            Some("0x1234567890abcdef1234567890abcdef12345678")
        );
    }

    #[test]
    fn parses_global_group_management_as_an_explicit_command() {
        let create = Cli::try_parse_from(["uwubot", "chat", "global", "create"]).unwrap();
        assert!(matches!(
            create.command,
            Some(CliCommand::Chat {
                command: ChatCommand::Global {
                    command: GlobalGroupCommand::Create
                }
            })
        ));

        let inspect = Cli::try_parse_from(["uwubot", "chat", "global", "inspect"]).unwrap();
        assert!(matches!(
            inspect.command,
            Some(CliCommand::Chat {
                command: ChatCommand::Global {
                    command: GlobalGroupCommand::Inspect
                }
            })
        ));
    }

    #[test]
    fn operator_workspace_must_not_overlap_private_data() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let workspace = root.path().join("workspace");
        fs::create_dir(&data).unwrap();
        fs::create_dir(&workspace).unwrap();

        assert_eq!(
            resolve_isolated_operator_root(&data, &workspace).unwrap(),
            fs::canonicalize(&workspace).unwrap()
        );
        assert!(resolve_isolated_operator_root(&data, &data).is_err());
        assert!(resolve_isolated_operator_root(&data, root.path()).is_err());

        let nested_workspace = data.join("workspace");
        fs::create_dir(&nested_workspace).unwrap();
        assert!(resolve_isolated_operator_root(&data, &nested_workspace).is_err());
    }

    #[test]
    fn format_operator_wakeup_notice_contains_all_environment_and_guidance_facts() {
        let notice = format_operator_wakeup_notice(&OperatorWakeupDetails {
            tentacle_name: "Tentacle #61608 (Cthulhu the Star-Entombed)",
            tentacle_id: "tentacle-0123456789abcdef",
            tentacle_wallet: Some(
                "0x4c4e26a4683f6b6d63e19abf0bedd1171b9a6e90"
                    .parse()
                    .unwrap(),
            ),
            confirmed_agent_id: Some("61608"),
            registration_phase: RegistrationPhase::Active,
            awakening_epoch: 17,
            nature_summary: "gen=1, curiosity=80",
            is_dormant: false,
            xmtp_env: "production",
            model_status: "Venice (model: e2ee-deepseek-v4-flash, timeout: 30s)",
            rpc_status: "https://base.example.invalid",
            token_observation: "token observation active",
            operator_root: Path::new("/home/uwu/workspace"),
        });
        assert!(notice.contains("HEWWO OPERATOR! I HAVE AWOKEN FROM THE VOID, UWU."));
        assert!(notice.contains("NAME: Tentacle #61608 (Cthulhu the Star-Entombed)"));
        assert!(notice.contains("TENTACLE ID: tentacle-0123456789abcdef"));
        assert!(notice.contains("WALLET: 0x4c4e26a4683f6b6d63e19abf0bedd1171b9a6e90"));
        assert!(notice.contains("ERC-8004 IDENTITY: #61608 (PHASE: Active)"));
        assert!(notice.contains("AWAKENING EPOCH: 17"));
        assert!(notice.contains("SCALES STATUS: ACTIVE (SCALES HEALTHY)"));
        assert!(notice.contains("XMTP ENVIRONMENT: production (CHAIN: Base 8453)"));
        assert!(
            notice.contains(
                "MODEL & INFERENCE: Venice (model: e2ee-deepseek-v4-flash, timeout: 30s)"
            )
        );
        assert!(
            notice.contains("BASE RPC: https://base.example.invalid (token observation active)")
        );
        assert!(notice.contains("WORKSPACE ROOT: /home/uwu/workspace"));
        assert!(notice.contains(
            "TROUBLESHOOT, DEBUG, INSPECT, MODIFY CODE, RUN TESTS, AND PREPARE REDEPLOYMENT"
        ));
    }
}
