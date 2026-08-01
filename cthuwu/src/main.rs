use anyhow::{bail, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "cthuwu", version, about)]
struct Cli {
    #[arg(long, env = "CTHUWU_DATA_DIR", default_value = ".cthuwu")]
    data_dir: PathBuf,

    #[arg(long, env = "CTHUWU_XMTP_ENV", value_enum, default_value_t = Network::Dev)]
    xmtp_env: Network,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Network {
    Dev,
    Production,
    Local,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize a dedicated Cthuwu identity and encrypted data directory.
    Init,
    /// Listen for XMTP messages and reply as Cthuwu.
    Serve {
        /// Model adapter. The echo adapter is safe for transport testing.
        #[arg(long, default_value = "echo")]
        model: String,
    },
    /// Print non-secret configuration and identity health.
    Status,
    /// Check storage, XMTP, and model connectivity.
    Doctor,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "cthuwu=info".into()))
        .without_time()
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Init => not_implemented("identity initialization"),
        Command::Serve { model } => {
            info!(?cli.xmtp_env, ?cli.data_dir, %model, "starting cthuwu");
            not_implemented("XMTP transport")
        }
        Command::Status => {
            println!(
                "data_dir={} xmtp_env={:?} transport=not-configured",
                cli.data_dir.display(),
                cli.xmtp_env
            );
            Ok(())
        }
        Command::Doctor => not_implemented("runtime diagnostics"),
    }
}

fn not_implemented(feature: &str) -> Result<()> {
    bail!("{feature} is not implemented yet; see ARCHITECTURE.md")
}
