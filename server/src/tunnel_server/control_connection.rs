use std::hash::Hash;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ::anyhow::{Context, anyhow, bail};
use ::tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, ReadHalf,
    WriteHalf, split,
};
use ::tokio::select;
use ::tokio::sync::{Mutex, RwLock, mpsc};
use ::tokio::task::JoinSet;
use ::tokio::time::timeout;
use ::tokio_util::sync::CancellationToken;
use ripgrok_common::{
    ClientControlHello, ServerControlCommand, ServerControlHello, get_random_u64,
};

use crate::tunnel_server::{TunnelListener, TunnelServer};

enum HelloValidation {
    Success,
    InvalidToken,
    ForbiddenPorts(Vec<u16>),
}

pub struct ControlConnection {
    pub id: u64,
    server: Arc<TunnelServer>,
    pub addr: SocketAddr,
    token: CancellationToken,

    tunnel_listeners: RwLock<Vec<Arc<TunnelListener>>>,
}

impl PartialEq for ControlConnection {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for ControlConnection {}

impl Hash for ControlConnection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.addr.hash(state);
    }

    fn hash_slice<H: std::hash::Hasher>(data: &[Self], state: &mut H)
    where
        Self: Sized,
    {
        for piece in data {
            piece.hash(state)
        }
    }
}

impl ControlConnection {
    pub async fn new(server: Arc<TunnelServer>, addr: SocketAddr) -> Self {
        ControlConnection {
            id: get_random_u64().await.unwrap(),
            server,
            addr,
            token: Default::default(),

            tunnel_listeners: Default::default(),
        }
    }

    async fn start_reader_task(
        &self,
        mut stream: BufReader<ReadHalf<impl AsyncRead + Send + Sync + 'static>>,
    ) -> () {
        let token = self.token.clone();

        tokio::spawn(async move {
            let mut line = String::new();
            loop {
                line.clear();
                let n = select! {
                    _ = token.cancelled() => {
                        tracing::debug!("Control connection cancelled, stopping reader processing");
                        break;
                    }
                    l = stream.read_line(&mut line) => match l {
                        Ok(n) => n,
                        Err(e) => {
                            tracing::warn!(?e, "read error; assuming peer closed");
                            token.cancel();
                            break;
                        }
                    }
                };

                if n == 0 {
                    // EOF: remote closed the connection.
                    tracing::debug!("Connection closed by peer");
                    token.cancel();
                    break;
                }

                // TODO; Handle messages from the client?
            }
        });
    }

    async fn start_writer_queue(
        &self,
        stream: Arc<Mutex<BufWriter<WriteHalf<impl AsyncWrite + Send + Sync + 'static>>>>,
    ) -> mpsc::Sender<ServerControlCommand> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(128);

        let token = self.token.clone();

        tokio::spawn(async move {
            loop {
                select! {
                    _ = token.cancelled() => {
                        tracing::debug!("Control connection cancelled, stopping queue processing");
                        break;
                    }
                    Some(command) = rx.recv() => {
                        // Send the command to the stream
                        let command_json = match serde_json::to_vec(&command) {
                            Ok(vec) => vec,
                            Err(e) => {
                                tracing::error!(?e, "Failed to serialize command: {:?}", command);
                                continue;
                            },
                        };

                        let mut s = stream.lock().await;
                        if s.write_all(&command_json).await.is_ok() {
                            if s.write_all(b"\n").await.is_ok() {
                                if s.flush().await.is_ok() {
                                    // Don't trigger error
                                    continue;
                                }
                            }
                        }

                        tracing::warn!("write failed; assuming peer closed");
                        token.cancel();
                        break;
                    }
                }
            }
        });

