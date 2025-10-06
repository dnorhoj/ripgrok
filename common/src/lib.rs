use serde::{Deserialize, Serialize};
use tokio::{fs::File, io::AsyncReadExt};

use crate::tunnel_specifier::{ResolvedTunnelSpecifier, TunnelSpecifier};

pub mod copy;
pub mod tunnel_specifier;

pub struct ClientHello;

impl ClientHello {
    pub const INIT_CONTROL_CONNECTION: u8 = 0x41;
    pub const INIT_TUNNEL_CONNECTION: u8 = 0x42;
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ClientControlHello {
    pub specifiers: Vec<TunnelSpecifier>,
    pub token: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerControlHello {
    Error {
        reason: String,
    },
    Listening {
        specifiers: Vec<ResolvedTunnelSpecifier>,
        public_host: Option<String>,
    },
}

impl ServerControlHello {
    pub fn error(e: impl Into<String>) -> Self {
        Self::Error { reason: e.into() }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ServerControlCommand {
    /// Server sends this to the client, to request the client to start a connection.
    StartTunnelConnection {
        connection_id: u64,
        server_port: u16,
    },
}

pub async fn get_random_u64() -> tokio::io::Result<u64> {
    // TODO; lmao
    let mut file = File::open("/dev/urandom").await?;
    Ok(file.read_u64().await?)
}
pub struct DropLogger<T> {
    name: String,
    inner: T,
}

impl<T> DropLogger<T> {
    pub fn new(val: T, name: impl Into<String>) -> Self {
        Self {
            inner: val,
            name: name.into(),
        }
    }
}

impl<T> Drop for DropLogger<T> {
    fn drop(&mut self) {
        println!("DROPPED {}", self.name)
    }
}

impl<T> std::ops::Deref for DropLogger<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> std::ops::DerefMut for DropLogger<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
