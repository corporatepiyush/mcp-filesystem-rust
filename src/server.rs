use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, warn};

use crate::actions;
use crate::config::Config;
use crate::errors::{MCSError, Result as MCSResult};
use crate::protocol::{JsonRpcRequest, JsonRpcResponse};
use std::sync::Arc;
use std::sync::LazyLock;
use tokio::sync::Semaphore;

static TOOLS_LIST_RESPONSE: LazyLock<Vec<u8>> = LazyLock::new(|| {
    let tools_json = include_str!("../tools.json");
    let tools: Vec<Value> = serde_json::from_str(tools_json).expect("Failed to parse tools.json");
    let resp = json!({ "tools": tools });
    serde_json::to_vec(&resp).expect("Failed to serialize tools/list response")
});

const BUFFER_CAPACITY: usize = 65536;
const NEWLINE: &[u8] = b"\n";

/// Outcome of attempting to read one newline-delimited request.
enum LineRead {
    /// A complete line was read into the buffer.
    Line,
    /// Clean EOF before any bytes of a new line.
    Eof,
    /// The line exceeded `max_request_bytes` before a newline was seen.
    TooLong,
}

/// Read a single newline-terminated line into `out`, but never buffer more than
/// `max` bytes. On overflow returns `TooLong` without allocating the whole line,
/// so a hostile client cannot exhaust memory with one giant unterminated line.
async fn read_line_capped<R>(
    reader: &mut R,
    out: &mut String,
    max: usize,
) -> std::io::Result<LineRead>
where
    R: AsyncBufReadExt + Unpin,
{
    out.clear();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            // EOF: a trailing line without a newline is still a valid request.
            if buf.is_empty() {
                return Ok(LineRead::Eof);
            }
            *out = String::from_utf8_lossy(&buf).into_owned();
            return Ok(LineRead::Line);
        }
        match available.iter().position(|&b| b == b'\n') {
            Some(i) => {
                if buf.len() + i + 1 > max {
                    reader.consume(i + 1);
                    return Ok(LineRead::TooLong);
                }
                buf.extend_from_slice(&available[..=i]);
                reader.consume(i + 1);
                *out = String::from_utf8_lossy(&buf).into_owned();
                return Ok(LineRead::Line);
            }
            None => {
                let take = available.len();
                if buf.len() + take > max {
                    reader.consume(take);
                    return Ok(LineRead::TooLong);
                }
                buf.extend_from_slice(available);
                reader.consume(take);
            }
        }
    }
}

/// Check a presented credential line against the expected token. Accepts an
/// optional `Bearer ` prefix. Uses a length-then-byte comparison that does not
/// early-return on the first differing byte.
pub fn token_matches(presented: &str, expected: &str) -> bool {
    let presented = presented.trim();
    let presented = presented
        .strip_prefix("Bearer ")
        .unwrap_or(presented)
        .trim();

    // Use hashes to ensure constant-time comparison even if lengths differ.
    let h_presented = Sha256::digest(presented.as_bytes());
    let h_expected = Sha256::digest(expected.as_bytes());

    h_presented.ct_eq(&h_expected).into()
}

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
    config: Arc<Config>,
}

