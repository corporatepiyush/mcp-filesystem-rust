use std::path::{Component, Path, PathBuf};

use crate::config::Config;
use crate::errors::{MCSError, Result};

/// Reject a path string whose `Normal` components resolve through a symlink.
/// Stops at the final component (caller decides how to treat it).
/// Returns `Ok(())` if no intermediate symlink component is found.
fn reject_symlink_components(path: &Path) -> Result<()> {
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
                        "Symlink component in path: {}",
                        c.to_string_lossy()
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Validate that `path` is inside one of the allowed directories and has no
/// symlink components when `config.server.follow_symlinks` is false.
///
/// When `follow_symlinks == false`:
///   1. Check every component of the user-supplied path — if any resolves
///      through a symlink, reject it.
///   2. Then canonicalize and verify the result is inside an allowed dir.
///
/// When `follow_symlinks == true`:
///   Canonicalize (which follows all symlinks) and check the result.
pub fn validate_path(path: &str, config: &Config) -> Result<PathBuf> {
    let user_path = Path::new(path);

    if !config.server.follow_symlinks {
        reject_symlink_components(user_path)?;
        if user_path.is_symlink() {
            return Err(MCSError::PathNotAllowed(format!(
                "Path is a symlink: {}",
                path
            )));
        }
    }

    let canonical = user_path
        .canonicalize()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                // Echo the user-supplied path, not the resolved absolute path,
                // to avoid disclosing filesystem layout outside the sandbox.
                MCSError::PathNotFound(format!("Path does not exist: {path}"))
            } else {
                MCSError::FilesystemError(format!("Cannot resolve path: {e}"))
            }
        })?;

    // Use the precomputed PathTrie for prefix matching against allowed dirs.
    if !config.allowed_trie().contains(&canonical) {
        return Err(MCSError::PathNotAllowed(format!(
            "Path {path} is not in allowed directories"
        )));
    }

    Ok(canonical)
}

/// Validate that `path` is allowed as a *write destination*.
/// The parent must be in an allowed dir AND the final path component itself
/// must not be an existing symlink (unless `follow_symlinks` is true).
/// Unlike `validate_path`, this does not require the file to exist.
///
/// Returns the canonicalized parent joined with the file name — never the
/// raw, un-normalized user input.
pub fn validate_destination(path: &str, config: &Config) -> Result<PathBuf> {
    let p = Path::new(path);

    // Get the parent directory. For a simple filename, parent is "" — map to ".".
    let parent_str = p.parent()
        .and_then(|pp| pp.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(".");
    let canonical_parent = validate_path(parent_str, config)?;

    let file_name = p.file_name().ok_or_else(|| {
        MCSError::InvalidParams(format!("Invalid destination path: {path}"))
    })?;
    let full = canonical_parent.join(file_name);

    // The doc-contract: the final path itself must not be a symlink that would
    // redirect the write outside the sandbox.
    if !config.server.follow_symlinks && full.is_symlink() {
        return Err(MCSError::PathNotAllowed(format!(
            "Destination is a symlink: {path}"
        )));
    }

    Ok(full)
}

/// Validate that the *parent directory* of `path` is allowed and the final
/// component is not a symlink escape. Kept as a thin alias over
/// `validate_destination` so all write destinations share one code path.
pub fn validate_path_parent(path: &str, config: &Config) -> Result<PathBuf> {
    validate_destination(path, config)
}

pub fn is_allowed_directory(dir: &Path, config: &Config) -> bool {
    let canonical = if config.server.follow_symlinks {
        dir.canonicalize()
    } else {
        if reject_symlink_components(dir).is_err() {
            return false;
        }
        dir.canonicalize()
    };

    let canonical = match canonical {
        Ok(p) => p,
        Err(_) => return false,
    };

    config.allowed_trie().contains(&canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn cfg(follow_symlinks: bool) -> Config {
        let mut c = Config::default();
        c.allowed_directories = vec![".".to_string()];
        c.server.follow_symlinks = follow_symlinks;
        c.rebuild_trie();
        c
    }

    #[test]
    fn test_validate_path_exists() {
        let result = validate_path("Cargo.toml", &cfg(false));
        assert!(result.is_ok(), "Cargo.toml should exist and be allowed: {:?}", result.err());
    }

    #[test]
    fn test_validate_path_not_found() {
        let result = validate_path("/nonexistent_path_xyz123", &cfg(false));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), MCSError::PathNotFound(_)));
    }

    #[test]
    fn test_validate_path_rejects_symlink_component() {
        // Create a symlink first
        let link = "test_symlink_target";
        let _ = fs::remove_file(link);
        fs::write("test_symlink_source", "content").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("test_symlink_source", link).unwrap();
            // Access via the symlink should be rejected when follow_symlinks=false
            let result = validate_path(link, &cfg(false));
            assert!(result.is_err(), "Symlink should be rejected when follow_symlinks=false");
            let _ = fs::remove_file(link);
        }
        let _ = fs::remove_file("test_symlink_source");
    }

    #[test]
    fn test_validate_path_follow_symlinks() {
        #[cfg(unix)]
        {
            let link = "test_follow_link";
            let _ = fs::remove_file(link);
            fs::write("test_follow_source", "content").unwrap();
            std::os::unix::fs::symlink("test_follow_source", link).unwrap();
            // With follow_symlinks = true, it should be allowed
            let result = validate_path(link, &cfg(true));
            assert!(result.is_ok(), "Symlink should be allowed when follow_symlinks=true: {:?}", result.err());
            let _ = fs::remove_file(link);
            let _ = fs::remove_file("test_follow_source");
        }
    }

    #[test]
    fn test_validate_destination() {
        // File doesn't exist yet, but parent does
        let result = validate_destination("test_new_file.tmp", &cfg(false));
        assert!(result.is_ok());
        let _ = fs::remove_file("test_new_file.tmp");
    }

    #[test]
    fn test_validate_path_parent_directory() {
        let result = validate_path_parent("src/actions/mod.rs", &cfg(false));
        assert!(result.is_ok());
    }

    /// S1 regression: a destination that already exists as a symlink pointing
    /// outside the sandbox must be rejected when follow_symlinks=false.
    #[test]
    #[cfg(unix)]
    fn test_validate_destination_rejects_symlink_final_component() {
        let link = "test_dest_symlink.tmp";
        let _ = fs::remove_file(link);
        // Point the symlink at an out-of-sandbox absolute target.
        std::os::unix::fs::symlink("/etc/hosts", link).unwrap();

        let result = validate_destination(link, &cfg(false));
        assert!(result.is_err(), "Symlinked destination must be rejected");

        // And allowed when follow_symlinks=true.
        let ok = validate_destination(link, &cfg(true));
        assert!(ok.is_ok(), "Symlinked destination allowed when following symlinks");

        let _ = fs::remove_file(link);
    }
}
