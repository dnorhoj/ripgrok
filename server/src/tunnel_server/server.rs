use std::io;
use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ::anyhow::{Context, anyhow};
use ::async_trait::async_trait;
use ::base64::Engine;
use ::base64::prelude::BASE64_STANDARD;
use ::dashmap::DashMap;
use ::dashmap::DashSet;
use ::tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use ::tokio::net::TcpListener;
use ::tokio::net::ToSocketAddrs;
use ::tokio::sync::oneshot;
use ::tokio::time::timeout;
use ::tokio_rustls::TlsAcceptor;
use ::tokio_rustls::rustls::ServerConfig;
use ::tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use ripgrok_common::ClientHello;

use crate::config::GeneralConfig;
use crate::config::TlsConfig;
use crate::tunnel_server::ControlConnection;

pub trait AsyncRW: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> AsyncRW for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

#[async_trait]
pub trait ServerListener {
    fn local_addr(&self) -> io::Result<SocketAddr>;
    fn protocol_specifier(&self) -> &'static str;
    async fn run(&self) -> anyhow::Result<()>;
}
pub struct TlsServerListener {
    server: Arc<TunnelServer>,
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

impl TlsServerListener {
    pub async fn new(
        server: Arc<TunnelServer>,
        addr: impl ToSocketAddrs,
        config: TlsConfig,
    ) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(addr).await?;

        let tls_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(Self::load_certs(&config)?, Self::load_key(&config)?)?;

        let acceptor = TlsAcceptor::from(Arc::new(tls_config));

        Ok(Self {
            server,
            listener,
            acceptor,
        })
    }

    fn load_certs(config: &TlsConfig) -> anyhow::Result<Vec<CertificateDer<'static>>> {
        Ok(match (&config.fullchain_b64, &config.fullchain_file) {
            (None, None) | (Some(_), Some(_)) => {
                return Err(anyhow!(
                    "You have to specify one of tls.fullchain_b64 or tls.fullchain_file"
                ));
            }
            (None, Some(path)) => {
                let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);

                rustls_pemfile::certs(&mut reader).collect::<Result<_, _>>()?
            }
            (Some(data), None) => {
                let mut cursor = Cursor::new(BASE64_STANDARD.decode(data)?);

                rustls_pemfile::certs(&mut cursor).collect::<Result<_, _>>()?
            }
        })
    }

    fn load_key(config: &TlsConfig) -> anyhow::Result<PrivateKeyDer<'static>> {
        Ok(match (&config.key_b64, &config.key_file) {
            (None, None) | (Some(_), Some(_)) => {
                return Err(anyhow!(
                    "You have to specify one of tls.key_b64 or tls.key_file"
                ));
            }
            (None, Some(path)) => {
                let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);

                rustls_pemfile::private_key(&mut reader)?
                    .ok_or(anyhow!("No key found in '{}'", path))?
            }
            (Some(data), None) => {
                let mut cursor = Cursor::new(BASE64_STANDARD.decode(data)?);

                rustls_pemfile::private_key(&mut cursor)?
                    .ok_or(anyhow!("Invalid or malformed key in tls.key_b64"))?
            }
        })
    }
}

#[async_trait]
impl ServerListener for TlsServerListener {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    fn protocol_specifier(&self) -> &'static str {
        "rgtps"
    }

    async fn run(&self) -> anyhow::Result<()> {
        tracing::info!(
            "Tunnel server listening on {}://{}",
            self.protocol_specifier(),
            self.local_addr()?
        );

        loop {
            let (stream, addr) = self.listener.accept().await?;
            let acceptor = self.acceptor.clone();
            let server = self.server.clone();

            tokio::spawn(async move {
                match acceptor.accept(stream).await {
                    Ok(tls_stream) => {
                        if let Err(e) = server.handle_new_connection(tls_stream, addr).await {
                            tracing::warn!(?addr, "Failed to handle connection: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(?addr, "TLS accept error: {}", e);
                    }
                }
            });
        }
    }
}

pub struct TcpServerListener {
    server: Arc<TunnelServer>,
    listener: TcpListener,
}

impl TcpServerListener {
    pub async fn new(server: Arc<TunnelServer>, addr: impl ToSocketAddrs) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(addr).await?;

        Ok(Self { server, listener })
    }
}

#[async_trait]
impl ServerListener for TcpServerListener {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    fn protocol_specifier(&self) -> &'static str {
        "rgtp"
    }

    async fn run(&self) -> anyhow::Result<()> {
        tracing::info!(
            "Tunnel server listening on {}://{}",
            self.protocol_specifier(),
            self.local_addr()?
        );

        loop {
            let (stream, addr) = self.listener.accept().await?;
            let server = self.server.clone();

            tokio::spawn(async move {
                if let Err(e) = server.handle_new_connection(stream, addr).await {
                    tracing::warn!(?addr, "Failed to handle connection: {}", e);
                }
            });
        }
    }
}

pub struct TunnelServer {
    pub general_config: GeneralConfig,

    pending_tunnel_conns: DashMap<u64, oneshot::Sender<Box<dyn AsyncRW>>>,
    control_connections: DashSet<Arc<ControlConnection>>,
}

impl TunnelServer {
    pub async fn new(general_config: GeneralConfig) -> Self {
        Self {
            general_config,

            pending_tunnel_conns: Default::default(),
            control_connections: Default::default(),
        }
    }

    async fn handle_new_connection(
        self: &Arc<TunnelServer>,
        mut stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
        addr: SocketAddr,
    ) -> anyhow::Result<()> {
        match timeout(Duration::from_secs(5), stream.read_u8())
            .await
            .context("Timeout in hello")??
        {
            ClientHello::INIT_CONTROL_CONNECTION => {
                tracing::info!(?addr, "New control connection");

                let conn = Arc::new(ControlConnection::new(self.clone(), addr).await);

                self.control_connections.insert(conn.clone());

                if let Err(e) = conn.run(stream).await {
                    tracing::warn!(?e, "Uncaught error while handling control connection");
                }

                self.control_connections.remove(&conn);
            }
            ClientHello::INIT_TUNNEL_CONNECTION => {
                let connection_id = timeout(Duration::from_secs(5), stream.read_u64())
                    .await
                    .context("Timeout in tunnel connection hello")??;

                tracing::debug!(?addr, ?connection_id, "Recveived TUNNEL_CONNECTION");

                if let Some(sender) = self.get_pending_connection(connection_id) {
                    if let Err(_) = sender.send(Box::new(stream)) {
                        tracing::warn!("Failed to send pending connection stream");
                    }
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

    pub fn add_pending_connection(
        &self,
        connection_id: u64,
        pending_connection: oneshot::Sender<Box<dyn AsyncRW>>,
    ) {
        self.pending_tunnel_conns
            .insert(connection_id, pending_connection);
    }

    pub fn get_pending_connection(
        &self,
        connection_id: u64,
    ) -> Option<oneshot::Sender<Box<dyn AsyncRW>>> {
        self.pending_tunnel_conns
            .remove(&connection_id)
            .map(|pending_connection| pending_connection.1)
    }

    pub fn remove_pending_connection(&self, connection_id: u64) -> () {
        self.pending_tunnel_conns.remove(&connection_id);
    }

    #[cfg(feature = "web-server")]
    pub fn get_control_connections(&self) -> Vec<Arc<ControlConnection>> {
        self.control_connections
            .iter()
            .map(|x| x.clone())
            .collect::<Vec<_>>()
    }
}
