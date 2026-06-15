pub mod actions;
pub mod config;
pub mod errors;
pub mod http;
pub mod protocol;
pub mod server;
pub mod structures;
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
}
