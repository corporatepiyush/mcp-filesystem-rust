use anyhow::Result;
use clap::Parser;
use mcp_filesystem::{Args, config, http, server};
use tracing::info;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<()> {
    unsafe { std::env::set_var("MIMALLOC_PAGE_RESET", "0") };
    unsafe { std::env::set_var("MIMALLOC_DECOMMIT_DELAY", "1000") };
    unsafe { std::env::set_var("MIMALLOC_ARENA_EAGER_COMMIT", "1") };
    unsafe { std::env::set_var("MIMALLOC_LARGE_OS_PAGES", "1") };
    unsafe { std::env::set_var("MIMALLOC_EAGER_REGION_COMMIT", "1") };
    unsafe { std::env::set_var("MIMALLOC_RESET_DELAY", "0") };

    let args = Args::parse();

    init_tracing(&args.log_level)?;

    info!("Starting MCP Filesystem Server");
    info!("Version: {}", env!("CARGO_PKG_VERSION"));

    let config = config::Config::from_args(&args)?;
    info!("Allowed directories: {:?}", config.allowed_directories);
    info!("Access mode: {:?}", config.server.access_mode);

    let mcp_server = server::MCPServer::new(config.clone());
    info!("Server initialized successfully");

    if args.stdio {
        info!("Running in stdio mode");
        mcp_server.run_stdio().await?;
    } else {
        info!("Starting TCP server on port {}", args.port);
        info!("Starting HTTP server on port {}", args.http_port);

        let tcp_handle = tokio::spawn(async move {
            if let Err(e) = mcp_server.run().await {
                eprintln!("TCP server error: {}", e);
            }
        });

        let http_config = config.clone();
        let http_port = args.http_port;
        let http_handle = tokio::spawn(async move {
            if let Err(e) = http::create_http_server(http_config, http_port).await {
                eprintln!("HTTP server error: {}", e);
            }
        });

        tokio::select! {
            _ = tcp_handle => info!("TCP server exited"),
            _ = http_handle => info!("HTTP server exited"),
        }
    }

    info!("Server shutdown complete");
    Ok(())
}

fn init_tracing(log_level: &str) -> Result<()> {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();

    Ok(())
}
