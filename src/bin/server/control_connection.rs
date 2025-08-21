use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ::anyhow::Context;
use ::tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use ::tokio::net::TcpStream;
use ::tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use ::tokio::select;
use ::tokio::sync::{Mutex, mpsc};
use ::tokio::task::JoinSet;
use ::tokio::time::timeout;
use ::tokio_util::sync::CancellationToken;
use ripgrok::{ClientControlHello, ServerControlCommand, ServerControlHello};

use crate::tunnel_listener::TunnelListener;

pub struct ControlConnection {
    addr: SocketAddr,
    token: CancellationToken,
}

impl ControlConnection {
    pub fn new(addr: SocketAddr) -> Self {
        ControlConnection {
            addr,
            token: CancellationToken::new(),
        }
    }

    async fn start_reader_task(&self, mut stream: BufReader<OwnedReadHalf>) -> () {
        let token = self.token.clone();

        tokio::spawn(async move {
            let mut line = String::new();
            loop {
                line.clear();
                let n = {
                    match stream.read_line(&mut line).await {
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
                    tracing::info!("peer closed (EOF on control connection)");
                    token.cancel();
                    break;
                }

                // TODO; Handle messages from the client?
            }
        });
    }

    async fn start_writer_queue(
        &self,
        stream: Arc<Mutex<BufWriter<OwnedWriteHalf>>>,
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

    pub async fn run(self, stream: TcpStream, public_host: Option<String>) -> anyhow::Result<()> {
        let (read_half, write_half) = stream.into_split();
        let mut read_buf = BufReader::new(read_half);
        let write_buf = Arc::new(Mutex::new(BufWriter::new(write_half)));

        // Read client hello
        let mut buf = String::new();
        timeout(Duration::from_secs(5), read_buf.read_line(&mut buf))
            .await?
            .context("Timeout while waiting for control connection parameters")?;
        let hello: ClientControlHello = serde_json::from_str(&buf)?;

        // Init queue for tunnel listeners to push ControlRequests to
        self.start_reader_task(read_buf).await;
        let command_tx = self.start_writer_queue(write_buf.clone()).await;

        // Create all listeners
        let mut tunnel_listener_init_set: JoinSet<anyhow::Result<TunnelListener>> = JoinSet::new();

        for specifier in hello.tunnel_specifiers {
            tunnel_listener_init_set.spawn({
                let tx = command_tx.clone();
                let token = self.token.clone();

                async move {
                    let tunnel_listener = match TunnelListener::init(token, specifier, tx).await {
                        Ok(tunnel_listener) => tunnel_listener,
                        Err(e) => {
                            tracing::error!(
                                ?e,
                                "An error occurred while initiating tunnel listener",
                            );
                            return Err(e);
                        }
                    };

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

        for listener in tunnel_listener_init_set.join_all().await {
            let listener = listener?;

            new_specifiers.push(listener.specifier);

            tunnel_listener_set.spawn(async move {
                if let Err(e) = listener.run().await {
                    tracing::error!(?e, "An error occurred in tunnel listener");
                }
            });
        }

        // Respond to handshake
        {
            let hello_json = serde_json::to_vec(&ServerControlHello::Listening {
                specifiers: new_specifiers,
                public_host,
            })?;

            let mut w = write_buf.lock().await;

            w.write_all(&hello_json).await?;
            w.write_all(b"\n").await?;
            w.flush().await?;
        }

        tunnel_listener_set.join_all().await;

        Ok(())
    }
}

impl Drop for ControlConnection {
    fn drop(&mut self) {
        tracing::info!("Control connection to {} closed", self.addr);
        self.token.cancel();
    }
}
