use std::collections::HashMap;
use std::io::{self, Cursor};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ::anyhow::anyhow;
use ::async_trait::async_trait;
use ::base64::Engine;
use ::base64::prelude::BASE64_STANDARD;
use ::thiserror::Error;
use ::tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, ReadHalf,
    WriteHalf, split,
};
use ::tokio::net::TcpStream;
use ::tokio::time::timeout;
use ::tokio_rustls::TlsConnector;
use ::tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
use ::tokio_rustls::rustls::{ClientConfig, RootCertStore};
use ripgrok_common::copy::my_copy_bidirectional;
use ripgrok_common::tunnel_specifier::TunnelSpecifier;
use ripgrok_common::{ClientControlHello, ClientHello, ServerControlCommand, ServerControlHello};

use crate::utils::config::TlsConfig;

#[derive(Debug, Error)]
enum ReadCommandError {
    #[error("Connection closed by server")]
    ServerClosed,
    #[error("Invalid command received from server")]
    InvalidCommand(#[from] serde_json::Error),
}

pub trait AsyncRW: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> AsyncRW for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub struct ControlConnection {
    reader: BufReader<ReadHalf<Box<dyn AsyncRW>>>,
    _writer: BufWriter<WriteHalf<Box<dyn AsyncRW>>>,
    /// A map of server_port to client_port
    ports: HashMap<u16, u16>,

    conn_initializer: Arc<dyn ConnectionInitializer>,
}

impl ControlConnection {
    pub async fn init(
        conn_initializer: Arc<dyn ConnectionInitializer>,
        tunnel_specifiers: Vec<TunnelSpecifier>,
        token: Option<String>,
    ) -> anyhow::Result<Self> {
        let stream = conn_initializer.new_connection().await?;

        let (r, w) = split(stream);
        let mut reader = BufReader::new(r);
        let mut writer = BufWriter::new(w);

        let response = timeout(
            Duration::from_secs(10),
            Self::execute_handshake(
                &mut reader,
                &mut writer,
                &ClientControlHello {
                    specifiers: tunnel_specifiers,
                    token,
                },
            ),
        )
        .await??;

        match response {
            ServerControlHello::Error { reason } => {
                tracing::error!("Server could not handle connection: {}", reason);
                return Err(anyhow::anyhow!("Server said goodbye: {}", reason));
            }
            ServerControlHello::Listening {
                specifiers,
                public_host,
            } => {
                tracing::info!("Connected!");

                let mut ports = HashMap::new();

                for specifier in &specifiers {
                    tracing::info!(
                        "You can now access your local port {} through {}://{}:{}",
                        specifier.client_port,
                        specifier.tunnel_type,
                        public_host
                            .as_ref()
                            .unwrap_or(&conn_initializer.server_name_fallback()),
                        specifier.server_port
                    );

                    ports.insert(specifier.server_port, specifier.client_port);
                }

                Ok(Self {
                    ports,
                    reader,
                    _writer: writer,
                    conn_initializer,
                })
            }
        }
    }

    async fn execute_handshake(
        reader: &mut BufReader<ReadHalf<Box<dyn AsyncRW>>>,
        writer: &mut BufWriter<WriteHalf<Box<dyn AsyncRW>>>,
        hello: &ClientControlHello,
    ) -> anyhow::Result<ServerControlHello> {
        writer
            .write_u8(ClientHello::INIT_CONTROL_CONNECTION)
            .await?;

        writer.write_all(&serde_json::to_vec(hello)?).await?;

        writer.write_all(b"\n").await?;

        writer.flush().await?;

        let mut buf = String::new();

        reader.read_line(&mut buf).await?;

        if buf.is_empty() {
            return Err(anyhow::anyhow!("Server did not respond to handshake"));
        }

        Ok(serde_json::from_str(&buf)?)
    }

    async fn read_command(
        reader: &mut BufReader<ReadHalf<Box<dyn AsyncRW>>>,
    ) -> Result<ServerControlCommand, ReadCommandError> {
        let mut command = String::new();
        match reader.read_line(&mut command).await {
            Ok(size) => {
                if size == 0 {
                    tracing::info!("EOF Received?");
                    return Err(ReadCommandError::ServerClosed);
                }
            }
            Err(e) => {
                tracing::warn!(
                    ?e,
                    "Unknown error occurred while getting command from server"
                );
                return Err(ReadCommandError::ServerClosed);
            }
        };

        Ok(serde_json::from_str(&command)?)
    }

    //async fn start_tunnel(connection_id)

