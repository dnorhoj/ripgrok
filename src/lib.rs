use serde::{Deserialize, Serialize};

use crate::tunnel_specifier::{ResolvedTunnelSpecifier, TunnelSpecifier};

pub mod tunnel_specifier;

pub struct ClientHello;

impl ClientHello {
    pub const INIT_CONTROL_CONNECTION: u8 = 0x41;
    pub const INIT_TUNNEL_CONNECTION: u8 = 0x42;
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ClientControlHello {
    pub tunnel_specifiers: Vec<TunnelSpecifier>,

    // Options
    pub ssl: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerControlHello {
    Goodbye {
        reason: String,
    },
    Listening {
        specifiers: Vec<ResolvedTunnelSpecifier>,
        public_host: Option<String>,
    },
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
