use std::path::PathBuf;

use ::anyhow::anyhow;
use ::clap::{Parser, Subcommand};
use ::tracing_subscriber::FmtSubscriber;

use crate::commands::init::InitCommand;
use crate::commands::run::RunCommand;

pub mod commands;
pub mod control_connection;
pub mod utils;

#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Where to get configuration from - defaults to $XDG_CONFIG_HOME/ripgrok/config.toml
    #[arg(short, long, default_value = None)]
    pub config_path: Option<PathBuf>,

    #[command(subcommand)]
    pub cmd: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run ripgrok
    Run(RunCommand),
    /// Set up ripgrok
    Init(InitCommand),
}

impl Commands {
    pub async fn run(self, config_path: PathBuf) -> anyhow::Result<()> {
        match self {
            Commands::Run(command) => command.run(config_path).await,
            Commands::Init(command) => command.run(config_path).await,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing::subscriber::set_global_default(FmtSubscriber::default())?;

    let cli = Cli::parse();

    let config_path = cli
        .config_path
        .or_else(|| dirs::config_dir().map(|dir| dir.join("ripgrok/config.toml")))
        .ok_or(anyhow!("Could not find config location - please specify your config with the -c/--config option."))?;

    cli.cmd.run(config_path).await
}
