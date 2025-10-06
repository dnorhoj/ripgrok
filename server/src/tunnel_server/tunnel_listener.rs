use std::io;
use std::{net::SocketAddr, sync::Arc, time::Duration};

use ::dashmap::DashSet;
use ::tokio::net::{TcpListener, TcpStream};
use ::tokio::select;
use ::tokio::sync::{mpsc, oneshot};
use ::tokio::time::timeout;
use ::tokio_util::sync::CancellationToken;
use ripgrok_common::tunnel_specifier::{ResolvedTunnelSpecifier, TunnelSpecifier};
use ripgrok_common::{ServerControlCommand, get_random_u64};

use crate::tunnel_server::TunnelServer;
use crate::tunnel_server::tunnel_connection::TunnelConnection;

pub struct TunnelListener {
    server: Arc<TunnelServer>,
    token: CancellationToken,
    tx: mpsc::Sender<ServerControlCommand>,
    listener: TcpListener,
    pub specifier: ResolvedTunnelSpecifier,

    connections: Arc<DashSet<Arc<TunnelConnection>>>,
}

impl TunnelListener {
    pub async fn init(
        server: Arc<TunnelServer>,
        token: CancellationToken,
        specifier: TunnelSpecifier,
        tx: mpsc::Sender<ServerControlCommand>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", specifier.server_port.unwrap_or(0))).await?;

        Ok(Self {
            server,
            specifier: ResolvedTunnelSpecifier {
                tunnel_type: specifier.tunnel_type,
                client_port: specifier.client_port,
                server_port: listener.local_addr()?.port(),
            },
            listener,
            token,
            tx,

            connections: Default::default(),
        })
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        loop {
            select! {
                _ = self.token.cancelled() => {
                    tracing::debug!("Control connection cancelled, stopping tunnel listener for {:?}", self.specifier);
                    break;
                }
                Ok((stream, addr)) = self.listener.accept() => {
                    self.handle_connection(stream, addr).await?;
                }
            }
        }

        Ok(())
    }

    async fn handle_connection(&self, stream: TcpStream, addr: SocketAddr) -> anyhow::Result<()> {
        tracing::info!("Accepted new connection from {}", addr);

        let connection_id = get_random_u64().await?;

        let (sender, receiver) = oneshot::channel();

        // Store the stream in pending connections
        self.server.add_pending_connection(connection_id, sender);

        self.tx
            .send(ServerControlCommand::StartTunnelConnection {
                connection_id,
                server_port: self.specifier.server_port,
            })
            .await?;

        tokio::spawn({
            let server = self.server.clone();
            let connections = self.connections.clone();

            async move {
                match timeout(Duration::from_secs(10), receiver).await {
                    Ok(proxy_connection) => {
                        let proxy_connection = proxy_connection.unwrap();

                        let conn = Arc::new(TunnelConnection::new(connection_id, addr));

                        connections.insert(conn.clone());

                        conn.run(stream, proxy_connection).await;

                        connections.remove(&conn);
                    }
                    Err(_) => {
                        // Timeout exceeded - remove pending conn
                        tracing::warn!(?connection_id, "Did not receive TunnelConnection in time");
                        server.remove_pending_connection(connection_id);
                    }
                }
            }
        });

        Ok(())
    }

    #[cfg(feature = "web-server")]
    pub fn get_connections(&self) -> Vec<Arc<TunnelConnection>> {
        self.connections
            .iter()
            .map(|conn| conn.clone())
            .collect::<Vec<_>>()
    }
}
