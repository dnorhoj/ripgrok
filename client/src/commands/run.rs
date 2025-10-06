use std::path::PathBuf;
use std::sync::Arc;

use ::anyhow::{Context, bail};
use ::clap::Parser;
use ripgrok_common::tunnel_specifier::TunnelSpecifier;

use crate::{
    control_connection::{
        ConnectionInitializer, ControlConnection, TcpInitializer, TlsInitializer,
    },
    utils::Config,
};

#[derive(Parser, Debug)]
pub struct RunCommand {
    /// Which tunnels to create
    #[arg(required = true)]
    pub tunnel_specifiers: Vec<String>,
}

impl RunCommand {
    pub async fn run(&self, config_path: PathBuf) -> anyhow::Result<()> {
        if !config_path.exists() {
            bail!(
                "Config file not found! Get started by running `{} init`",
                std::env::args().next().unwrap(),
            );
        }

        let config = Config::load(config_path).context("Could not parse configuration")?;

        let addr = (config.server.host, config.server.port);

        let tunnel_specifiers: Vec<TunnelSpecifier> = self
            .tunnel_specifiers
            .iter()
            .map(|s| s.parse())
            .collect::<Result<_, _>>()
            .map_err(|e| anyhow::anyhow!("Failed to parse tunnel specifier: {}", e))?;

        let initializer: Arc<dyn ConnectionInitializer> = if let Some(tls_config) = config.tls {
            Arc::new(TlsInitializer::new(addr.clone(), &tls_config)?)
        } else {
            tracing::warn!("Using unencrypted connection - consider enabling TLS");
            Arc::new(TcpInitializer::new(addr.clone()))
        };

        tracing::info!(
            "Connecting to server at {}://{}:{}",
            initializer.protocol_identifier(),
            addr.0,
            addr.1
        );

        let connection = ControlConnection::init(
            initializer,
            tunnel_specifiers,
            config.general.map_or(None, |g| g.token),
        )
        .await
        .context("Failed to connect to server")?;

        connection.run().await?;

        Ok(())
    }
}
