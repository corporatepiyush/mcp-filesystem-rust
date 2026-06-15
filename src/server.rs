use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, warn};

use crate::actions;
use crate::config::Config;
use crate::errors::{MCSError, Result as MCSResult};
use crate::protocol::{JsonRpcRequest, JsonRpcResponse};
use std::sync::LazyLock;

static TOOLS_LIST_RESPONSE: LazyLock<Vec<u8>> = LazyLock::new(|| {
    let tools_json = include_str!("../tools.json");
    let tools: Vec<Value> = serde_json::from_str(tools_json).expect("Failed to parse tools.json");
    let resp = json!({ "tools": tools });
    serde_json::to_vec(&resp).expect("Failed to serialize tools/list response")
});

const BUFFER_CAPACITY: usize = 65536;
const NEWLINE: &[u8] = b"\n";

fn parse_error(msg: String) -> JsonRpcResponse {
    let mcp_error = MCSError::ParseError(msg);
    JsonRpcResponse::error(None, mcp_error.error_code(), mcp_error.to_string())
}

fn parse_request(line: &str) -> std::result::Result<JsonRpcRequest, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err("Empty request".to_string());
    }
    serde_json::from_str::<JsonRpcRequest>(trimmed).map_err(|e| e.to_string())
}

pub struct MCPServer {
    config: Config,
}

impl MCPServer {
    pub const fn new(config: Config) -> Self {
        Self { config }
    }

    pub async fn run_stdio(&self) -> MCSResult<()> {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::with_capacity(BUFFER_CAPACITY, stdin);
        let mut stdout = tokio::io::stdout();
        let mut line = String::with_capacity(1024);
        let mut response_buf = Vec::with_capacity(65536);

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    process_one_line(&line, &self.config, &mut response_buf, &mut stdout).await?;
                }
                Err(e) => {
                    error!("IO error: {}", e);
                    break;
                }
            }
        }
        Ok(())
    }

    pub async fn run(&self) -> MCSResult<()> {
        let addr = format!("{}:{}", self.config.server.host, self.config.server.port);
        let listener = TcpListener::bind(&addr).await?;
        tracing::info!("MCP filesystem server listening on {}", addr);

        loop {
            let (socket, peer_addr) = listener.accept().await?;
            if let Err(e) = socket.set_nodelay(true) {
                warn!("Failed to set TCP_NODELAY: {}", e);
            }

            let config = self.config.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_client(socket, config).await {
                    error!("Client {} error: {}", peer_addr, e);
                }
            });
        }
    }
}

async fn handle_client(socket: TcpStream, config: Config) -> MCSResult<()> {
    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::with_capacity(BUFFER_CAPACITY, reader);
    let mut line = String::with_capacity(1024);
    let mut response_buf = Vec::with_capacity(65536);

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                process_one_line(&line, &config, &mut response_buf, &mut writer).await?;
            }
            Err(e) => {
                error!("IO error: {}", e);
                break;
            }
        }
    }
    Ok(())
}

async fn process_one_line<W: AsyncWriteExt + Unpin>(
    line: &str,
    config: &Config,
    response_buf: &mut Vec<u8>,
    writer: &mut W,
) -> MCSResult<()> {
    let (response, is_notification) = match parse_request(line) {
        Ok(req) => {
            let is_notif = req.id.is_none();
            match process_request(&req, config).await {
                Ok(result) => (JsonRpcResponse::success(req.id, result), is_notif),
                Err(e) => (JsonRpcResponse::error(req.id, e.error_code(), e.to_string()), is_notif),
            }
        }
        Err(e) => (parse_error(e), false),
    };

    if is_notification {
        return Ok(());
    }

    response_buf.clear();
    serde_json::to_writer(&mut *response_buf, &response)?;
    response_buf.extend_from_slice(NEWLINE);

    writer.write_all(response_buf).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn process_request(req: &JsonRpcRequest, config: &Config) -> MCSResult<Value> {
    match req.method.as_str() {
        "initialize" => handle_initialize(req),
        "tools/list" => handle_tools_list(),
        "tools/call" => handle_tools_call(req, config).await,
        "ping" => handle_ping(),
        method if method.starts_with("notifications/") => handle_notification(method),
        _ => Err(MCSError::MethodNotFound(req.method.clone())),
    }
}

const fn handle_ping() -> MCSResult<Value> {
    Ok(Value::Null)
}

fn handle_notification(method: &str) -> MCSResult<Value> {
    tracing::trace!("Received notification: {method}");
    Ok(Value::Null)
}

pub async fn process_request_http(req: &JsonRpcRequest, config: &Config) -> JsonRpcResponse {
    match process_request(req, config).await {
        Ok(result) => JsonRpcResponse::success(req.id.clone(), result),
        Err(e) => JsonRpcResponse::error(req.id.clone(), e.error_code(), e.to_string()),
    }
}

fn handle_initialize(_req: &JsonRpcRequest) -> MCSResult<Value> {
    static INIT_RESPONSE: LazyLock<Value> = LazyLock::new(|| {
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "subscribe": false, "listChanged": false },
                "prompts": { "listChanged": false }
            },
            "serverInfo": {
                "name": "mcp-filesystem",
                "version": env!("CARGO_PKG_VERSION")
            }
        })
    });
    Ok(INIT_RESPONSE.clone())
}

