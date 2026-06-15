use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

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
            _ => Err(format!("Invalid access mode: {s}. Use 'unrestricted' or 'readonly'")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[repr(align(64))]
pub struct Config {
    pub allowed_directories: Vec<String>,
    pub server: ServerConfig,
    pub max_file_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[repr(align(64))]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub http_port: u16,
    pub request_timeout: Duration,
    pub access_mode: AccessMode,
    pub follow_symlinks: bool,
}

impl Config {
    pub fn from_args(args: &super::Args) -> Result<Self> {
        let allowed_dirs: Vec<String> = if !args.directories.is_empty() {
            args.directories.clone()
        } else {
            vec![std::env::current_dir()?.to_string_lossy().to_string()]
        };

        Ok(Config {
            allowed_directories: allowed_dirs,
            server: ServerConfig {
                host: args.host.clone(),
                port: args.port,
                http_port: args.http_port,
                request_timeout: Duration::from_secs(args.request_timeout),
                access_mode: args.access_mode,
                follow_symlinks: args.follow_symlinks,
            },
            max_file_size: args.max_file_size * 1024 * 1024,
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            allowed_directories: vec![".".to_string()],
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 3000,
                http_port: 3001,
                request_timeout: Duration::from_secs(30),
                access_mode: AccessMode::Unrestricted,
                follow_symlinks: false,
            },
            max_file_size: 100 * 1024 * 1024,
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
        assert_eq!(cfg.server.port, 3000);
        assert_eq!(cfg.max_file_size, 100 * 1024 * 1024);
        assert!(!cfg.server.follow_symlinks);
    }

    #[test]
    fn test_access_mode_parse() {
        assert_eq!("unrestricted".parse::<AccessMode>().unwrap(), AccessMode::Unrestricted);
        assert_eq!("readonly".parse::<AccessMode>().unwrap(), AccessMode::ReadOnly);
        assert_eq!("read-only".parse::<AccessMode>().unwrap(), AccessMode::ReadOnly);
        assert!("invalid".parse::<AccessMode>().is_err());
    }
}
