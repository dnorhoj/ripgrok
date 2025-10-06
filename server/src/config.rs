use ::config::{Config as ConfigLoader, File};
use ::serde::Deserialize;

#[derive(Deserialize)]
pub struct GeneralConfig {
    pub public_host: Option<String>,
    /// Which ports clients are allowed to bind to. None to allow all ports.
    pub allowed_ports: Option<Vec<u16>>,
    /// Token that the client has to send to connect. No token required if None
    pub token: Option<String>,
}

#[derive(Deserialize)]
pub struct BindConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Deserialize)]
pub struct TlsConfig {
    pub fullchain_b64: Option<String>,
    pub fullchain_file: Option<String>,
    pub key_b64: Option<String>,
    pub key_file: Option<String>,
}

#[cfg(feature = "web-server")]
#[derive(Deserialize)]
pub struct WebServerAuthConfig {
    pub username: String,
    pub password: String,
}

#[cfg(feature = "web-server")]
#[derive(Deserialize)]
pub struct WebServerConfig {
    pub auth: Option<WebServerAuthConfig>,
    pub host: String,
    pub port: u16,
}

#[derive(Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub bind: BindConfig,
    #[cfg(feature = "web-server")]
    pub web_server: WebServerConfig,
    pub tls: Option<TlsConfig>,
}

impl Config {
    pub fn load(config_path: &str) -> anyhow::Result<Self> {
        let config_builder = ConfigLoader::builder()
            .set_default("general.allowed_ports", None::<Vec<u16>>)?
            .set_default("bind.host", "0.0.0.0")?
            .set_default("bind.port", 7267)?;

        #[cfg(feature = "web-server")]
        let config_builder = config_builder
            .set_default("web_server.host", "0.0.0.0")?
            .set_default("web_server.port", 5000)?;

        Ok(config_builder
            .add_source(File::with_name(config_path).required(false))
            .build()?
            .try_deserialize()?)
    }
}
