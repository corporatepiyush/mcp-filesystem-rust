use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::structures::PathTrie;
use crate::validation::Sandbox;
pub use crate::tools::ToolCategory;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AccessMode {
    #[serde(rename = "unrestricted")]
    Unrestricted,
    #[serde(rename = "readonly")]
    ReadOnly,
}

impl fmt::Display for AccessMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccessMode::Unrestricted => write!(f, "unrestricted"),
            AccessMode::ReadOnly => write!(f, "readonly"),
        }
    }
}

impl FromStr for AccessMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "unrestricted" => Ok(AccessMode::Unrestricted),
            "readonly" | "read-only" => Ok(AccessMode::ReadOnly),
            _ => Err(format!(
                "Invalid access mode: {s}. Use 'unrestricted' or 'readonly'"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub allowed_directories: Vec<String>,
    pub server: ServerConfig,
    pub max_file_size: u64,
    /// Cap on decompression/extraction output size, in bytes.
    pub max_decompressed_size: u64,
    /// Precomputed, canonicalized trie of `allowed_directories`.
    /// Built once at construction time; never serialized. Rebuild with
    /// `Config::rebuild_trie` if `allowed_directories` is mutated.
    #[serde(skip)]
    allowed_trie: Arc<PathTrie>,
    /// Capability-backed sandbox for all filesystem operations.
    /// Lazily initialised on first use; never serialized.
    #[serde(skip)]
    sandbox: OnceLock<Arc<Sandbox>>,
    /// Pre-serialized `{"tools":[...]}` payload for `tools/list`, filtered to
    /// the enabled categories (see `server.enabled_categories`). Skipped during
    /// (de)serialization and rebuilt from the enabled set.
    #[serde(skip, default = "default_tools_list_bytes")]
    pub tools_list_bytes: Arc<Vec<u8>>,
}

fn default_tools_list_bytes() -> Arc<Vec<u8>> {
    Arc::new(crate::server::build_tools_list_response(&[]))
}

/// Build a canonicalized `PathTrie` from a list of allowed directory strings.
/// Canonicalizes each allowed directory before inserting; falls back to the
/// absolute (non-canonical) path when canonicalization fails.
fn build_allowed_trie(allowed_dirs: &[String]) -> PathTrie {
    let mut trie = PathTrie::new();
    for dir_str in allowed_dirs {
        let p = Path::new(dir_str);
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(p)
        };
        if let Ok(canonical) = abs.canonicalize() {
            trie.insert(&canonical);
        } else {
            trie.insert(&abs);
        }
    }
    trie
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub http_port: u16,
    pub request_timeout: Duration,
    pub access_mode: AccessMode,
    pub follow_symlinks: bool,
    /// Max bytes for a single request line on the stdio transport.
    pub max_request_bytes: usize,
    /// Optional bearer token for HTTP authentication.
    pub auth_token: Option<String>,
    /// Tool categories exposed by this server. Empty (the default) means no
    /// tools are advertised or callable until enabled with `--enable-*`.
    #[serde(default)]
    pub enabled_categories: Vec<ToolCategory>,
    /// PEM certificate chain for serving the HTTP transport over TLS (HTTPS).
    /// `None` (the default) keeps the HTTP transport plaintext. Engaged only
    /// when both `tls_cert` and `tls_key` are set.
    #[serde(default)]
    pub tls_cert: Option<std::path::PathBuf>,
    /// PEM private key matching `tls_cert`.
    #[serde(default)]
    pub tls_key: Option<std::path::PathBuf>,
}

