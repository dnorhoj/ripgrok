use std::sync::Arc;

use ::axum::Json;
use ::axum::body::Body;
use ::axum::extract::Request;
use ::axum::http::HeaderMap;
use ::axum::middleware::{self, Next};
use ::axum::response::{IntoResponse, Response};
use ::axum::routing::get;
use ::axum::{Router, extract::State};
use ::base64::Engine;
use ::base64::prelude::BASE64_STANDARD;
use ::futures::future::join_all;
use ::serde::Serialize;
use ::tokio::net::TcpListener;
use ripgrok_common::tunnel_specifier::ResolvedTunnelSpecifier;

use crate::config::WebServerAuthConfig;
use crate::{config::WebServerConfig, tunnel_server::TunnelServer};

#[derive(Clone)]
pub struct App {
    tunnel_server: Arc<TunnelServer>,
    config: Arc<WebServerConfig>,
}

pub struct WebServer {
    tunnel_server: Arc<TunnelServer>,
    config: Arc<WebServerConfig>,
}

async fn is_authenticated(request: &HeaderMap, auth_config: &WebServerAuthConfig) -> bool {
    let authorization = match request.get("Authorization") {
        Some(val) => val,
        None => return false,
    };

    let value = match authorization.to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };

    match value.split_once(" ") {
        Some((scheme, param)) => match (scheme.to_lowercase().as_str(), param) {
            ("basic", param) => match BASE64_STANDARD.decode(param) {
                Ok(res) => match String::from_utf8_lossy(&res).split_once(":") {
                    Some((username, password)) => {
                        username == auth_config.username && password == auth_config.password
                    }
                    None => false,
                },
                Err(_) => false,
            },
            _ => false,
        },
        None => false,
    }
}

async fn require_auth(State(app): State<App>, request: Request, next: Next) -> impl IntoResponse {
    if let Some(auth_config) = &app.config.auth {
        if !is_authenticated(&request.headers(), auth_config).await {
            return Response::builder()
                .status(401)
                .header(
                    "WWW-Authenticate",
                    r#"Basic realm="RipGrok Admin WebUI", charset="UTF-8""#,
                )
                .body(Body::from("Unauthorized"))
                .unwrap();
        }
    }

    next.run(request).await
}

#[derive(Serialize)]
struct ConnectionDTO {
    id: String,
    addr: String,
    tunnels: Vec<TunnelDTO>,
}

#[derive(Serialize)]
struct TunnelDTO {
    #[serde(flatten)]
    specifier: ResolvedTunnelSpecifier,
    connections: Vec<TunnelConnectionDTO>,
}
#[derive(Serialize)]
struct TunnelConnectionDTO {
    connection_id: String,
    addr: String,
    bytes_transfered: (u64, u64),
}

async fn get_connections(State(app): State<App>) -> impl IntoResponse {
    let connections = app.tunnel_server.get_control_connections();

    let mut res = Vec::new();

    for connection in connections {
        // Don't mind this clusterfuck
        res.push(ConnectionDTO {
            id: connection.id.to_string(),
            addr: connection.addr.to_string(),
            tunnels: join_all(
                connection
                    .get_listeners()
                    .await
                    .iter()
                    .map(async |tunnel_listener| TunnelDTO {
                        specifier: tunnel_listener.specifier,
                        connections: join_all(
                            tunnel_listener
                                .get_connections()
                                .iter()
                                .map(async |tunnel_connection| TunnelConnectionDTO {
                                    addr: tunnel_connection.addr.to_string(),
                                    connection_id: tunnel_connection.connection_id.to_string(),
                                    bytes_transfered: tunnel_connection
                                        .get_bytes_transfered()
                                        .await,
                                })
                                .collect::<Vec<_>>(),
                        )
                        .await,
                    })
                    .collect::<Vec<_>>(),
            )
            .await,
        });
    }

    Json(res)
}

impl WebServer {
    pub fn new(tunnel_server: Arc<TunnelServer>, config: WebServerConfig) -> Self {
        Self {
            tunnel_server,
            config: config.into(),
        }
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let listener = TcpListener::bind((self.config.host.clone(), self.config.port)).await?;

        let app = App {
            tunnel_server: self.tunnel_server.clone(),
            config: self.config.clone(),
        };

        let router = Router::new()
            .route("/api/connections", get(get_connections))
            .with_state(app.clone());

        let router = if self.config.auth.is_some() {
            router.layer(middleware::from_fn_with_state(app, require_auth))
        } else {
            router
        };

        tracing::info!("Web server listening on http://{}", listener.local_addr()?);

        Ok(axum::serve(listener, router).await?)
    }
}
