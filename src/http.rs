use axum::{
    Json, Router,
    extract::State,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::debug;

use crate::config::Config;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse};

#[derive(Clone)]
pub struct HttpState {
    pub config: Arc<Config>,
}

pub async fn create_http_server(config: Config, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let http_state = HttpState {
        config: Arc::new(config.clone()),
    };

    let app = Router::new()
        .route("/rpc", axum::routing::post(handle_rpc))
        .route("/health", axum::routing::get(handle_health))
        .with_state(http_state);

    let addr = format!("{}:{}", config.server.host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("HTTP server listening on {}", addr);

    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle_rpc(
    State(state): State<HttpState>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    debug!("HTTP RPC request: {:?}", req.method);
    let response = crate::server::process_request_http(&req, &state.config).await;
    Json(response)
}

async fn handle_health() -> Json<Value> {
    Json(json!({
        "status": "healthy",
        "service": "mcp-filesystem",
        "version": env!("CARGO_PKG_VERSION")
    }))
}
