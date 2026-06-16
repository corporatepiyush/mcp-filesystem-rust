use anyhow::Result;
use clap::Parser;
use mcp_filesystem::{Args, config, http, server};
use std::sync::Arc;
use tracing::{info, warn};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Set mimalloc tuning env vars **before** any thread is spawned.
/// Called from `main()` before the tokio runtime starts.
/// # Safety
/// `set_var` is safe here because no other thread exists yet in `main()`.
fn set_mimalloc_opts() {
    // These are read by mimalloc at first allocation. Setting them before
    // any allocation (aside from the #[global_allocator] static itself) is
    // required. The tokio runtime is created *after* this returns.
    // SAFETY: we are in `main()` before spawning any threads.
    unsafe { std::env::set_var("MIMALLOC_PAGE_RESET", "0") };
    unsafe { std::env::set_var("MIMALLOC_DECOMMIT_DELAY", "1000") };
    unsafe { std::env::set_var("MIMALLOC_ARENA_EAGER_COMMIT", "1") };
    unsafe { std::env::set_var("MIMALLOC_LARGE_OS_PAGES", "1") };
    unsafe { std::env::set_var("MIMALLOC_EAGER_REGION_COMMIT", "1") };
    unsafe { std::env::set_var("MIMALLOC_RESET_DELAY", "0") };
}

fn main() -> Result<()> {
    set_mimalloc_opts();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        inner_main().await
    })
}

async fn inner_main() -> Result<()> {
    let args = Args::parse();

    init_tracing(&args.log_level)?;

    info!("Starting MCP Filesystem Server");
    info!("Version: {}", env!("CARGO_PKG_VERSION"));

    let config = Arc::new(config::Config::from_args(&args)?);
    info!("Allowed directories: {:?}", config.allowed_directories);
    info!("Access mode: {:?}", config.server.access_mode);

    let mcp_server = server::MCPServer::from_arc(Arc::clone(&config));
    info!("Server initialized successfully");

    if !is_loopback_host(&config.server.host) && config.server.auth_token.is_none() && !args.stdio {
        warn!(
            "Binding to non-loopback host '{}' WITHOUT authentication — all allowed directories are exposed to the network. Set --auth-token to require a bearer token.",
            config.server.host
        );
    }

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

        let http_config = Arc::clone(&config);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_mimalloc_opts_does_not_panic() {
        set_mimalloc_opts();
    }
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1" | "localhost") || host.starts_with("127.")
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
