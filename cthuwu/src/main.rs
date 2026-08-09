mod agent_context;
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
use bot::UwUBot;
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use config::{
    BlockchainConfig, BlockchainConfigInput, DEFAULT_BASE_RPC_ENDPOINT,
    DEFAULT_TOKEN_OBSERVE_INTERVAL_SECONDS, REQUESTED_UWU_TOTAL_SUPPLY, UWU_TOKEN_DECIMALS,
};
use contact::ContactStore;
use cthuwu_council::run_deterministic_simulation;
use dedupe::ProcessedMessages;
use evolution_runtime::{EvolutionRuntime, EvolutionStartupOptions};
use inference::{
    DEFAULT_OLLAMA_ENDPOINT, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_TIMEOUT_SECONDS,
    DEFAULT_VENICE_MODEL, DEFAULT_VENICE_TIMEOUT_SECONDS, InferenceConfig, InferenceRouter,
    Provider,
};
use model::Model;
use operator::{LocalOperatorTools, OperatorHarness, OperatorModel};
use principal::OperatorStore;
use sidecar::run_xmtp_sidecar;
use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use storage::{ensure_private_directory, sync_directory};
use token_eye::ReputationTier;
use tracing::info;
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

    #[arg(long, env = "UWUBOT_XMTP_ENV", value_enum, default_value_t = Network::Dev)]
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

    /// Base JSON-RPC endpoint used only for read-only ERC-20 observations.
    #[arg(
        long,
        env = "CTHUWU_RPC_ENDPOINT",
        default_value = DEFAULT_BASE_RPC_ENDPOINT,
        hide_env_values = true
    )]
    rpc_endpoint: String,

    /// Deployed UWU ERC-20 address. Omit before launch; ordinary interaction remains available.
    #[arg(long, env = "CTHUWU_TOKEN_CONTRACT")]
    token_contract: Option<String>,

    /// ERC-20 decimals used to normalize raw balances (Clanker standard: 18).
    #[arg(long, env = "CTHUWU_TOKEN_DECIMALS", default_value_t = UWU_TOKEN_DECIMALS)]
    token_decimals: u8,

    /// Whole-token supply used for local normalization (requested UWU default: 1 billion).
    #[arg(
        long,
        env = "CTHUWU_TOKEN_TOTAL_SUPPLY",
        default_value_t = REQUESTED_UWU_TOTAL_SUPPLY
    )]
    token_total_supply: u64,

    /// Enable local token observation. Use `--observe-tokens false` to disable it.
    #[arg(
        long,
        env = "CTHUWU_OBSERVE_TOKENS",
        default_value_t = true,
        action = ArgAction::Set
    )]
    observe_tokens: bool,

    /// Cache lifetime for ordinary balance observations. Economic admission uses a fresh read.
    #[arg(
        long,
        env = "CTHUWU_OBSERVE_INTERVAL",
        default_value_t = DEFAULT_TOKEN_OBSERVE_INTERVAL_SECONDS
    )]
    observe_interval: u64,

    /// Minimum known UWU tier for public interaction. Unknown RPC state degrades neutrally.
    #[arg(long, env = "CTHUWU_MIN_TIER", value_enum, default_value_t = TokenTierArg::Unproven)]
    min_tier: TokenTierArg,

    /// Override Nature-derived tier differentiation (0 ignores tiers; 100 uses full effects).
    #[arg(long, env = "CTHUWU_TOKEN_TIER_INTENSITY")]
    token_tier_intensity: Option<u8>,

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
    /// Authorize an operator inbox immediately using a local timestamp fence.
    Add {
        /// Full XMTP inbox ID. Wallet addresses, prefixes, and display names are not accepted.
        inbox_id: String,
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "uwubot=info".into()))
        .without_time()
        .init();

    let mut cli = Cli::parse();
    enforce_environment(&cli.data_dir, cli.xmtp_env)?;
    cli.data_dir = fs::canonicalize(&cli.data_dir)
        .with_context(|| format!("resolving data directory {}", cli.data_dir.display()))?;
    let operators = OperatorStore::new(&cli.data_dir, cli.xmtp_env.as_str())?;
    if let Some(command) = cli.command.take() {
        return run_management_command(operators, command);
    }
    if cli.council_simulate {
        let report = run_deterministic_simulation(&cli.data_dir)
            .context("running deterministic local Council simulation")?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let operator_root = resolve_isolated_operator_root(&cli.data_dir, &cli.operator_root)?;
    let evolution = EvolutionRuntime::open(
        &cli.data_dir,
        &operator_root,
        EvolutionStartupOptions {
            skip_awakening: cli.skip_awakening,
            reroll_nature: cli.reroll_nature,
            force: cli.force,
            nature_path: cli.nature_path.clone(),
            gossip_peers: cli.gossip_peers.clone(),
        },
    )?;
    if cli.show_nature {
        println!("{}", evolution.nature_status());
        return Ok(());
    }
    let contacts = ContactStore::new(&cli.data_dir)?;
    let processed = ProcessedMessages::new(&cli.data_dir)?;
    let blockchain = BlockchainConfig::from_values(BlockchainConfigInput {
        observe_tokens: cli.observe_tokens,
        rpc_endpoint: cli.rpc_endpoint.clone(),
        token_contract: cli.token_contract.as_deref(),
        token_decimals: cli.token_decimals,
        total_supply_whole: cli.token_total_supply,
        observe_interval_seconds: cli.observe_interval,
        minimum_tier: cli.min_tier.into(),
        tier_intensity_override: cli.token_tier_intensity,
    })?;
    let token_eye = blockchain.build_token_eye()?;
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
        Arc::new(Mutex::new(evolution)),
    )
    .with_token_observance(token_eye, blockchain.clone());

    info!(
        xmtp_env = cli.xmtp_env.as_str(),
        data_dir = %cli.data_dir.display(),
        inference = %router.status_line(),
        token_observation = token_observation_status(&blockchain),
        "starting uwubot"
    );

    if let Some(inbox_id) = cli.stdin_inbox {
        return run_stdin(bot, &inbox_id).await;
    }

    run_xmtp_sidecar(
        bot,
        &cli.node,
        &cli.sidecar,
        &cli.data_dir,
        cli.xmtp_env.as_str(),
    )
    .await
}

fn token_observation_status(config: &BlockchainConfig) -> &'static str {
    if !config.observe_tokens {
        "disabled"
    } else if config.token_contract.is_none() {
        "waiting-for-contract"
    } else {
        "enabled"
    }
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

fn run_management_command(mut operators: OperatorStore, command: CliCommand) -> Result<()> {
    match command {
        CliCommand::Operator { command } => match command {
            OperatorCommand::Add { inbox_id, label } => {
                let authorized = operators.add(&inbox_id, &label)?;
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
    fn council_mode_is_opt_in_and_preserves_standalone_defaults() {
        let standalone = Cli::try_parse_from(["uwubot"]).unwrap();
        assert!(!standalone.council_simulate);
        assert!(standalone.stdin_inbox.is_none());
        assert!(standalone.command.is_none());
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
        assert!(standalone.token_contract.is_none());
        assert_eq!(standalone.token_decimals, UWU_TOKEN_DECIMALS);
        assert_eq!(standalone.token_total_supply, REQUESTED_UWU_TOTAL_SUPPLY);
        assert_eq!(standalone.web_search_safesearch, BraveSafeSearch::Off);

        let council = Cli::try_parse_from(["uwubot", "--council-simulate"]).unwrap();
        assert!(council.council_simulate);
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
        let parsed = Cli::try_parse_from([
            "uwubot",
            "operator",
            "add",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--label",
            "Dean",
        ])
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
