use ::anyhow::Context;
use ::clap::Parser;
use ::thiserror::Error;
use ::tracing_subscriber::FmtSubscriber;
use ripgrok::tunnel_specifier::TunnelSpecifier;

use crate::{
    config::{Config, ServerConfig},
    control_connection::ControlConnection,
};

pub mod config;
mod control_connection;

#[derive(Parser)]
struct CliArguments {
    #[arg(long)]
    pub server_host: Option<String>,
    #[arg(long)]
    pub server_port: Option<u16>,
    #[arg(required = true)]
    pub tunnel_specifiers: Vec<String>,
}

#[derive(Error, Debug)]
enum CliError {
    #[error("Missing server details. Either provide through cli args or configuration file.")]
    MissingConnectionDetails,
}

fn get_connection_addr(
    config: &ServerConfig,
    cli_args: &CliArguments,
) -> Result<(String, u16), CliError> {
    Ok((
        cli_args
            .server_host
            .clone()
            .or_else(|| config.host.clone())
            .ok_or(CliError::MissingConnectionDetails)?,
        cli_args.server_port.unwrap_or(config.port),
    ))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing::subscriber::set_global_default(FmtSubscriber::default())?;

    let cli_args = CliArguments::parse();
    let config = Config::load()?;

    let addr = get_connection_addr(&config.server, &cli_args)?;

    tracing::info!("Connecting to server at tcp://{}:{}", addr.0, addr.1);

    let tunnel_specifiers: Vec<TunnelSpecifier> = cli_args
        .tunnel_specifiers
        .iter()
        .map(|s| s.parse())
        .collect::<Result<_, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to parse tunnel specifier: {}", e))?;

    let connection = ControlConnection::init(addr, tunnel_specifiers)
        .await
        .context("Failed to connect to server")?;

    connection.run().await?;

    Ok(())
}
