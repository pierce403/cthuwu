mod agent_context;
mod autonomy;
pub mod awakening;
mod bot;
mod config;
mod contact;
mod deadline;
mod dedupe;
pub mod economics;
pub mod evolution;
pub mod evolution_runtime;
pub mod hermes;
mod inference;
mod matching;
mod model;
mod operator;
pub mod personality;
mod principal;
pub mod scales;
mod sidecar;
mod storage;
pub mod token_eye;
pub mod token_gov;
mod web_search;

use agent_context::AgentContext;
use anyhow::{Context, Result, bail};
use autonomy::LifecycleExecutor;
use bot::UwUBot;
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
use evolution::{LifecycleAction, LifecycleReceipt, LifecycleReceiptStatus};
use evolution_runtime::{EvolutionRuntime, EvolutionStartupOptions, MandatoryRecoveryKind};
use inference::{
    DEFAULT_OLLAMA_ENDPOINT, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_TIMEOUT_SECONDS,
    DEFAULT_VENICE_MODEL, DEFAULT_VENICE_TIMEOUT_SECONDS, InferenceConfig, InferenceRouter,
    Provider,
};
use model::Model;
use operator::{LocalOperatorTools, OperatorHarness, OperatorModel};
use principal::OperatorStore;
use sidecar::{resolve_operator_inbox, resolve_xmtp_wallet_address, run_xmtp_sidecar};
use std::{
    collections::BTreeSet,
    env,
    fs::{self, OpenOptions},
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use storage::{ensure_private_directory, sync_directory};
use token_eye::{Address, ReputationTier, TokenEye};
use tokio::sync::watch;
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

    /// Death-to-shutdown grace period. Production default is 24 hours.
    #[arg(
        long,
        env = "CTHUWU_DEATH_GRACE_SECONDS",
        default_value_t = 24 * 60 * 60
    )]
    death_grace_seconds: u64,

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
        default_value_t = 120
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "uwubot=info".into()))
        .without_time()
        .init();

    let mut cli = Cli::parse();
    let starts_normal_runtime = cli.command.is_none() && !cli.council_simulate && !cli.show_nature;
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
        if mandatory_recovery == MandatoryRecoveryKind::None && cli.death_grace_seconds == 0 {
            bail!("CTHUWU_DEATH_GRACE_SECONDS must be positive (default: 86400)");
        }
        if mandatory_recovery == MandatoryRecoveryKind::None
            && cli.propagation_minimum_stake_basis_points > 10_000
        {
            bail!("CTHUWU_PROPAGATION_MINIMUM_STAKE_BPS must not exceed 10000");
        }
        if !matches!(
            mandatory_recovery,
            MandatoryRecoveryKind::CompletedShutdown
                | MandatoryRecoveryKind::AbsorptionProjectionRequired
        ) {
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
        let operators = OperatorStore::new(&cli.data_dir, cli.xmtp_env.as_str())?;
        return run_management_command(
            operators,
            command,
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
        death_grace_period_seconds: if recovering_binding_death && cli.death_grace_seconds == 0 {
            24 * 60 * 60
        } else {
            cli.death_grace_seconds
        },
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
    let operators = OperatorStore::new(&cli.data_dir, cli.xmtp_env.as_str())?;
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
            .with_model_control(router.clone()),
    );
    let bot = UwUBot::new(
        contacts,
        processed,
        model,
        Arc::new(Mutex::new(operators)),
        operator_harness,
        evolution.clone(),
    )
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
    );
    tokio::pin!(transport);
    tokio::select! {
        transport_result = &mut transport => {
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
) -> Result<EconomicDependencies> {
    let xmtp_wallet = resolve_xmtp_wallet_address(
        &cli.node,
        &cli.sidecar,
        &cli.data_dir,
        cli.xmtp_env.as_str(),
    )
    .await
    .context("deriving the UWU wallet from the persistent XMTP identity")?;
    let blockchain = blockchain_config_from_cli(cli, Some(xmtp_wallet))?;
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
            match execute_with_death_preemption(
                &evolution,
                executor,
                economics.map(|dependencies| dependencies.blockchain.rpc_endpoint.as_str()),
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
                LifecycleAction::SpendForSurvival { .. } | LifecycleAction::Spawn { .. }
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

            let execution = execute_with_death_preemption(
                &evolution,
                executor,
                Some(&blockchain.rpc_endpoint),
                &intent,
            )
            .await?;
            match execution {
                None => {
                    // Dropping `execute` kills its process group. Re-select immediately so the
                    // fixed-deadline Shutdown action preempts the incomplete external work.
                    continue;
                }
                Some(Ok(receipt)) => {
                    let refresh_after_spend =
                        matches!(intent.action, LifecycleAction::SpendForSurvival { .. })
                            && receipt.status == LifecycleReceiptStatus::Succeeded;
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
                    match evolution
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Evolution runtime lock is poisoned"))?
                        .record_node_economic_observation(snapshot, provenance)
                    {
                        Ok(_) => true,
                        Err(error) => {
                            warn!(%error, "could not bind refreshed Tentacle economics");
                            false
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
    }
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
}