impl MCPServer {
    pub fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    pub const fn from_arc(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn run_stdio(&self) -> MCSResult<()> {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::with_capacity(BUFFER_CAPACITY, stdin);
        let mut stdout = tokio::io::stdout();
        let mut line = String::with_capacity(1024);
        let mut response_buf = Vec::with_capacity(65536);
        let max = self.config.server.max_request_bytes;

        loop {
            match read_line_capped(&mut reader, &mut line, max).await {
                Ok(LineRead::Eof) => break,
                Ok(LineRead::Line) => {
                    process_one_line(&line, &self.config, &mut response_buf, &mut stdout).await?;
                }
                Ok(LineRead::TooLong) => {
                    write_oversize_error(&mut response_buf, &mut stdout, max).await?;
                    break;
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

        // Bound concurrent connections to prevent a connection-flood DoS.
        let limiter = Arc::new(Semaphore::new(self.config.server.max_connections));

        loop {
            // Acquire a permit before accepting so we apply backpressure.
            let permit = Arc::clone(&limiter)
                .acquire_owned()
                .await
                .expect("connection semaphore closed");
            let (socket, peer_addr) = listener.accept().await?;
            if let Err(e) = socket.set_nodelay(true) {
                warn!("Failed to set TCP_NODELAY: {}", e);
            }

            let config = Arc::clone(&self.config);
            tokio::spawn(async move {
                if let Err(e) = handle_client(socket, config).await {
                    error!("Client {} error: {}", peer_addr, e);
                }
                drop(permit);
            });
        }
    }
}

async fn handle_client(socket: TcpStream, config: Arc<Config>) -> MCSResult<()> {
    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::with_capacity(BUFFER_CAPACITY, reader);
    let mut line = String::with_capacity(1024);
    let mut response_buf = Vec::with_capacity(65536);
    let max = config.server.max_request_bytes;

    // When an auth token is configured, require the first line to present it
    // before any request is processed.
    if let Some(expected) = config.server.auth_token.as_deref() {
        match read_line_capped(&mut reader, &mut line, max).await {
            Ok(LineRead::Line) if token_matches(&line, expected) => {}
            Ok(LineRead::Eof) => return Ok(()),
            _ => {
                let err = MCSError::InvalidParams(
                    "Authentication required: send the bearer token as the first line".into(),
                );
                let response = JsonRpcResponse::error(None, err.error_code(), err.to_string());
                response_buf.clear();
                serde_json::to_writer(&mut response_buf, &response)?;
                response_buf.extend_from_slice(NEWLINE);
                writer.write_all(&response_buf).await?;
                writer.flush().await?;
                return Ok(());
            }
        }
    }

    loop {
        match read_line_capped(&mut reader, &mut line, max).await {
            Ok(LineRead::Eof) => break,
            Ok(LineRead::Line) => {
                process_one_line(&line, &config, &mut response_buf, &mut writer).await?;
            }
            Ok(LineRead::TooLong) => {
                write_oversize_error(&mut response_buf, &mut writer, max).await?;
                break;
            }
            Err(e) => {
                error!("IO error: {}", e);
                break;
            }
        }
    }
    Ok(())
}

async fn write_oversize_error<W: AsyncWriteExt + Unpin>(
    response_buf: &mut Vec<u8>,
    writer: &mut W,
    max: usize,
) -> MCSResult<()> {
    let err = MCSError::InvalidParams(format!("Request exceeds maximum size of {max} bytes"));
    let response = JsonRpcResponse::error(None, err.error_code(), err.to_string());
    response_buf.clear();
    serde_json::to_writer(&mut *response_buf, &response)?;
    response_buf.extend_from_slice(NEWLINE);
    writer.write_all(response_buf).await?;
    writer.flush().await?;
    Ok(())
}

async fn process_one_line<W: AsyncWriteExt + Unpin>(
    line: &str,
    config: &Arc<Config>,
    response_buf: &mut Vec<u8>,
    writer: &mut W,
) -> MCSResult<()> {
    let (response, is_notification) = match parse_request(line) {
        Ok(req) => {
            let is_notif = req.id.is_none();
            match tokio::time::timeout(config.server.request_timeout, process_request(&req, config))
                .await
            {
                Ok(Ok(result)) => (JsonRpcResponse::success(req.id, result), is_notif),
                Ok(Err(e)) => (
                    JsonRpcResponse::error(req.id, e.error_code(), e.to_string()),
                    is_notif,
                ),
                Err(_) => {
                    let e = timeout_error(config);
                    (
                        JsonRpcResponse::error(req.id, e.error_code(), e.to_string()),
                        is_notif,
                    )
                }
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
    match tokio::time::timeout(config.server.request_timeout, process_request(req, config)).await {
        Ok(Ok(result)) => JsonRpcResponse::success(req.id.clone(), result),
        Ok(Err(e)) => JsonRpcResponse::error(req.id.clone(), e.error_code(), e.to_string()),
        Err(_) => {
            let e = timeout_error(config);
            JsonRpcResponse::error(req.id.clone(), e.error_code(), e.to_string())
        }
    }
}

fn timeout_error(config: &Config) -> MCSError {
    MCSError::FilesystemError(format!(
        "Request timed out after {}s",
        config.server.request_timeout.as_secs()
    ))
}

/// MCP protocol revisions this server can speak, newest first (for `initialize`
/// version negotiation).
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] =
    &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// Newest revision we implement; offered when the client requests an unknown one.
const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";

/// `instructions` surfaced to the client and appended to the model's system prompt.
const SERVER_INSTRUCTIONS: &str = "Sandboxed filesystem MCP server. All paths are restricted to the \
allowed directories; use `list_allowed_directories` to see them. Call `create_directory` before \
writing into a new path. Tool results carry both human-readable text and a machine-readable \
`structuredContent` object; `read_media_file` returns base64 image/binary content. Tool failures are \
returned with `isError: true` rather than as protocol errors — read the message and retry.";

fn handle_initialize(req: &JsonRpcRequest) -> MCSResult<Value> {
    // Version negotiation: echo a supported requested revision, else offer latest.
    let protocol_version = req
        .params
        .as_ref()
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str)
        .filter(|v| SUPPORTED_PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(LATEST_PROTOCOL_VERSION);

    Ok(json!({
        "protocolVersion": protocol_version,
        // Advertise only implemented capabilities. Earlier releases falsely
        // declared `resources` and `prompts` with no handlers.
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": {
            "name": "mcp-filesystem",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": SERVER_INSTRUCTIONS
    }))
}

/// Wrap a successful tool result in an MCP `CallToolResult`. If the action
/// already produced a `CallToolResult` (an object with a `content` array — e.g.
/// `read_media_file` returning `ImageContent`), it is passed through with
/// `isError` defaulted. Otherwise the value is provided as serialized text plus,
/// for objects, `structuredContent` (MCP 2025-06-18+).
#[inline]
fn tool_success(value: Value) -> Value {
    if value.get("content").is_some_and(Value::is_array) {
        let mut v = value;
        if v.get("isError").is_none() {
            v["isError"] = Value::Bool(false);
        }
        return v;
    }
    let text = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
    if value.is_object() {
        json!({
            "content": [{ "type": "text", "text": text }],
            "structuredContent": value,
            "isError": false
        })
    } else {
        json!({
            "content": [{ "type": "text", "text": text }],
            "isError": false
        })
    }
}

/// Wrap a tool execution failure as a `CallToolResult` with `isError: true` so
/// the model can read the message and self-correct instead of receiving an
/// opaque JSON-RPC protocol error.
#[inline]
fn tool_error(message: impl Into<String>) -> Value {
    json!({
        "content": [{ "type": "text", "text": message.into() }],
        "isError": true
    })
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
        // Policy rejection of a valid call → surface as an isError result.
        return Ok(tool_error(format!(
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
        "list_directory_with_sizes" => {
            actions::files::list_directory_with_sizes(tool_args, config).await
        }
        "move_file" => actions::files::move_file(tool_args, config).await,
        "copy_file" => actions::files::copy_file(tool_args, config).await,
        "delete_file" => actions::files::delete_file(tool_args, config).await,
        "delete_directory" => actions::files::delete_directory(tool_args, config).await,
        "search_files" => actions::files::search_files(tool_args, config).await,
        "directory_tree" => actions::files::directory_tree(tool_args, config).await,
        "get_file_info" => actions::files::get_file_info(tool_args, config).await,
        "list_allowed_directories" => {
            actions::files::list_allowed_directories(tool_args, config).await
        }
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
        "csv_read_column_values_range" => {
            actions::csv::csv_read_column_values_range(tool_args, config).await
        }
        "csv_read_row_range" => actions::csv::csv_read_row_range(tool_args, config).await,
        "csv_select_column_range" => actions::csv::csv_select_column_range(tool_args, config).await,
        tool => Err(method_not_found(tool)),
    };

    // Wrap in an MCP `CallToolResult`. Execution failures become isError
    // results so the model can self-correct, not opaque protocol errors.
    match result {
        Ok(value) => Ok(tool_success(value)),
        Err(e) => {
            error!("Tool '{tool_name}' error: {e:?}");
            Ok(tool_error(e.to_string()))
        }
    }
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
    fn test_token_matches() {
        assert!(token_matches("secret", "secret"));
        assert!(token_matches("Bearer secret", "secret"));
        assert!(token_matches("  Bearer secret  ", "secret"));
        assert!(!token_matches("wrong", "secret"));
        assert!(!token_matches("secre", "secret"));
        assert!(!token_matches("", "secret"));
    }

    #[tokio::test]
    async fn test_read_line_capped_rejects_oversize() {
        // A line longer than the cap, without a newline, must be rejected.
        let data = vec![b'a'; 1024];
        let mut reader = tokio::io::BufReader::new(&data[..]);
        let mut line = String::new();
        let res = read_line_capped(&mut reader, &mut line, 100).await.unwrap();
        assert!(matches!(res, LineRead::TooLong));
    }

    #[tokio::test]
    async fn test_read_line_capped_reads_line() {
        let data = b"hello\nworld\n";
        let mut reader = tokio::io::BufReader::new(&data[..]);
        let mut line = String::new();
        let res = read_line_capped(&mut reader, &mut line, 100).await.unwrap();
        assert!(matches!(res, LineRead::Line));
        assert_eq!(line, "hello\n");
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
        assert_eq!(result["protocolVersion"], LATEST_PROTOCOL_VERSION);
        // False resources/prompts capabilities must not be advertised.
        assert!(result["capabilities"]["resources"].is_null());
        assert!(result["capabilities"]["prompts"].is_null());
        assert!(result["instructions"].is_string());
        assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_handle_initialize_version_negotiation() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "initialize".to_string(),
            params: Some(json!({ "protocolVersion": "2024-11-05" })),
            id: Some(Value::Number(1.into())),
        };
        assert_eq!(
            handle_initialize(&req).unwrap()["protocolVersion"],
            "2024-11-05"
        );
    }

    #[test]
    fn test_tool_result_wrapping() {
        let ok = tool_success(json!({ "ok": 1 }));
        assert_eq!(ok["isError"], false);
        assert_eq!(ok["content"][0]["type"], "text");
        assert_eq!(ok["structuredContent"]["ok"], 1);

        // A ready-made CallToolResult (content array) is passed through.
        let img = tool_success(json!({ "content": [{ "type": "image", "data": "x", "mimeType": "image/png" }] }));
        assert_eq!(img["content"][0]["type"], "image");
        assert_eq!(img["isError"], false);

        let err = tool_error("nope");
        assert_eq!(err["isError"], true);
        assert_eq!(err["content"][0]["text"], "nope");
    }
}
