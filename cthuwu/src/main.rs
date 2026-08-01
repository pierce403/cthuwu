mod bot;
mod contact;

use anyhow::{bail, Context, Result};
use bot::UwUBot;
use clap::{Parser, ValueEnum};
use contact::ContactStore;
use std::{
    io::{self, BufRead},
    path::PathBuf,
};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "uwubot", version, about = "Run Cthuwu's one-to-one XMTP companion")]
struct Cli {
    #[arg(long, env = "UWUBOT_DATA_DIR", default_value = ".")]
    data_dir: PathBuf,

    #[arg(long, env = "UWUBOT_XMTP_ENV", value_enum, default_value_t = Network::Dev)]
    xmtp_env: Network,

    /// Development harness: receive newline-delimited messages for one inbox on stdin.
    #[arg(long, hide = true)]
    stdin_inbox: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Network {
    Dev,
    Production,
    Local,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "uwubot=info".into()))
        .without_time()
        .init();

    let cli = Cli::parse();
    let contacts = ContactStore::new(&cli.data_dir)?;
    let bot = UwUBot::new(contacts);

    info!(?cli.xmtp_env, data_dir = %cli.data_dir.display(), "starting uwubot");

    if let Some(inbox_id) = cli.stdin_inbox {
        return run_stdin(bot, &inbox_id);
    }

    bail!(
        "the contact engine is ready, but the native libxmtp transport is not wired yet;          use --stdin-inbox <hex-inbox-id> for the local harness"
    )
}

fn run_stdin(bot: UwUBot, inbox_id: &str) -> Result<()> {
    for line in io::stdin().lock().lines() {
        let line = line.context("reading stdin")?;
        println!("{}", bot.receive_text(inbox_id, &line)?);
    }
    Ok(())
}
