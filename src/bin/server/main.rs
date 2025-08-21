use std::net::SocketAddr;
use std::sync::LazyLock;
use std::time::Duration;

use ::anyhow::Context;
use ::dashmap::DashMap;
use ::tokio::io::{AsyncReadExt, copy_bidirectional};
use ::tokio::net::{TcpListener, TcpStream};
use ::tokio::time::timeout;
use ::tracing_subscriber::FmtSubscriber;
use ripgrok::ClientHello;

use crate::config::Config;
use crate::control_connection::ControlConnection;

mod config;
mod control_connection;
mod tunnel_listener;

static PENDING_TUNNEL_CONNS: LazyLock<DashMap<u64, TcpStream>> = LazyLock::new(|| DashMap::new());

async fn handle_new_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    public_host: Option<String>,
) -> anyhow::Result<()> {
    match timeout(Duration::from_secs(5), stream.read_u8())
        .await
        .context("Timeout in hello")??
    {
        ClientHello::INIT_CONTROL_CONNECTION => {
            tracing::info!("Received INIT_CONTROL_CONNECTION from {}", addr);

            let conn = ControlConnection::new(addr);

            conn.run(stream, public_host).await?;
        }
        ClientHello::INIT_TUNNEL_CONNECTION => {
            let connection_id = timeout(Duration::from_secs(5), stream.read_u64())
                .await
                .context("Timeout in tunnel connection hello")??;

            if let Some((_, mut conn)) = PENDING_TUNNEL_CONNS.remove(&connection_id) {
                copy_bidirectional(&mut conn, &mut stream).await?;
            } else {
                tracing::warn!(
                    ?connection_id,
                    "Got INIT_TUNNEL_CONNECTION on invalid connection_id"
                )
            }
        }
        _ => {
            tracing::error!("Unknown ClientHello type received from {}", addr);
            return Err(anyhow::anyhow!("Unknown ClientHello type"));
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing::subscriber::set_global_default(FmtSubscriber::default())?;

    let config = Config::load()?;

    let listener = TcpListener::bind((config.bind.host, config.bind.port)).await?;

    tracing::info!("Server listening on {}", listener.local_addr()?);

    loop {
        let (stream, addr) = listener.accept().await?;
        let public_host = config.bind.public_host.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_new_connection(stream, addr, public_host).await {
                tracing::warn!(?addr, "Failed to handle connection: {}", e);
            }
        });
    }
}
