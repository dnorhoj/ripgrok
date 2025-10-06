use std::{fmt::Display, str::FromStr};

use ::serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TunnelType {
    Tcp,
    Ssl,
}

impl Display for TunnelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                TunnelType::Tcp => "tcp",
                TunnelType::Ssl => "ssl",
            }
        )
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct TunnelSpecifier {
    pub tunnel_type: TunnelType,
    pub client_port: u16,
    pub server_port: Option<u16>,
}

impl TunnelSpecifier {
    pub fn to_string(&self) -> String {
        if let Some(server_port) = self.server_port {
            format!("{}:{}", self.client_port, server_port)
        } else {
            format!("{}", self.client_port)
        }
    }
}

/*
    80 = Forward port 80 to a random port on the server
    80:8080 = Forward port 80 to port 8080 on the server
    :80 == 80:80
*/

impl FromStr for TunnelSpecifier {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<_> = s.split(':').collect();
        match parts.len() {
            1 => Ok(Self {
                tunnel_type: TunnelType::Tcp,
                client_port: parts[0].parse().map_err(|_| "Invalid port".to_string())?,
                server_port: None,
            }),
            2 => {
                let server_port = parts[1]
                    .parse()
                    .map_err(|_| "Invalid server port".to_string())?;

                let client_port = if parts[0].is_empty() {
                    server_port
                } else {
                    parts[0]
                        .parse()
                        .map_err(|_| "Invalid client port".to_string())?
                };

                Ok(Self {
                    tunnel_type: TunnelType::Tcp,
                    client_port,
                    server_port: Some(server_port),
                })
            }
            _ => Err("Invalid tunnel specifier format".to_string()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResolvedTunnelSpecifier {
    pub tunnel_type: TunnelType,
    pub client_port: u16,
    pub server_port: u16,
}
