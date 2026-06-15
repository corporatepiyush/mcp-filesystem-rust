use cap_std::fs::Dir;
use cap_std::ambient_authority;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use crate::config::Config;
use crate::errors::{MCSError, Result};

pub struct Sandbox {
    roots: HashMap<PathBuf, Dir>,
}

impl Sandbox {
    pub fn new(config: &Config) -> Result<Self> {
        let mut roots = HashMap::new();
        for dir_str in &config.allowed_directories {
            let p = Path::new(dir_str);
            let abs = if p.is_absolute() {
                p.to_path_buf()
            } else {
                std::env::current_dir().unwrap_or_default().join(p)
            };
            
            let canonical = abs.canonicalize().map_err(|e| {
                MCSError::FilesystemError(format!("Cannot canonicalize allowed directory {}: {}", dir_str, e))
            })?;
            
            let dir = Dir::open_ambient_dir(&canonical, ambient_authority()).map_err(|e| {
                MCSError::FilesystemError(format!("Failed to open sandboxed directory {}: {}", dir_str, e))
            })?;
            roots.insert(canonical, dir);
        }
        Ok(Self { roots })
    }

    pub fn resolve(&self, path: &str, config: &Config) -> Result<(&Dir, PathBuf)> {
        let p = Path::new(path);
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(p)
        };
        
        let normalized = normalize_path(&abs);

        let canonical = if let Ok(can) = normalized.canonicalize() {
            can
        } else {
            normalized
        };

        if let Some(root_path) = config.allowed_trie().longest_prefix(&canonical) {
            if let Some(dir) = self.roots.get(&root_path) {
                let relative = canonical.strip_prefix(&root_path).unwrap_or(Path::new("")).to_path_buf();
                return Ok((dir, relative));
            }
        }
        
        Err(MCSError::PathNotAllowed(path.to_string()))
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => { result.pop(); }
            std::path::Component::CurDir => {}
            _ => { result.push(component); }
        }
    }
    result
}

pub fn validate_path(path: &str, config: &Config) -> Result<PathBuf> {
    let p = Path::new(path);
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
    let canonical = abs.canonicalize().map_err(|_| MCSError::PathNotFound(format!("Path does not exist: {path}")))?;
    if config.allowed_trie().contains(&canonical) {
        Ok(canonical)
    } else {
        Err(MCSError::PathNotAllowed(path.to_string()))
    }
}

pub fn validate_destination(path: &str, config: &Config) -> Result<PathBuf> {
    let p = Path::new(path);
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
    if config.allowed_trie().contains(&abs) {
        Ok(abs)
    } else if let Some(parent) = abs.parent() {
        if config.allowed_trie().contains(parent) {
             Ok(abs)
        } else {
             Err(MCSError::PathNotAllowed(path.to_string()))
        }
    } else {
        Err(MCSError::PathNotAllowed(path.to_string()))
    }
}

pub fn validate_path_parent(path: &str, config: &Config) -> Result<PathBuf> {
    validate_destination(path, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        let mut c = Config::default();
        c.allowed_directories = vec![std::env::current_dir().unwrap().to_string_lossy().to_string()];
        c.rebuild_trie();
        c
    }

    #[test]
    fn test_sandbox_init() {
        let c = cfg();
        let sandbox = Sandbox::new(&c).unwrap();
        assert!(!sandbox.roots.is_empty());
    }

    #[test]
    fn test_sandbox_resolve() {
        let c = cfg();
        let sandbox = Sandbox::new(&c).unwrap();
        let (dir, rel) = sandbox.resolve("Cargo.toml", &c).unwrap();
        assert!(dir.metadata(rel).is_ok());
    }
}