        tx
    }

    fn validate_hello(&self, hello: &ClientControlHello) -> HelloValidation {
        if self
            .server
            .general_config
            .token
            .as_ref()
            .is_some_and(|token| {
                hello
                    .token
                    .as_ref()
                    .is_none_or(|hello_token| hello_token != token)
            })
        {
            return HelloValidation::InvalidToken;
        }

        if let Some(allowed_ports) = &self.server.general_config.allowed_ports {
            tracing::info!(?allowed_ports);

            let mut forbidden_ports = Vec::new();

            for specifier in hello.specifiers.iter() {
                tracing::info!(?specifier);
                match specifier.server_port {
                    Some(port) => {
                        if !allowed_ports.contains(&port) {
                            forbidden_ports.push(port);
                        }
                    }
                    None => forbidden_ports.push(0),
                }
            }

            if !forbidden_ports.is_empty() {
                return HelloValidation::ForbiddenPorts(forbidden_ports);
            }
        }

        HelloValidation::Success
    }

    pub async fn run(
        self: &Arc<Self>,
        stream: impl AsyncRead + AsyncWrite + Send + Sync + 'static,
    ) -> anyhow::Result<()> {
        let (read_half, write_half) = split(stream);
        let mut read_buf = BufReader::new(read_half);
        let write_buf = Arc::new(Mutex::new(BufWriter::new(write_half)));

        // Read client hello
        let mut buf = String::new();
        timeout(Duration::from_secs(5), read_buf.read_line(&mut buf))
            .await?
            .context("Timeout while waiting for control connection parameters")?;
        let hello: ClientControlHello = serde_json::from_str(&buf)?;

        // Validate hello
        match self.validate_hello(&hello) {
            HelloValidation::Success => {}
            HelloValidation::InvalidToken => {
                tracing::info!(?self.addr, "Invalid token");

                self.send_hello(
                    &ServerControlHello::error("Invalid token"),
                    &mut *write_buf.lock().await,
                )
                .await?;

                return Ok(());
            }
            HelloValidation::ForbiddenPorts(forbidden_ports) => {
                // Respond to handshake
                self.send_hello(
                    &ServerControlHello::error(format!(
                        "Cannot bind to following port(s): {}",
                        forbidden_ports
                            .iter()
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                    &mut *write_buf.lock().await,
                )
                .await?;

                return Ok(());
            }
        }

        // Init queue for tunnel listeners to push ControlRequests to
        self.start_reader_task(read_buf).await;
        let command_tx = self.start_writer_queue(write_buf.clone()).await;

        // Create all listeners
        let mut tunnel_listener_init_set: JoinSet<io::Result<Arc<TunnelListener>>> = JoinSet::new();

        for specifier in hello.specifiers {
            tunnel_listener_init_set.spawn({
                let tx = command_tx.clone();
                let token = self.token.clone();
                let server = self.server.clone();

                async move {
                    let tunnel_listener =
                        Arc::new(TunnelListener::init(server, token, specifier, tx).await?);

                    tracing::info!(
                        "Tunnel listener bound to {} :{}",
                        tunnel_listener.specifier.tunnel_type,
                        tunnel_listener.specifier.server_port
                    );

                    Ok(tunnel_listener)
                }
            });
        }

        // Get the resolved specifiers, and start tunnel listeners
        let mut tunnel_listener_set = JoinSet::<()>::new();
        let mut new_specifiers = Vec::new();

        *self.tunnel_listeners.write().await = match tunnel_listener_init_set
            .join_all()
            .await
            .into_iter()
            .collect::<Result<_, _>>()
        {
            Ok(listeners) => listeners,
            Err(e) => {
                let err_message = match e.kind() {
                    io::ErrorKind::PermissionDenied => {
                        "Permission denied - cannot bind to one or more ports"
                    }
                    io::ErrorKind::AddrInUse => "One or more ports are already in use",
                    _ => "Unknown error!",
                };

                self.send_hello(
                    &ServerControlHello::error(err_message),
                    &mut *write_buf.lock().await,
                )
                .await?;

                bail!(anyhow!(e).context("One or more tunnel listeners failed to initialize"))
            }
        };

        for listener in self.tunnel_listeners.read().await.iter() {
            new_specifiers.push(listener.specifier);

            tunnel_listener_set.spawn({
                let listener = listener.clone();

                async move {
                    if let Err(e) = listener.run().await {
                        tracing::error!(?e, "An error occurred in tunnel listener");
                    }
                }
            });
        }

        // Respond to handshake
        self.send_hello(
            &ServerControlHello::Listening {
                specifiers: new_specifiers,
                public_host: self.server.general_config.public_host.clone(),
            },
            &mut *write_buf.lock().await,
        )
        .await?;

        tunnel_listener_set.join_all().await;

        Ok(())
    }

    pub async fn send_hello(
        &self,
        hello: &ServerControlHello,
        write_buf: &mut BufWriter<impl AsyncWrite + Unpin>,
    ) -> anyhow::Result<()> {
        let hello_json = serde_json::to_vec(hello)?;

        write_buf.write_all(&hello_json).await?;
        write_buf.write_all(b"\n").await?;
        write_buf.flush().await?;

        Ok(())
    }

    #[cfg(feature = "web-server")]
    pub async fn get_listeners(&self) -> Vec<Arc<TunnelListener>> {
        self.tunnel_listeners.read().await.clone()
    }
}

impl Drop for ControlConnection {
    fn drop(&mut self) {
        tracing::info!(?self.addr, "Control connection closed");
        self.token.cancel();
    }
}
