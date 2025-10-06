use std::hash::Hash;
use std::net::SocketAddr;

use ::tokio::io::{AsyncRead, AsyncWrite};
use ::tokio::net::TcpStream;
use ::tokio::sync::{RwLock, watch};
use ripgrok_common::copy::my_copy_bidirectional_counting;

pub struct TunnelConnection {
    pub connection_id: u64,
    pub addr: SocketAddr,

    pub bytes_written: RwLock<Option<(watch::Receiver<u64>, watch::Receiver<u64>)>>,
}

impl TunnelConnection {
    pub fn new(connection_id: u64, addr: SocketAddr) -> Self {
        Self {
            connection_id,
            addr,

            bytes_written: Default::default(),
        }
    }

    pub async fn run(
        &self,
        incoming: TcpStream,
        stream: impl AsyncRead + AsyncWrite + Unpin + Send,
    ) {
        let (w_client_to_server, r_client_to_server) = watch::channel(0_u64);
        let (w_server_to_client, r_server_to_client) = watch::channel(0_u64);

        *self.bytes_written.write().await = Some((r_client_to_server, r_server_to_client));

        if let Err(e) =
            my_copy_bidirectional_counting(incoming, stream, w_client_to_server, w_server_to_client)
                .await
        {
            tracing::warn!(
                ?e,
                "An error occurred while copying data between client and tunnel client."
            );
        }
    }

    #[cfg(feature = "web-server")]
    pub async fn get_bytes_transfered(&self) -> (u64, u64) {
        if let Some((client_to_server, server_to_client)) = self.bytes_written.read().await.as_ref()
        {
            //.map_or((0, 0), |(a, b)| (a.borrow().clone(), b.borrow().clone()))
            (*client_to_server.borrow(), *server_to_client.borrow())
        } else {
            (0, 0)
        }
    }
}

impl PartialEq for TunnelConnection {
    fn eq(&self, other: &Self) -> bool {
        self.connection_id == other.connection_id && self.addr == other.addr
    }
}

impl Eq for TunnelConnection {}

impl Hash for TunnelConnection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.connection_id.hash(state);
        self.addr.hash(state);
    }
}
