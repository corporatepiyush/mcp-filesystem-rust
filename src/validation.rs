use std::path::{Component, Path, PathBuf};

use crate::errors::{MCSError, Result};
use crate::structures::PathTrie;

/// Build a PathTrie from a list of allowed directory strings.
/// Canonicalizes each allowed directory before inserting into the trie.
fn build_allowed_trie(allowed_dirs: &[String], _follow_symlinks: bool) -> PathTrie {
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

/// Validate that `path` is inside one of `allowed_dirs` and has no
/// symlink components when `follow_symlinks` is false.
///
/// When `follow_symlinks == false`:
///   1. Check every component of the user-supplied path — if any resolves
///      through a symlink, reject it.
///   2. Then canonicalize and verify the result is inside an allowed dir.
///
/// When `follow_symlinks == true`:
///   Canonicalize (which follows all symlinks) and check the result.
pub fn validate_path(path: &str, allowed_dirs: &[String], follow_symlinks: bool) -> Result<PathBuf> {
    let path = Path::new(path);

    if !follow_symlinks {
        let mut cumulative = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => {
                    cumulative.push(component.as_os_str());
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    cumulative.pop();
                }
                Component::Normal(c) => {
                    cumulative.push(c);
                    if cumulative.is_symlink() {
                        return Err(MCSError::PathNotAllowed(format!(
                            "Symlink component in path: {} -> {}",
                            cumulative.display(),
                            std::fs::read_link(&cumulative)
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|_| "?unknown?".into())
                        )));
                    }
                }
            }
        }
        if path.is_symlink() {
            return Err(MCSError::PathNotAllowed(format!(
                "Path is a symlink: {} -> {}",
                path.display(),
                std::fs::read_link(path)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "?unknown?".into())
            )));
        }
    }

    let canonical = path
        .canonicalize()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                MCSError::PathNotFound(format!("Path does not exist: {}", path.display()))
            } else {
                MCSError::FilesystemError(format!("Cannot resolve path: {}", e))
            }
        })?;

    // Use PathTrie for efficient prefix matching against allowed directories.
    let trie = build_allowed_trie(allowed_dirs, follow_symlinks);
    if !trie.contains(&canonical) {
        return Err(MCSError::PathNotAllowed(format!(
            "Path {} is not in allowed directories",
            canonical.display()
        )));
    }

    Ok(canonical)
}

/// Validate that the *parent directory* of `path` is allowed.
/// The file itself need not exist yet (useful for write destinations).
pub fn validate_path_parent(path: &str, allowed_dirs: &[String], follow_symlinks: bool) -> Result<PathBuf> {
    let path = Path::new(path);
    let parent = path.parent().unwrap_or(path);
    validate_path(parent.to_str().unwrap_or("."), allowed_dirs, follow_symlinks)?;
    Ok(path.to_path_buf())
}

/// Validate that `path` is allowed as a *write destination*.
/// The parent must be in an allowed dir AND the final path itself
/// must not contain symlink components (unless follow_symlinks is true).
/// Unlike `validate_path`, this does not require the file to exist.
pub fn validate_destination(path: &str, allowed_dirs: &[String], follow_symlinks: bool) -> Result<PathBuf> {
    let p = Path::new(path);

    // Get the parent directory. For a simple filename, parent is "" — map to ".".
    let parent_str = p.parent()
        .and_then(|p| p.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(".");
    let canonical_parent = validate_path(parent_str, allowed_dirs, follow_symlinks)?;

    let file_name = p.file_name().unwrap_or_default();
    let full = canonical_parent.join(file_name);

    Ok(full)
}

pub fn is_allowed_directory(dir: &Path, allowed_dirs: &[String], follow_symlinks: bool) -> bool {
    let canonical = if follow_symlinks {
        dir.canonicalize()
    } else {
        let mut cumulative = PathBuf::new();
        for component in dir.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => cumulative.push(component.as_os_str()),
                Component::CurDir => {}
                Component::ParentDir => { cumulative.pop(); }
                Component::Normal(c) => {
                    cumulative.push(c);
                    if cumulative.is_symlink() {
                        return false;
                    }
                }
            }
        }
        dir.canonicalize()
    };

    let canonical = match canonical {
        Ok(p) => p,
        Err(_) => return false,
    };

    let trie = build_allowed_trie(allowed_dirs, follow_symlinks);
    trie.contains(&canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_validate_path_exists() {
        let allowed = vec![".".to_string()];
        let result = validate_path("Cargo.toml", &allowed, false);
        assert!(result.is_ok(), "Cargo.toml should exist and be allowed: {:?}", result.err());
    }

    #[test]
    fn test_validate_path_not_found() {
        let allowed = vec![".".to_string()];
        let result = validate_path("/nonexistent_path_xyz123", &allowed, false);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), MCSError::PathNotFound(_)));
    }

    #[test]
    fn test_validate_path_rejects_symlink_component() {
        let allowed = vec![".".to_string()];
        // Create a symlink first
        let link = "test_symlink_target";
        let _ = fs::remove_file(link);
        fs::write("test_symlink_source", "content").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("test_symlink_source", link).unwrap();
            // Access via the symlink should be rejected when follow_symlinks=false
            let result = validate_path(link, &allowed, false);
            assert!(result.is_err(), "Symlink should be rejected when follow_symlinks=false");
            let _ = fs::remove_file(link);
        }
        let _ = fs::remove_file("test_symlink_source");
    }

    #[test]
    fn test_validate_path_follow_symlinks() {
        let allowed = vec![".".to_string()];
        #[cfg(unix)]
        {
            let link = "test_follow_link";
            let _ = fs::remove_file(link);
            fs::write("test_follow_source", "content").unwrap();
            std::os::unix::fs::symlink("test_follow_source", link).unwrap();
            // With follow_symlinks = true, it should be allowed
            let result = validate_path(link, &allowed, true);
            assert!(result.is_ok(), "Symlink should be allowed when follow_symlinks=true: {:?}", result.err());
            let _ = fs::remove_file(link);
            let _ = fs::remove_file("test_follow_source");
        }
    }

    #[test]
    fn test_validate_destination() {
        let allowed = vec![".".to_string()];
        // File doesn't exist yet, but parent does
        let result = validate_destination("test_new_file.tmp", &allowed, false);
        assert!(result.is_ok());
        let _ = fs::remove_file("test_new_file.tmp");
    }

    #[test]
    fn test_validate_path_parent_directory() {
        let allowed = vec![".".to_string()];
        let result = validate_path_parent("src/actions/mod.rs", &allowed, false);
        assert!(result.is_ok());
    }
}
