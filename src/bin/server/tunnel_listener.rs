use ::tokio::fs::File;
use ::tokio::io::AsyncReadExt;
use ::tokio::net::TcpListener;
use ::tokio::select;
use ::tokio::sync::mpsc;
use ::tokio_util::sync::CancellationToken;
use ripgrok::{
    ServerControlCommand,
    tunnel_specifier::{ResolvedTunnelSpecifier, TunnelSpecifier},
};

use crate::PENDING_TUNNEL_CONNS;

pub struct TunnelListener {
    token: CancellationToken,
    tx: mpsc::Sender<ServerControlCommand>,
    listener: TcpListener,
    pub specifier: ResolvedTunnelSpecifier,
}

impl TunnelListener {
    pub async fn init(
        token: CancellationToken,
        specifier: TunnelSpecifier,
        tx: mpsc::Sender<ServerControlCommand>,
    ) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", specifier.server_port.unwrap_or(0)))
            .await
            .map_err(|e| {
                tracing::error!("Failed to bind tunnel listener: {}", e);
                anyhow::anyhow!("Failed to bind tunnel listener")
            })?;

        Ok(Self {
            specifier: ResolvedTunnelSpecifier {
                tunnel_type: specifier.tunnel_type,
                client_port: specifier.client_port,
                server_port: listener.local_addr()?.port(),
            },
            listener,
            token,
            tx,
        })
    }

    async fn get_random_u64() -> tokio::io::Result<u64> {
        // TODO; lmao
        let mut file = File::open("/dev/urandom").await?;
        Ok(file.read_u64().await?)
    }

    pub async fn run(self) -> anyhow::Result<()> {
        loop {
            select! {
                _ = self.token.cancelled() => {
                    tracing::info!("Control connection cancelled, stopping tunnel listener for {:?}", self.specifier);
                    break;
                }
                Ok((stream, addr)) = self.listener.accept() => {
                    tracing::info!("Accepted new connection from {}", addr);

                    let connection_id = Self::get_random_u64().await?;

                    // Store the stream in pending connections
                    PENDING_TUNNEL_CONNS.insert(connection_id, stream);

                    self.tx.send(ServerControlCommand::StartTunnelConnection {
                        connection_id,
                        server_port: self.specifier.server_port
                    }).await?;
                }
            }
        }

        Ok(())
    }
}
