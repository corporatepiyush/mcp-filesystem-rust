use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::debug;

use crate::config::Config;
use crate::protocol::JsonRpcRequest;

#[derive(Clone)]
pub struct HttpState {
    pub config: Arc<Config>,
}

pub async fn create_http_server(
    config: Arc<Config>,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let host = config.server.host.clone();
    let tls_cert = config.server.tls_cert.clone();
    let tls_key = config.server.tls_key.clone();
    let http_state = HttpState { config };

    let app = Router::new()
        .route("/rpc", axum::routing::post(handle_rpc))
        .route("/health", axum::routing::get(handle_health))
        .with_state(http_state);

    let addr = format!("{host}:{port}");

    // Serve over TLS (HTTPS) when a cert+key are configured, otherwise plaintext.
    // `config.rs` guarantees the two are set together.
    if let (Some(cert), Some(key)) = (tls_cert, tls_key) {
        let tls = crate::tls::server_config(&cert, &key).await?;
        let socket_addr = resolve_addr(&addr)?;
        tracing::info!("HTTPS server listening on {} (TLS)", socket_addr);
        axum_server::bind_rustls(socket_addr, tls)
            .serve(app.into_make_service())
            .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!("HTTP server listening on {}", addr);
        axum::serve(listener, app).await?;
    }
    Ok(())
}

/// Resolve a `host:port` string to a single `SocketAddr` for `axum_server`,
/// which binds an address rather than an already-bound listener.
fn resolve_addr(addr: &str) -> Result<std::net::SocketAddr, Box<dyn std::error::Error>> {
    use std::net::ToSocketAddrs;
    addr.to_socket_addrs()?
        .next()
        .ok_or_else(|| format!("could not resolve bind address '{addr}'").into())
}

async fn handle_rpc(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    if let Some(expected) = state.config.server.auth_token.as_deref() {
        let presented = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !crate::server::token_matches(presented, expected) {
            return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        }
    }
    debug!("HTTP RPC request: {:?}", req.method);
    let response = crate::server::process_request_http(&req, &state.config).await;
    Json(response).into_response()
}

async fn handle_health() -> Json<Value> {
    Json(json!({
        "status": "healthy",
        "service": "mcp-filesystem",
        "version": env!("CARGO_PKG_VERSION")
    }))
}
