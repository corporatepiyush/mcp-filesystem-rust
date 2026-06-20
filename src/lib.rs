pub mod actions;
pub mod config;
pub mod errors;
pub mod http;
pub mod protocol;
pub mod server;
pub mod structures;
pub mod tls;
pub mod tools;
pub mod validation;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "MCP Filesystem Server")]
#[command(about = "High-performance Model Context Protocol server for filesystem access", long_about = None)]
pub struct Args {
    /// Directories to allow access to (can specify multiple)
    #[arg(short, long)]
    pub directories: Vec<String>,

    /// Server host
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    pub host: String,

    /// TCP server port
    #[arg(short = 'p', long, default_value = "3000")]
    pub port: u16,

    /// HTTP server port
    #[arg(long, default_value = "3001")]
    pub http_port: u16,

    /// Log level
    #[arg(short, long, default_value = "info")]
    pub log_level: String,

    /// Maximum file size in MB for read operations
    #[arg(long, default_value = "100")]
    pub max_file_size: u64,

    /// Maximum decompressed output size in MB (guards against decompression bombs)
    #[arg(long, default_value = "1024")]
    pub max_decompressed_size: u64,

    /// Run in stdio mode for MCP compatibility
    #[arg(long)]
    pub stdio: bool,

    /// Access mode: unrestricted or readonly
    #[arg(long, default_value = "unrestricted")]
    pub access_mode: config::AccessMode,

    /// Follow symbolic links
    #[arg(long)]
    pub follow_symlinks: bool,

    /// Request timeout in seconds
    #[arg(long, default_value = "30")]
    pub request_timeout: u64,

    /// Maximum size in bytes of a single JSON-RPC request line (TCP/stdio).
    /// Requests exceeding this are rejected to prevent memory exhaustion.
    #[arg(long, default_value = "16777216")]
    pub max_request_bytes: usize,

    /// Optional bearer token required to access the TCP and HTTP transports.
    /// When unset, the transports are unauthenticated.
    #[arg(long)]
    pub auth_token: Option<String>,

    /// Maximum number of concurrent TCP connections.
    #[arg(long, default_value = "1024")]
    pub max_connections: usize,

    /// Path to a PEM certificate chain to serve the HTTP transport over TLS
    /// (HTTPS). Requires --tls-key. Falls back to the MCP_TLS_CERT env var.
    /// When unset, the HTTP transport stays plaintext.
    #[arg(long)]
    pub tls_cert: Option<String>,

    /// Path to the PEM private key matching --tls-cert. Falls back to the
    /// MCP_TLS_KEY env var.
    #[arg(long)]
    pub tls_key: Option<String>,
}