impl Config {
    pub fn from_args(args: &super::Args) -> Result<Self> {
        let allowed_dirs: Vec<String> = if !args.directories.is_empty() {
            args.directories.clone()
        } else {
            vec![std::env::current_dir()?.to_string_lossy().to_string()]
        };

        // TLS cert/key for the HTTP transport, from CLI flags or env vars. Both
        // must be supplied together; one without the other is a hard error.
        let tls_cert = args
            .tls_cert
            .clone()
            .or_else(|| std::env::var("MCP_TLS_CERT").ok())
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from);
        let tls_key = args
            .tls_key
            .clone()
            .or_else(|| std::env::var("MCP_TLS_KEY").ok())
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from);
        if tls_cert.is_some() != tls_key.is_some() {
            anyhow::bail!(
                "--tls-cert and --tls-key must be provided together (or both omitted for plaintext HTTP)"
            );
        }

        let allowed_trie = Arc::new(build_allowed_trie(&allowed_dirs));
        let enabled_categories = args.enabled_categories();
        let tools_list_bytes =
            Arc::new(crate::server::build_tools_list_response(&enabled_categories));
        Ok(Config {
            allowed_directories: allowed_dirs,
            server: ServerConfig {
                host: args.host.clone(),
                http_port: args.http_port,
                request_timeout: Duration::from_secs(args.request_timeout),
                access_mode: args.access_mode,
                follow_symlinks: args.follow_symlinks,
                max_request_bytes: args.max_request_bytes,
                auth_token: args.auth_token.clone(),
                enabled_categories,
                tls_cert,
                tls_key,
            },
            max_file_size: args.max_file_size * 1024 * 1024,
            max_decompressed_size: args.max_decompressed_size * 1024 * 1024,
            allowed_trie,
            sandbox: OnceLock::new(),
            tools_list_bytes,
        })
    }

    /// Construct a `Config`, building the precomputed allowed-directory trie and
    /// the category-filtered `tools/list` payload.
    pub fn new(allowed_directories: Vec<String>, server: ServerConfig, max_file_size: u64) -> Self {
        let allowed_trie = Arc::new(build_allowed_trie(&allowed_directories));
        let tools_list_bytes =
            Arc::new(crate::server::build_tools_list_response(&server.enabled_categories));
        Self {
            allowed_directories,
            server,
            max_file_size,
            max_decompressed_size: 1024 * 1024 * 1024,
            allowed_trie,
            sandbox: OnceLock::new(),
            tools_list_bytes,
        }
    }

    /// Borrow the precomputed allowed-directory trie.
    pub fn allowed_trie(&self) -> &PathTrie {
        &self.allowed_trie
    }

    /// Borrow the precomputed allowed-directory trie (alias).
    pub const fn allowed_trie_ref(&self) -> &Arc<PathTrie> {
        &self.allowed_trie
    }

    /// Access the capability-backed sandbox, lazily initialised on first call.
    pub fn sandbox(&self) -> &Arc<Sandbox> {
        self.sandbox.get_or_init(|| {
            Arc::new(Sandbox::new(self).expect("failed to initialise capability sandbox"))
        })
    }

    /// Rebuild the precomputed trie from the current `allowed_directories`.
    /// Needed because the trie is not serialized.
    pub fn rebuild_trie(&mut self) {
        self.allowed_trie = Arc::new(build_allowed_trie(&self.allowed_directories));
    }
}

impl Default for Config {
    fn default() -> Self {
        let allowed_directories = vec![".".to_string()];
        let allowed_trie = Arc::new(build_allowed_trie(&allowed_directories));
        Self {
            allowed_directories,
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                http_port: 3001,
                request_timeout: Duration::from_secs(30),
                access_mode: AccessMode::Unrestricted,
                follow_symlinks: false,
                max_request_bytes: 16 * 1024 * 1024,
                auth_token: None,
                enabled_categories: Vec::new(),
                tls_cert: None,
                tls_key: None,
            },
            max_file_size: 100 * 1024 * 1024,
            max_decompressed_size: 1024 * 1024 * 1024,
            allowed_trie,
            sandbox: OnceLock::new(),
            tools_list_bytes: default_tools_list_bytes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.server.host, "127.0.0.1");
        assert_eq!(cfg.server.http_port, 3001);
        assert_eq!(cfg.max_file_size, 100 * 1024 * 1024);
        assert!(!cfg.server.follow_symlinks);
    }

    #[test]
    fn test_access_mode_parse() {
        assert_eq!(
            "unrestricted".parse::<AccessMode>().unwrap(),
            AccessMode::Unrestricted
        );
        assert_eq!(
            "readonly".parse::<AccessMode>().unwrap(),
            AccessMode::ReadOnly
        );
        assert_eq!(
            "read-only".parse::<AccessMode>().unwrap(),
            AccessMode::ReadOnly
        );
        assert!("invalid".parse::<AccessMode>().is_err());
    }
}
