use std::path::PathBuf;

use ::config::{Config as ConfigLoader, File};
use ::serde::{Deserialize, Serialize};
use ::tokio::fs::{OpenOptions, create_dir_all};
use ::tokio::io::AsyncWriteExt;

#[derive(Deserialize, Serialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Deserialize, Serialize, Default)]
pub struct TlsConfig {
    pub cert_b64: Option<String>,
    pub cert_file: Option<String>,
}

#[derive(Deserialize, Serialize, Default)]
pub struct GeneralConfig {
    pub token: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct Config {
    pub server: ServerConfig,
    pub tls: Option<TlsConfig>,
    pub general: Option<GeneralConfig>,
}

impl Config {
    pub fn load(config_path: PathBuf) -> anyhow::Result<Self> {
        Ok(ConfigLoader::builder()
            .set_default("server.port", 7267)?
            .add_source(File::from(config_path))
            .build()?
            .try_deserialize()?)
    }

    pub async fn save(&self, config_path: &PathBuf) -> anyhow::Result<()> {
        if let Some(config_dir) = config_path.parent() {
            create_dir_all(config_dir).await?;
        }

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(config_path)
            .await?;

        file.write(toml::to_string_pretty(&self)?.as_bytes())
            .await?;

        Ok(())
    }
}