    async fn open_tunnel_connection(
        connection_id: u64,
        connection_initializer: Arc<dyn ConnectionInitializer>,
    ) -> anyhow::Result<Box<dyn AsyncRW>> {
        let mut server_stream = connection_initializer.new_connection().await?;

        server_stream
            .write_u8(ClientHello::INIT_TUNNEL_CONNECTION)
            .await?;
        server_stream.write_u64(connection_id).await?;
        server_stream.flush().await?;

        Ok(server_stream)
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        loop {
            match Self::read_command(&mut self.reader).await? {
                ServerControlCommand::StartTunnelConnection {
                    connection_id,
                    server_port,
                } => {
                    let client_port = match self.ports.get(&server_port) {
                        Some(port) => port.clone(),
                        None => continue,
                    };

                    let connection_initializer = self.conn_initializer.clone();

                    tokio::spawn(async move {
                        let mut server_stream = match Self::open_tunnel_connection(
                            connection_id,
                            connection_initializer,
                        )
                        .await
                        {
                            Ok(stream) => stream,
                            Err(e) => {
                                tracing::error!(
                                    ?e,
                                    "Failed to connect to server to start tunnel connection."
                                );
                                return;
                            }
                        };

                        let mut local_stream =
                            match TcpStream::connect(("127.0.0.1", client_port)).await {
                                Ok(stream) => stream,
                                Err(e) => {
                                    tracing::warn!(?e, "Failed to connect to local service");
                                    return;
                                }
                            };

                        if let Err(e) =
                            my_copy_bidirectional(&mut server_stream, &mut local_stream).await
                        {
                            tracing::warn!(
                                ?e,
                                "An error occurred while copying data between tunnel and local stream."
                            );
                        };
                    });
                }
            }
        }
    }
}

#[async_trait]
pub trait ConnectionInitializer: Send + Sync {
    async fn new_connection(&self) -> anyhow::Result<Box<dyn AsyncRW>>;
    fn protocol_identifier(&self) -> &'static str;

    fn server_name_fallback(&self) -> String {
        "<server_host>".to_string()
    }
}

pub struct TcpInitializer {
    server_addr: (String, u16),
}

impl TcpInitializer {
    pub fn new(server_addr: (String, u16)) -> Self {
        Self { server_addr }
    }
}

#[async_trait]
impl ConnectionInitializer for TcpInitializer {
    async fn new_connection(&self) -> anyhow::Result<Box<dyn AsyncRW>> {
        let stream = TcpStream::connect(self.server_addr.clone()).await?;

        Ok(Box::new(stream))
    }

    fn protocol_identifier(&self) -> &'static str {
        "rgtp"
    }

    fn server_name_fallback(&self) -> String {
        self.server_addr.0.clone()
    }
}

pub struct TlsInitializer {
    server_addr: SocketAddr,
    server_name: ServerName<'static>,
    cert_store: Arc<RootCertStore>,
}

impl TlsInitializer {
    pub fn new(addr: (String, u16), config: &TlsConfig) -> anyhow::Result<Self> {
        let server_addr = std::net::ToSocketAddrs::to_socket_addrs(&addr)?
            .next()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;

        let server_name = ServerName::try_from(addr.0.clone())?;

        let mut cert_store = RootCertStore::empty();
        cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        if let Some(cert) = Self::get_trusted_cert(config)? {
            cert_store.add(cert)?;
        }

        Ok(Self {
            server_addr,
            server_name,
            cert_store: Arc::new(cert_store),
        })
    }

    fn get_trusted_cert(config: &TlsConfig) -> anyhow::Result<Option<CertificateDer<'static>>> {
        Ok(match (&config.cert_b64, &config.cert_file) {
            (None, None) => None,
            (Some(_), Some(_)) => {
                return Err(anyhow!(
                    "You cannot specify both tls.cert_b64 and tls.cert_file"
                ));
            }
            (None, Some(path)) => {
                let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);

                rustls_pemfile::certs(&mut reader).last().transpose()?
            }
            (Some(encoded), None) => {
                let mut cursor = Cursor::new(BASE64_STANDARD.decode(encoded)?);

                rustls_pemfile::certs(&mut cursor).last().transpose()?
            }
        })
    }
}

#[async_trait]
impl ConnectionInitializer for TlsInitializer {
    async fn new_connection(&self) -> anyhow::Result<Box<dyn AsyncRW>> {
        let config = ClientConfig::builder()
            .with_root_certificates(Arc::clone(&self.cert_store))
            .with_no_client_auth();

        let connector = TlsConnector::from(Arc::new(config));

        let stream = TcpStream::connect(self.server_addr).await?;

        Ok(Box::new(
            connector.connect(self.server_name.clone(), stream).await?,
        ))
    }

    fn protocol_identifier(&self) -> &'static str {
        "rgtps"
    }

    fn server_name_fallback(&self) -> String {
        self.server_name.to_str().into()
    }
}
