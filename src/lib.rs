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

    /// Server host (used by the HTTP transport)
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    pub host: String,

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

    /// Maximum size in bytes of a single JSON-RPC request line (stdio).
    /// Requests exceeding this are rejected to prevent memory exhaustion.
    #[arg(long, default_value = "16777216")]
    pub max_request_bytes: usize,

    /// Optional bearer token required to access the HTTP transport.
    /// When unset, the transport is unauthenticated.
    #[arg(long)]
    pub auth_token: Option<String>,

    /// Path to a PEM certificate chain to serve the HTTP transport over TLS
    /// (HTTPS). Requires --tls-key. Falls back to the MCP_TLS_CERT env var.
    /// When unset, the HTTP transport stays plaintext.
    #[arg(long)]
    pub tls_cert: Option<String>,

    /// Path to the PEM private key matching --tls-cert. Falls back to the
    /// MCP_TLS_KEY env var.
    #[arg(long)]
    pub tls_key: Option<String>,

    // ── Tool exposure ────────────────────────────────────────────────────
    // No tools are exposed unless explicitly enabled. Each flag turns on one
    // category (hidden from tools/list and rejected from tools/call when its
    // category is disabled). Use --enable-all for every category at once.
    /// Expose ALL tool categories (overrides the individual flags).
    #[arg(long)]
    pub enable_all: bool,

    /// Enable Read tools: read files, list/search/stat, hashes, disk usage.
    #[arg(long)]
    pub enable_read: bool,

    /// Enable Write tools: write/edit, create dir, move/copy, perms, symlink.
    #[arg(long)]
    pub enable_write: bool,

    /// Enable Delete tools: delete file/directory.
    #[arg(long)]
    pub enable_delete: bool,

    /// Enable Compress tools: gzip, zstd, tar (de)compression.
    #[arg(long)]
    pub enable_compress: bool,

    /// Enable Crypto tools: encrypt/decrypt files and key generation.
    #[arg(long)]
    pub enable_crypto: bool,

    /// Enable CSV tools: CSV read/write helpers.
    #[arg(long)]
    pub enable_csv: bool,
}

impl Args {
    /// Resolve the set of enabled tool categories from the `--enable-*` flags.
    /// `--enable-all` turns on every category; otherwise only the categories
    /// whose individual flag is set. With no flags, the result is empty and no
    /// tools are exposed.
    pub fn enabled_categories(&self) -> Vec<tools::ToolCategory> {
        use tools::ToolCategory as C;
        if self.enable_all {
            return C::ALL.to_vec();
        }
        let mut cats = Vec::new();
        let mut push = |on: bool, cat: C| {
            if on {
                cats.push(cat);
            }
        };
        push(self.enable_read, C::Read);
        push(self.enable_write, C::Write);
        push(self.enable_delete, C::Delete);
        push(self.enable_compress, C::Compress);
        push(self.enable_crypto, C::Crypto);
        push(self.enable_csv, C::Csv);
        cats
    }
}
