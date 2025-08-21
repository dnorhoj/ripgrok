use ::config::{Config as ConfigLoader, File};
use ::serde::Deserialize;

#[derive(Deserialize)]
pub struct ServerConfig {
    pub host: Option<String>,
    pub port: u16,
}

#[derive(Deserialize)]
pub struct Config {
    pub server: ServerConfig,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        Ok(ConfigLoader::builder()
            .set_default("server.port", 7267)?
            .add_source(File::with_name("config.toml").required(false))
            .build()?
            .try_deserialize()?)
    }
}
