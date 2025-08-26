use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use ::thiserror::Error;
use ::tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, ReadHalf,
    WriteHalf, copy_bidirectional, split,
};
use ::tokio::net::{TcpStream, ToSocketAddrs};
use ::tokio_rustls::TlsConnector;
use ::tokio_rustls::client::TlsStream;
use ::tokio_rustls::rustls::pki_types::ServerName;
use ::tokio_rustls::rustls::{ClientConfig, RootCertStore};
use ripgrok::tunnel_specifier::TunnelSpecifier;
use ripgrok::{ClientControlHello, ClientHello, ServerControlCommand, ServerControlHello};

#[derive(Debug, Error)]
enum ReadCommandError {
    #[error("Connection closed by server")]
    ServerClosed,
    #[error("Invalid command received from server")]
    InvalidCommand(#[from] serde_json::Error),
}

pub struct ControlConnection<R, W>
where
    R: AsyncRead,
    W: AsyncWrite,
{
    server_addr: (String, u16),
    reader: BufReader<ReadHalf<R>>,
    // Writer is not currently used, but if we drop it from scope, the connection is terminated
    // also, we're probably gonna use it in the future
    _writer: BufWriter<WriteHalf<W>>,
    /// A map of server_port to client_port
    ports: HashMap<u16, u16>,
}

impl<R, W> ControlConnection<R, W>
where
    R: AsyncRead,
    W: AsyncWrite,
{
    async fn execute_handshake(
        reader: &mut BufReader<ReadHalf<impl AsyncRead>>,
        writer: &mut BufWriter<WriteHalf<impl AsyncWrite>>,
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
        reader: &mut BufReader<ReadHalf<R>>,
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
        server_addr: impl ToSocketAddrs,
        connection_id: u64,
    ) -> anyhow::Result<TcpStream> {
        let mut server_stream = TcpStream::connect(server_addr).await?;

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

                    let server_addr = self.server_addr.clone();

                    tokio::spawn(async move {
                        let mut server_stream =
                            match Self::open_tunnel_connection(server_addr, connection_id).await {
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
                            copy_bidirectional(&mut server_stream, &mut local_stream).await
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

impl ControlConnection<TlsStream<TcpStream>, TlsStream<TcpStream>> {
    pub async fn init(
        server_addr: (String, u16),
        tunnel_specifiers: Vec<TunnelSpecifier>,
    ) -> anyhow::Result<Self> {
        let addr = std::net::ToSocketAddrs::to_socket_addrs(&server_addr)?
            .next()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;

        let mut cert_store = RootCertStore::empty();
        cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = ClientConfig::builder()
            .with_root_certificates(cert_store)
            .with_no_client_auth();

        let connector = TlsConnector::from(Arc::new(config));

        let stream = TcpStream::connect(addr).await?;

        let server_name = ServerName::try_from(server_addr.0.clone())?;

        let stream = connector.connect(server_name, stream).await?;

        let (r, w) = split(stream);
        let mut reader = BufReader::new(r);
        let mut writer = BufWriter::new(w);

        let response = Self::execute_handshake(
            &mut reader,
            &mut writer,
            &ClientControlHello {
                tunnel_specifiers,
                ssl: false,
            },
        )
        .await?;

        match response {
            ServerControlHello::Goodbye { reason } => {
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
                        public_host.as_ref().unwrap_or(&server_addr.0),
                        specifier.server_port
                    );

                    ports.insert(specifier.server_port, specifier.client_port);
                }

                Ok(Self {
                    server_addr,
                    ports,
                    reader,
                    _writer: writer,
                })
            }
        }
    }
}
