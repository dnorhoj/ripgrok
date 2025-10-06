use std::sync::Arc;

use crate::config::Config;
use crate::tunnel_server::{ServerListener, TcpServerListener, TlsServerListener, TunnelServer};

#[cfg(feature = "web-server")]
use crate::web_server::WebServer;

// Core
mod config;
mod tunnel_server;

#[cfg(feature = "web-server")]
mod web_server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .event_format(
            tracing_subscriber::fmt::format(), //.with_file(true).with_line_number(true),
        )
        .init();

    let config = Config::load("server_config.toml")?;

    let tunnel_server = Arc::new(TunnelServer::new(config.general).await);

    let addr = (config.bind.host, config.bind.port);
    let server_listener: Box<dyn ServerListener> = if let Some(tls_config) = config.tls {
        Box::new(TlsServerListener::new(tunnel_server.clone(), addr, tls_config).await?)
    } else {
        tracing::warn!("Using unencrypted connection - consider enabling TLS");
        Box::new(TcpServerListener::new(tunnel_server.clone(), addr).await?)
    };

    #[cfg(feature = "web-server")]
    {
        let web_server = WebServer::new(tunnel_server.clone(), config.web_server);

        tokio::spawn(async move {
            web_server.run().await.unwrap();
        });
    }

    server_listener.run().await
}
