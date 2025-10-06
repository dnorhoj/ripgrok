use std::path::PathBuf;

use ::anyhow::{Context, bail};
use ::clap::Parser;

use crate::utils::Config;
use crate::utils::config::{GeneralConfig, ServerConfig, TlsConfig};

#[derive(Parser, Debug)]
pub struct InitCommand {
    /// Server host
    #[arg()]
    pub host: String,
    /// Server port
    #[arg(default_value = "7267")]
    pub port: u16,
    /// Overwrite config if it already exists
    #[arg(long, short)]
    pub overwrite: bool,
    /// Authentication token
    #[arg(long, short)]
    pub token: Option<String>,
    /// Use unencrypted connection
    #[arg(long)]
    pub no_tls: bool,
}

impl InitCommand {
    pub async fn run(self, config_path: PathBuf) -> anyhow::Result<()> {
        if !self.overwrite && config_path.exists() {
            bail!("Config path already exists - use --overwite to overwrite config")
        }

        let new_config = Config {
            general: Some(GeneralConfig { token: self.token }),
            server: ServerConfig {
                host: self.host,
                port: self.port,
            },
            tls: if self.no_tls {
                None
            } else {
                Some(TlsConfig::default())
            },
        };

        new_config
            .save(&config_path)
            .await
            .context("Failed to save config")?;

        tracing::info!("New config saved at {}!", config_path.to_str().unwrap());

        Ok(())
    }
}
