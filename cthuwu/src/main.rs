mod bot;
mod contact;
mod dedupe;
mod matching;
mod model;
mod sidecar;
mod storage;

use anyhow::{Context, Result, bail};
use bot::UwUBot;
use clap::{Parser, ValueEnum};
use contact::ContactStore;
use cthuwu_council::run_deterministic_simulation;
use dedupe::ProcessedMessages;
use model::{DeterministicModel, Model, OpenAiCompatibleModel};
use sidecar::run_xmtp_sidecar;
use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use storage::{ensure_private_directory, sync_directory};
use tracing::info;
use tracing_subscriber::EnvFilter;

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

    #[arg(long, env = "UWUBOT_MODEL", value_enum, default_value_t = ModelKind::Deterministic)]
    model: ModelKind,

    #[arg(long, env = "UWUBOT_MODEL_ENDPOINT")]
    model_endpoint: Option<String>,

    #[arg(long, env = "UWUBOT_MODEL_NAME")]
    model_name: Option<String>,

    #[arg(long, env = "UWUBOT_MODEL_API_KEY", hide = true)]
    model_api_key: Option<String>,

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
    Deterministic,
    Ollama,
    Openai,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "uwubot=info".into()))
        .without_time()
        .init();

    let cli = Cli::parse();
    enforce_environment(&cli.data_dir, cli.xmtp_env)?;
    if cli.council_simulate {
        let report = run_deterministic_simulation(&cli.data_dir)
            .context("running deterministic local Council simulation")?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let contacts = ContactStore::new(&cli.data_dir)?;
    let processed = ProcessedMessages::new(&cli.data_dir)?;
    let model = build_model(&cli)?;
    let bot = UwUBot::new(contacts, processed, model);

    info!(
        xmtp_env = cli.xmtp_env.as_str(),
        data_dir = %cli.data_dir.display(),
        model = ?cli.model,
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

fn build_model(cli: &Cli) -> Result<Arc<dyn Model>> {
    match cli.model {
        ModelKind::Deterministic => Ok(Arc::new(DeterministicModel)),
        ModelKind::Ollama => Ok(Arc::new(OpenAiCompatibleModel::new(
            cli.model_endpoint
                .as_deref()
                .unwrap_or("http://127.0.0.1:11434/v1"),
            None,
            cli.model_name.as_deref().unwrap_or("qwen3:8b"),
        )?)),
        ModelKind::Openai => {
            let api_key = cli
                .model_api_key
                .clone()
                .context("UWUBOT_MODEL_API_KEY is required for --model openai")?;
            Ok(Arc::new(OpenAiCompatibleModel::new(
                cli.model_endpoint
                    .as_deref()
                    .unwrap_or("https://api.openai.com/v1"),
                Some(api_key),
                cli.model_name.as_deref().unwrap_or("gpt-5-mini"),
            )?))
        }
    }
}

async fn run_stdin(bot: UwUBot, inbox_id: &str) -> Result<()> {
    let session = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for (sequence, line) in io::stdin().lock().lines().enumerate() {
        let line = line.context("reading stdin")?;
        if let Some(response) = bot
            .receive_text(
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
        assert!(matches!(standalone.model, ModelKind::Deterministic));

        let council = Cli::try_parse_from(["uwubot", "--council-simulate"]).unwrap();
        assert!(council.council_simulate);
    }
}
