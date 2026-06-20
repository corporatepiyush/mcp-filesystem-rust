//! Server-side TLS (HTTPS) for the HTTP transport.
//!
//! TLS is opt-in: the HTTP server only serves over TLS when both a
//! `--tls-cert` and `--tls-key` are configured, otherwise it stays plaintext
//! (the default). Backed by rustls with the `ring` crypto provider — no
//! OpenSSL/aws-lc, matching the rest of the dependency graph.

/// Install the rustls `ring` crypto provider as the process default.
///
/// Idempotent — only the first install in the process wins; later calls are
/// ignored. The rustls `ServerConfig` builder reads this process default, so it
/// must be installed before any TLS config is built.
pub fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Build an axum-server rustls config from a PEM certificate chain and private
/// key. Installs the `ring` provider on first call.
pub async fn server_config(
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> anyhow::Result<axum_server::tls_rustls::RustlsConfig> {
    ensure_crypto_provider();
    axum_server::tls_rustls::RustlsConfig::from_pem_file(cert_path, key_path)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to load TLS certificate '{}' and key '{}': {e}",
                cert_path.display(),
                key_path.display()
            )
        })
}
