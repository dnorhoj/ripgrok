use ::config::{Config as ConfigLoader, File};
use ::serde::Deserialize;

#[derive(Deserialize)]
pub struct BindConfig {
    pub host: String,
    pub port: u16,
    pub public_host: Option<String>,
}

#[derive(Deserialize)]
pub struct Config {
    pub bind: BindConfig,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        Ok(ConfigLoader::builder()
            .set_default("bind.host", "0.0.0.0")?
            .set_default("bind.port", 7267)?
            .add_source(File::with_name("config.toml").required(false))
            .build()?
            .try_deserialize()?)
    }
}
