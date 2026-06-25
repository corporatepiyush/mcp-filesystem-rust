//! MCP 2025-11-25 spec-compliance tests, driven end-to-end through
//! `process_request` (the same path stdio/TCP/HTTP use).

use mcp_filesystem::config::{AccessMode, Config, ServerConfig};
use mcp_filesystem::protocol::JsonRpcRequest;
use mcp_filesystem::server::process_request;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

fn unique_dir() -> PathBuf {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test_tmp")
        .join(format!(
            "compliance_{}_{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn config_for(dir: &PathBuf, mode: AccessMode) -> Config {
    Config::new(
        vec![dir.to_string_lossy().to_string()],
        ServerConfig {
            host: "127.0.0.1".into(),
            http_port: 0,
            request_timeout: std::time::Duration::from_secs(5),
            access_mode: mode,
            follow_symlinks: false,
            max_request_bytes: 16 * 1024 * 1024,
            auth_token: None,
            enabled_categories: mcp_filesystem::tools::ToolCategory::ALL.to_vec(),
            tls_cert: None,
            tls_key: None,
        },
        100 * 1024 * 1024,
    )
}

async fn call(config: &Config, method: &str, params: Value) -> Result<Value, String> {
    let req = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        method: method.into(),
        params: Some(params),
        id: Some(json!(1)),
    };
    process_request(&req, config).await.map_err(|e| e.to_string())
}

async fn tool_call(config: &Config, name: &str, args: Value) -> Result<Value, String> {
    call(config, "tools/call", json!({ "name": name, "arguments": args })).await
}

/// A valid `CallToolResult` has a non-empty `content` array of typed items and a
/// boolean `isError`.
fn assert_valid_call_tool_result(v: &Value) {
    let content = v["content"].as_array().expect("content is an array");
    assert!(!content.is_empty(), "content must be non-empty: {v}");
    assert!(content[0]["type"].is_string(), "content item needs a type: {v}");
    assert!(v["isError"].is_boolean(), "isError must be a bool: {v}");
}

#[tokio::test]
async fn initialize_negotiates_supported_versions() {
    let dir = unique_dir();
    let config = config_for(&dir, AccessMode::Unrestricted);
    for v in ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"] {
        let res = call(&config, "initialize", json!({ "protocolVersion": v }))
            .await
            .unwrap();
        assert_eq!(res["protocolVersion"], v, "{v} should be echoed");
    }
}

#[tokio::test]
async fn initialize_falls_back_and_is_honest() {
    let dir = unique_dir();
    let config = config_for(&dir, AccessMode::Unrestricted);

    let res = call(&config, "initialize", json!({ "protocolVersion": "1900-01-01" }))
        .await
        .unwrap();
    assert_eq!(res["protocolVersion"], "2025-11-25");
    assert!(res["instructions"].as_str().is_some_and(|s| !s.is_empty()));

    // No falsely-advertised capabilities.
    let caps = &res["capabilities"];
    assert!(caps["tools"].is_object());
    assert!(caps["resources"].is_null());
    assert!(caps["prompts"].is_null());
}

#[tokio::test]
async fn tool_success_returns_structured_call_tool_result() {
    let dir = unique_dir();
    let config = config_for(&dir, AccessMode::Unrestricted);
    fs::write(dir.join("hello.txt"), "hi there").unwrap();

    let res = tool_call(
        &config,
        "read_text_file",
        json!({ "path": dir.join("hello.txt").to_string_lossy() }),
    )
    .await
    .unwrap();

    assert_valid_call_tool_result(&res);
    assert_eq!(res["isError"], false);
    // Payload is preserved under structuredContent.
    assert_eq!(res["structuredContent"]["content"], "hi there");
}

#[tokio::test]
async fn tool_failure_is_iserror_not_protocol_error() {
    let dir = unique_dir();
    let config = config_for(&dir, AccessMode::Unrestricted);

    // Reading a missing file is an execution failure, not a protocol error.
    let res = tool_call(
        &config,
        "read_text_file",
        json!({ "path": dir.join("does_not_exist.txt").to_string_lossy() }),
    )
    .await
    .expect("should be Ok(CallToolResult), not Err");

    assert_valid_call_tool_result(&res);
    assert_eq!(res["isError"], true);
}

#[tokio::test]
async fn read_only_policy_rejection_is_iserror() {
    let dir = unique_dir();
    let config = config_for(&dir, AccessMode::ReadOnly);

    let res = tool_call(
        &config,
        "write_file",
        json!({ "path": dir.join("x.txt").to_string_lossy(), "content": "nope" }),
    )
    .await
    .expect("policy rejection should be a CallToolResult, not Err");

    assert_eq!(res["isError"], true);
    assert!(res["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("read-only"));
}

#[tokio::test]
async fn read_media_file_returns_typed_image_content() {
    let dir = unique_dir();
    let config = config_for(&dir, AccessMode::Unrestricted);
    // Minimal valid PNG header so the magic-byte sniffer detects image/png.
    let png: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89,
    ];
    fs::write(dir.join("pixel.png"), png).unwrap();

    let res = tool_call(
        &config,
        "read_media_file",
        json!({ "path": dir.join("pixel.png").to_string_lossy() }),
    )
    .await
    .unwrap();

    assert_eq!(res["isError"], false);
    assert_eq!(res["content"][0]["type"], "image");
    assert_eq!(res["content"][0]["mimeType"], "image/png");
    assert!(res["content"][0]["data"].as_str().is_some_and(|d| !d.is_empty()));
}

#[tokio::test]
async fn protocol_errors_stay_protocol_errors() {
    let dir = unique_dir();
    let config = config_for(&dir, AccessMode::Unrestricted);

    // Missing `name` → JSON-RPC protocol error (Err), not an isError result.
    assert!(call(&config, "tools/call", json!({})).await.is_err());

    // Unknown tool → protocol error.
    assert!(tool_call(&config, "no_such_tool", json!({})).await.is_err());

    // Unknown method → protocol error.
    assert!(call(&config, "no/such/method", json!({})).await.is_err());
}
