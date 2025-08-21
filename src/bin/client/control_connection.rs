use std::collections::HashMap;

use ::thiserror::Error;
use ::tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter, copy_bidirectional};
use ::tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use ::tokio::net::{TcpStream, ToSocketAddrs};
use ripgrok::tunnel_specifier::TunnelSpecifier;
use ripgrok::{ClientControlHello, ClientHello, ServerControlCommand, ServerControlHello};

#[derive(Debug, Error)]
enum ReadCommandError {
    #[error("Connection closed by server")]
    ServerClosed,
    #[error("Invalid command received from server")]
    InvalidCommand(#[from] serde_json::Error),
}

pub struct ControlConnection {
    server_addr: (String, u16),
    reader: BufReader<OwnedReadHalf>,
    writer: BufWriter<OwnedWriteHalf>,
    /// A map of server_port to client_port
    ports: HashMap<u16, u16>,
}

impl ControlConnection {
    async fn execute_handshake(
        reader: &mut BufReader<OwnedReadHalf>,
        writer: &mut BufWriter<OwnedWriteHalf>,
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
        reader: &mut BufReader<OwnedReadHalf>,
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
                        Some(port) => port,
                        None => continue,
                    };

                    let mut server_stream =
                        match Self::open_tunnel_connection(self.server_addr.clone(), connection_id)
                            .await
                        {
                            Ok(stream) => stream,
                            Err(e) => {
                                tracing::error!(
                                    ?e,
                                    "Failed to connect to server to start tunnel connection."
                                );
                                continue;
                            }
                        };

                    let mut local_stream =
                        match TcpStream::connect(("127.0.0.1", *client_port)).await {
                            Ok(stream) => stream,
                            Err(e) => {
                                tracing::warn!(?e, "Failed to connect to local service");
                                continue;
                            }
                        };

                    if let Err(e) = copy_bidirectional(&mut server_stream, &mut local_stream).await
                    {
                        tracing::warn!(
                            ?e,
                            "An error occurred while copying data between tunnel and local stream."
                        );
                        continue;
                    };
                }
            }
        }
    }

    pub async fn init(
        server_addr: (String, u16),
        tunnel_specifiers: Vec<TunnelSpecifier>,
    ) -> anyhow::Result<Self> {
        let stream = TcpStream::connect(server_addr.clone()).await?;

        let (r, w) = stream.into_split();
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
                    writer,
                })
            }
        }
    }
}