fn handle_tools_list() -> MCSResult<Value> {
    Ok(serde_json::from_slice(&TOOLS_LIST_RESPONSE)?)
}

async fn handle_tools_call(req: &JsonRpcRequest, config: &Config) -> MCSResult<Value> {
    let tool_name = req
        .params
        .as_ref()
        .and_then(|p| p.get("name").and_then(|v| v.as_str()))
        .ok_or_else(|| MCSError::InvalidParams("Missing 'name' parameter".into()))?;

    let tool_args = req.params.as_ref().and_then(|p| p.get("arguments"));

    if config.server.access_mode == crate::config::AccessMode::ReadOnly
        && crate::tools::is_write_tool(tool_name)
    {
        return Err(MCSError::InvalidParams(format!(
            "Operation '{tool_name}' is not allowed in read-only mode"
        )));
    }

    if !crate::tools::tool_exists(tool_name) {
        return Err(method_not_found(tool_name));
    }

    let result = match tool_name {
        "read_text_file" => actions::files::read_text_file(tool_args, config).await,
        "read_media_file" => actions::files::read_media_file(tool_args, config).await,
        "write_file" => actions::files::write_file(tool_args, config).await,
        "edit_file" => actions::files::edit_file(tool_args, config).await,
        "create_directory" => actions::files::create_directory(tool_args, config).await,
        "list_directory" => actions::files::list_directory(tool_args, config).await,
        "list_directory_with_sizes" => actions::files::list_directory_with_sizes(tool_args, config).await,
        "move_file" => actions::files::move_file(tool_args, config).await,
        "copy_file" => actions::files::copy_file(tool_args, config).await,
        "delete_file" => actions::files::delete_file(tool_args, config).await,
        "delete_directory" => actions::files::delete_directory(tool_args, config).await,
        "search_files" => actions::files::search_files(tool_args, config).await,
        "directory_tree" => actions::files::directory_tree(tool_args, config).await,
        "get_file_info" => actions::files::get_file_info(tool_args, config).await,
        "list_allowed_directories" => actions::files::list_allowed_directories(tool_args, config).await,
        "hash_file" => actions::files::hash_file(tool_args, config).await,
        "grep_files" => actions::files::grep_files(tool_args, config).await,
        "set_permissions" => actions::files::set_permissions(tool_args, config).await,
        "get_disk_usage" => actions::files::get_disk_usage(tool_args, config).await,
        "create_symlink" => actions::files::create_symlink(tool_args, config).await,
        "read_file_range" => actions::files::read_file_range(tool_args, config).await,
        "compress_gzip" => actions::files::compress_gzip(tool_args, config).await,
        "decompress_gzip" => actions::files::decompress_gzip(tool_args, config).await,
        "compress_zstd" => actions::files::compress_zstd(tool_args, config).await,
        "decompress_zstd" => actions::files::decompress_zstd(tool_args, config).await,
        "compress_tar" => actions::files::compress_tar(tool_args, config).await,
        "decompress_tar" => actions::files::decompress_tar(tool_args, config).await,
        "encrypt_file" => actions::crypto::encrypt_file(tool_args, config).await,
        "decrypt_file" => actions::crypto::decrypt_file(tool_args, config).await,
        "generate_key" => actions::crypto::generate_key(tool_args, config).await,
        "csv_create" => actions::csv::csv_create(tool_args, config).await,
        "csv_read" => actions::csv::csv_read(tool_args, config).await,
        "csv_add_row" => actions::csv::csv_add_row(tool_args, config).await,
        "csv_update_cell" => actions::csv::csv_update_cell(tool_args, config).await,
        "csv_remove_row" => actions::csv::csv_remove_row(tool_args, config).await,
        "csv_add_column" => actions::csv::csv_add_column(tool_args, config).await,
        "csv_remove_column" => actions::csv::csv_remove_column(tool_args, config).await,
        "csv_rename_column" => actions::csv::csv_rename_column(tool_args, config).await,
        "csv_read_column_values_range" => actions::csv::csv_read_column_values_range(tool_args, config).await,
        "csv_read_row_range" => actions::csv::csv_read_row_range(tool_args, config).await,
        tool => Err(method_not_found(tool)),
    };

    if let Err(ref e) = result {
        error!("Tool '{}' error: {:?}", tool_name, e);
    }
    result
}

fn method_not_found(name: &str) -> MCSError {
    MCSError::MethodNotFound(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_request() {
        let line = r#"{"jsonrpc":"2.0","method":"initialize","id":1}"#;
        let req = parse_request(line).unwrap();
        assert_eq!(req.method, "initialize");
    }

    #[test]
    fn test_parse_invalid_json() {
        let err = parse_request("{invalid}").unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn test_handle_initialize_response() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "initialize".to_string(),
            params: None,
            id: Some(Value::Number(1.into())),
        };
        let result = handle_initialize(&req).unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    }
}
