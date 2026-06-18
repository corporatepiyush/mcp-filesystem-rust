use async_compression::tokio::bufread::GzipDecoder as AsyncGzipDecoder;
use async_compression::tokio::bufread::ZstdDecoder as AsyncZstdDecoder;
use serde_json::{Value, json};

#[cfg(unix)]
use cap_std::fs::PermissionsExt as CapPermissionsExt;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256, Sha512};
use std::io::{BufRead, Read, Seek};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::AsyncWrite;
use walkdir::WalkDir;

use crate::config::Config;
use crate::errors::{MCSError, Result};
use crate::structures::RingBuffer;
use memmap2::Mmap;

/// Files below this size are read with a plain `read` syscall; at or above it
/// memory-mapping wins. Avoids mmap/munmap + page-fault overhead on tiny files.
const MMAP_THRESHOLD: u64 = 256 * 1024;

pub async fn read_text_file(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let head = get_opt_i64(args, "head").map(|v| v.clamp(0, 100_000));
    let tail = get_opt_i64(args, "tail").map(|v| v.clamp(0, 100_000));

    if head.is_some() && tail.is_some() {
        return Err(MCSError::InvalidParams(
            "Cannot specify both head and tail simultaneously".into(),
        ));
    }

    let valid_path = config.sandbox().resolve_path(&path)?;
    if !valid_path.exists() {
        return Err(MCSError::PathNotFound(format!(
            "Path does not exist: {path}"
        )));
    }
    let cap_meta = config.sandbox().metadata(&path).await?;

    if !cap_meta.is_file() {
        return Err(MCSError::InvalidParams(format!(
            "Path is not a file: {path}"
        )));
    }

    let file_size = cap_meta.len();
    if file_size > config.max_file_size {
        return Err(MCSError::FilesystemError(format!(
            "File size {size} exceeds maximum allowed size {max}",
            size = file_size,
            max = config.max_file_size
        )));
    }

    if let Some(h) = head {
        let h = h as usize;
        let path_clone = valid_path.clone();
        let (result_lines, count) = tokio::task::spawn_blocking(
            move || -> std::result::Result<(Vec<String>, usize), String> {
                let file = std::fs::File::open(&path_clone)
                    .map_err(|e| format!("Cannot open file: {e}"))?;
                let reader = std::io::BufReader::new(file);
                let mut result_lines = Vec::with_capacity(h);
                let mut count = 0usize;
                for line in reader.lines() {
                    if count >= h {
                        break;
                    }
                    count += 1;
                    result_lines.push(line.map_err(|e| format!("Cannot read file: {e}"))?);
                }
                Ok((result_lines, count))
            },
        )
        .await
        .map_err(|e| MCSError::FilesystemError(format!("read_text_file task failed: {e}")))?
        .map_err(MCSError::FilesystemError)?;
        return Ok(json!({
            "content": result_lines.join("\n"),
            "size": file_size,
            "totalLines": count,
            "path": valid_path.to_string_lossy(),
        }));
    }

    if let Some(t) = tail {
        let t = t as usize;
        let path_clone = valid_path.clone();
        let (total_lines, lines) = tokio::task::spawn_blocking(
            move || -> std::result::Result<(usize, Vec<String>), String> {
                let file = std::fs::File::open(&path_clone)
                    .map_err(|e| format!("Cannot open file: {e}"))?;
                let reader = std::io::BufReader::new(file);
                let mut ring = RingBuffer::new(t);
                let mut total_lines = 0usize;
                for line in reader.lines() {
                    total_lines += 1;
                    ring.push(line.map_err(|e| format!("Cannot read file: {e}"))?);
                }
                Ok((total_lines, ring.into_vec()))
            },
        )
        .await
        .map_err(|e| MCSError::FilesystemError(format!("read_text_file task failed: {e}")))?
        .map_err(MCSError::FilesystemError)?;
        return Ok(json!({
            "content": lines.join("\n"),
            "size": file_size,
            "totalLines": total_lines,
            "path": valid_path.to_string_lossy(),
        }));
    }

    let path_clone = valid_path.clone();
    let content = tokio::task::spawn_blocking(move || -> std::result::Result<String, String> {
        if file_size < MMAP_THRESHOLD {
            let bytes = std::fs::read(&path_clone).map_err(|e| format!("Cannot read file: {e}"))?;
            Ok(match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
            })
        } else {
            let file =
                std::fs::File::open(&path_clone).map_err(|e| format!("Cannot open file: {e}"))?;
            let mmap = unsafe { Mmap::map(&file).map_err(|e| format!("Cannot mmap file: {e}"))? };
            Ok(String::from_utf8_lossy(&mmap).into_owned())
        }
    })
    .await
    .map_err(|e| MCSError::FilesystemError(format!("read_text_file task failed: {e}")))?
    .map_err(MCSError::FilesystemError)?;

    let total_lines = content.lines().count();

    Ok(json!({
        "content": content,
        "size": file_size,
        "totalLines": total_lines,
        "path": valid_path.to_string_lossy(),
    }))
}

pub async fn read_media_file(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = config.sandbox().resolve_path(&path)?;
    let cap_meta = config.sandbox().metadata(&path).await?;

    if !cap_meta.is_file() {
        return Err(MCSError::InvalidParams(format!(
            "Path is not a file: {path}"
        )));
    }

    let file_size = cap_meta.len();
    if file_size > config.max_file_size {
        return Err(MCSError::FilesystemError(format!(
            "File size {size} exceeds maximum allowed size {max}",
            size = file_size,
            max = config.max_file_size
        )));
    }

    let path_clone = valid_path.clone();
    let data = tokio::task::spawn_blocking(move || -> std::result::Result<Vec<u8>, String> {
        if file_size < MMAP_THRESHOLD {
            std::fs::read(&path_clone).map_err(|e| format!("Cannot read file: {e}"))
        } else {
            let file =
                std::fs::File::open(&path_clone).map_err(|e| format!("Cannot open file: {e}"))?;
            let mmap = unsafe { Mmap::map(&file).map_err(|e| format!("Cannot mmap file: {e}"))? };
            Ok(mmap.to_vec())
        }
    })
    .await
    .map_err(|e| MCSError::FilesystemError(format!("read_media_file task failed: {e}")))?
    .map_err(MCSError::FilesystemError)?;

    let mime_type = infer::get(&data)
        .map(|t| t.mime_type())
        .unwrap_or("application/octet-stream");
    let content_type = content_inspector::inspect(&data);
    let detected_mime = if content_type == content_inspector::ContentType::BINARY {
        "application/octet-stream"
    } else {
        "text/plain"
    };

    let encoded = base64_simd::STANDARD.encode_to_string(&data);

    // Return spec-compliant typed content: ImageContent/AudioContent for
    // recognised media, otherwise a text note with the base64 in structuredContent.
    let kind = if mime_type.starts_with("image/") {
        "image"
    } else if mime_type.starts_with("audio/") {
        "audio"
    } else {
        ""
    };

    if kind.is_empty() {
        Ok(json!({
            "content": [{
                "type": "text",
                "text": format!(
                    "Binary file ({mime_type}, {file_size} bytes); base64 data in structuredContent.data"
                )
            }],
            "structuredContent": {
                "data": encoded,
                "mimeType": mime_type,
                "detectedType": detected_mime,
                "size": file_size,
                "path": valid_path.to_string_lossy(),
            },
            "isError": false
        }))
    } else {
        Ok(json!({
            "content": [{
                "type": kind,
                "data": encoded,
                "mimeType": mime_type,
            }],
            "isError": false
        }))
    }
}

pub async fn write_file(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let content = get_str_arg(args, "content")?;
    let valid_path = config.sandbox().resolve_destination_path(&path)?;

    config.sandbox().write(&path, content.as_bytes()).await?;

    Ok(json!({ "success": true, "path": valid_path.to_string_lossy() }))
}

pub async fn edit_file(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let edits = get_edits_arg(args)?;
    let dry_run = get_opt_bool(args, "dryRun").unwrap_or(false);

    let valid_path = config.sandbox().resolve_path(&path)?;

    let cap_meta = config.sandbox().metadata(&path).await?;
    if cap_meta.len() > config.max_file_size {
        return Err(MCSError::FilesystemError(format!(
            "File size {size} exceeds maximum allowed size {max}",
            size = cap_meta.len(),
            max = config.max_file_size
        )));
    }

    let content = config.sandbox().read_to_string(&path).await?;

    let indent = detect_indent(&content);
    let mut result = content;
    let mut diffs = Vec::new();

    for edit in &edits {
        let old_text = edit
            .get("oldText")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                MCSError::InvalidParams("Each edit must have 'oldText' string".into())
            })?;
        let new_text = edit.get("newText").and_then(|v| v.as_str()).unwrap_or("");

        let normalized_old = normalize_whitespace(old_text, indent);
        let normalized_new = normalize_whitespace(new_text, indent);

        if let Some(pos) = result.find(&normalized_old) {
            let end = pos + normalized_old.len();
            let context_start = floor_char_boundary(&result, pos.saturating_sub(40));
            let context_end = ceil_char_boundary(&result, (end + 40).min(result.len()));

            diffs.push(json!({
                "oldText": old_text,
                "newText": new_text,
                "position": pos,
                "context": format!("...{}...", &result[context_start..context_end].replace('\n', "\\n")),
            }));

            result.replace_range(pos..end, &normalized_new);
        } else {
            diffs.push(json!({
                "oldText": old_text,
                "newText": new_text,
                "error": "Pattern not found in file",
            }));
        }
    }

    if !dry_run {
        config.sandbox().write(&path, result.as_bytes()).await?;
    }

    Ok(json!({
        "success": !dry_run,
        "dryRun": dry_run,
        "edits": diffs,
        "path": valid_path.to_string_lossy(),
    }))
}

pub async fn create_directory(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = config.sandbox().resolve_destination_path(&path)?;

    config.sandbox().create_dir_all(&path).await?;

    Ok(json!({ "success": true, "path": valid_path.to_string_lossy() }))
}

pub async fn list_directory(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = config.sandbox().resolve_path(&path)?;

    let entries_raw = config.sandbox().read_dir(&path).await?;
    let mut entries: Vec<String> = entries_raw
        .into_iter()
        .map(|(name, is_dir)| {
            let prefix = if is_dir { "[DIR]" } else { "[FILE]" };
            format!("{prefix} {name}")
        })
        .collect();
    entries.sort_unstable();

    Ok(json!({ "entries": entries, "path": valid_path.to_string_lossy() }))
}

pub async fn list_directory_with_sizes(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let sort_by = get_opt_str(args, "sortBy").unwrap_or_else(|| "name".to_string());
    let valid_path = config.sandbox().resolve_path(&path)?;

    let path_clone = valid_path.clone();
    let (mut entries, total_files, total_dirs, combined_size) = tokio::task::spawn_blocking(
        move || -> std::result::Result<(Vec<Value>, u64, u64, u64), String> {
            let mut entries = Vec::new();
            let mut total_files = 0u64;
            let mut total_dirs = 0u64;
            let mut combined_size = 0u64;

            let read_dir = std::fs::read_dir(&path_clone)
                .map_err(|e| format!("Cannot read directory: {e}"))?;

            for entry in read_dir {
                let entry = entry.map_err(|e| format!("Error reading directory entry: {e}"))?;
                let name = entry.file_name().to_string_lossy().to_string();
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };

                if file_type.is_dir() {
                    total_dirs += 1;
                    entries.push(
                        json!({ "name": name, "type": "dir", "display": format!("[DIR] {name}") }),
                    );
                } else {
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    total_files += 1;
                    combined_size += size;
                    entries.push(json!({
                        "name": name,
                        "type": "file",
                        "size": size,
                        "display": format!("[FILE] {name} ({size} B)"),
                    }));
                }
            }

            Ok((entries, total_files, total_dirs, combined_size))
        },
    )
    .await
    .map_err(|e| MCSError::FilesystemError(format!("Directory listing task failed: {e}")))?
    .map_err(MCSError::FilesystemError)?;

    match sort_by.as_str() {
        "size" => entries.sort_by(|a, b| {
            let a_size = a.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            let b_size = b.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            b_size.cmp(&a_size)
        }),
        _ => entries.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        }),
    }

    Ok(json!({
        "entries": entries,
        "summary": {
            "totalFiles": total_files,
            "totalDirectories": total_dirs,
            "combinedSize": combined_size,
        },
        "path": valid_path.to_string_lossy(),
    }))
}

pub async fn move_file(args: Option<&Value>, config: &Config) -> Result<Value> {
    let source = get_str_arg(args, "source")?;
    let destination = get_str_arg(args, "destination")?;

    let valid_source = config.sandbox().resolve_path(&source)?;
    let valid_dest = config.sandbox().resolve_destination_path(&destination)?;

    config.sandbox().rename(&source, &destination).await?;

    Ok(json!({
        "success": true,
        "source": valid_source.to_string_lossy(),
        "destination": valid_dest.to_string_lossy(),
    }))
}

pub async fn copy_file(args: Option<&Value>, config: &Config) -> Result<Value> {
    let source = get_str_arg(args, "source")?;
    let destination = get_str_arg(args, "destination")?;

    let valid_source = config.sandbox().resolve_path(&source)?;
    let valid_dest = config.sandbox().resolve_destination_path(&destination)?;

    config.sandbox().copy(&source, &destination).await?;

    Ok(json!({
        "success": true,
        "source": valid_source.to_string_lossy(),
        "destination": valid_dest.to_string_lossy(),
    }))
}

pub async fn delete_file(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = config.sandbox().resolve_path(&path)?;

    if !valid_path.is_file() {
        if !valid_path.exists() {
            return Err(MCSError::PathNotFound(path));
        }
        return Err(MCSError::InvalidParams(format!(
            "Path is not a file: {path}"
        )));
    }

    config.sandbox().remove_file(&path).await?;

    Ok(json!({ "success": true, "path": valid_path.to_string_lossy() }))
}

pub async fn delete_directory(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let recursive = get_opt_bool(args, "recursive").unwrap_or(false);
    let valid_path = config.sandbox().resolve_path(&path)?;

    if recursive {
        config.sandbox().remove_dir_all(&path).await?;
    } else {
        config.sandbox().remove_dir(&path).await?;
    }

    Ok(json!({ "success": true, "path": valid_path.to_string_lossy(), "recursive": recursive }))
}

pub async fn search_files(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let pattern = get_str_arg(args, "pattern")?;
    let exclude_patterns: Vec<String> = get_opt_str_array(args, "excludePatterns");
    let valid_path = config.sandbox().resolve_path(&path)?;

    let glob = globset::Glob::new(&pattern)
        .map_err(|e| MCSError::InvalidParams(format!("Invalid glob pattern: {e}")))?
        .compile_matcher();

    let exclude_globs: Vec<globset::GlobMatcher> = exclude_patterns
        .iter()
        .filter_map(|p| globset::Glob::new(p).ok().map(|g| g.compile_matcher()))
        .collect();

    let root = valid_path.clone();
    let follow = config.server.follow_symlinks;

    let results =
        tokio::task::spawn_blocking(move || -> std::result::Result<Vec<String>, String> {
            let mut res = Vec::new();
            const SEARCH_LIMIT: usize = 100_000;
            let walker = WalkDir::new(&root)
                .follow_links(follow)
                .into_iter()
                .filter_entry(|e| !is_hidden(e));
            for entry in walker.filter_map(|e| e.ok()) {
                if res.len() >= SEARCH_LIMIT {
                    break;
                }
                let relative = entry.path().strip_prefix(&root).unwrap_or(entry.path());
                let relative_str = relative.to_string_lossy();
                if exclude_globs
                    .iter()
                    .any(|g| g.is_match(relative_str.as_ref()))
                {
                    continue;
                }
                if glob.is_match(relative_str.as_ref()) {
                    res.push(entry.path().to_string_lossy().to_string());
                }
            }
            Ok(res)
        })
        .await
        .map_err(|e| MCSError::FilesystemError(format!("Search task failed: {e}")))?
        .map_err(MCSError::FilesystemError)?;

    Ok(json!({
        "results": results,
        "count": results.len(),
        "pattern": pattern,
        "path": valid_path.to_string_lossy(),
    }))
}

pub async fn directory_tree(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let exclude_patterns: Vec<String> = get_opt_str_array(args, "excludePatterns");
    let valid_path = config.sandbox().resolve_path(&path)?;

    let root = valid_path.clone();
    let exclude_globs: Vec<globset::GlobMatcher> = exclude_patterns
        .iter()
        .filter_map(|p| globset::Glob::new(p).ok().map(|g| g.compile_matcher()))
        .collect();

    let tree = tokio::task::spawn_blocking(move || {
        let mut nodes = 0usize;
        build_tree(&root, &root, &exclude_globs, 0, &mut nodes)
    })
    .await
    .map_err(|e| MCSError::FilesystemError(format!("Directory tree task failed: {e}")))?;

    Ok(json!(tree))
}

pub async fn get_file_info(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = config.sandbox().resolve_path(&path)?;
    let cap_meta = config.sandbox().metadata(&path).await?;

    let file_type = if cap_meta.is_dir() {
        "directory"
    } else if cap_meta.file_type().is_symlink() {
        "symlink"
    } else {
        "file"
    };

    let created = cap_meta.created().ok().and_then(|t| {
        t.into_std()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs_f64())
    });
    let modified = cap_meta.modified().ok().and_then(|t| {
        t.into_std()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs_f64())
    });
    let accessed = cap_meta.accessed().ok().and_then(|t| {
        t.into_std()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs_f64())
    });

    let permissions = format!("{:o}", cap_meta.permissions().mode() & 0o777);

    Ok(json!({
        "path": valid_path.to_string_lossy(),
        "type": file_type,
        "size": cap_meta.len(),
        "permissions": permissions,
        "created": created,
        "modified": modified,
        "accessed": accessed,
        "readonly": cap_meta.permissions().readonly(),
    }))
}

pub async fn list_allowed_directories(_args: Option<&Value>, config: &Config) -> Result<Value> {
    Ok(json!({ "directories": config.allowed_directories }))
}

pub async fn hash_file(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let algorithm = get_opt_str(args, "algorithm").unwrap_or_else(|| "sha256".to_string());
    let valid_path = config.sandbox().resolve_path(&path)?;

    if !valid_path.is_file() {
        return Err(MCSError::InvalidParams(format!(
            "Path is not a file: {path}"
        )));
    }

    let max_size = config.max_file_size;
    let path_clone = valid_path.clone();
    let alg = algorithm.clone();
    let (hash, _file_size) =
        tokio::task::spawn_blocking(move || -> std::result::Result<(String, u64), String> {
            let meta =
                std::fs::metadata(&path_clone).map_err(|e| format!("Cannot get metadata: {e}"))?;
            let size = meta.len();
            if size > max_size {
                return Err(format!(
                    "File size {size} exceeds maximum allowed size {max_size}"
                ));
            }
            let file = std::fs::File::open(&path_clone)
                .map_err(|e| format!("Cannot open file for hashing: {e}"))?;
            let result = if size < MMAP_THRESHOLD {
                let data = std::fs::read(&path_clone)
                    .map_err(|e| format!("Cannot read file for hashing: {e}"))?;
                hash_bytes(&alg, &data)
            } else {
                let mmap =
                    unsafe { Mmap::map(&file).map_err(|e| format!("Cannot mmap file: {e}"))? };
                hash_bytes(&alg, &mmap)
            }?;
            Ok((result, size))
        })
        .await
        .map_err(|e| MCSError::FilesystemError(format!("Hash task failed: {e}")))?
        .map_err(MCSError::FilesystemError)?;

    Ok(json!({
        "path": valid_path.to_string_lossy(),
        "algorithm": algorithm,
        "hash": hash,
    }))
}

pub async fn grep_files(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let pattern = get_str_arg(args, "pattern")?;
    let valid_path = config.sandbox().resolve_path(&path)?;

    let re = regex::Regex::new(&pattern)
        .map_err(|e| MCSError::InvalidParams(format!("Invalid regex pattern: {e}")))?;

    let exclude_patterns: Vec<String> = get_opt_str_array(args, "excludePatterns");
    let exclude_globs: Vec<globset::GlobMatcher> = exclude_patterns
        .iter()
        .filter_map(|p| globset::Glob::new(p).ok().map(|g| g.compile_matcher()))
        .collect();

    let root = valid_path.clone();
    let follow = config.server.follow_symlinks;
    let max_bytes = config.max_file_size;

    let results =
        tokio::task::spawn_blocking(move || -> std::result::Result<Vec<Value>, String> {
            let mut res = Vec::new();
            const GREP_LIMIT: usize = 100_000;

            let walker = WalkDir::new(&root)
                .follow_links(follow)
                .into_iter()
                .filter_entry(|e| !is_hidden(e));

            for entry in walker.filter_map(|e| e.ok()) {
                if res.len() >= GREP_LIMIT {
                    break;
                }
                if !entry.file_type().is_file() {
                    continue;
                }

                let relative = entry.path().strip_prefix(&root).unwrap_or(entry.path());
                let relative_str = relative.to_string_lossy();

                if exclude_globs
                    .iter()
                    .any(|g| g.is_match(relative_str.as_ref()))
                {
                    continue;
                }

                let ext = entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if is_binary_extension(ext) {
                    continue;
                }

                if entry.metadata().map(|m| m.len()).unwrap_or(0) > max_bytes {
                    continue;
                }

                let file = match std::fs::File::open(entry.path()) {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                let reader = std::io::BufReader::new(file);

                for (idx, line) in reader.lines().enumerate() {
                    let Ok(line) = line else { break };
                    if res.len() >= GREP_LIMIT {
                        break;
                    }
                    if re.is_match(&line) {
                        res.push(json!({
                            "file": entry.path().to_string_lossy(),
                            "line": idx + 1,
                            "content": line,
                        }));
                    }
                }
            }
            Ok(res)
        })
        .await
        .map_err(|e| MCSError::FilesystemError(format!("Grep task failed: {e}")))?
        .map_err(MCSError::FilesystemError)?;

    Ok(json!({ "results": results, "count": results.len(), "pattern": pattern }))
}

pub async fn set_permissions(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let mode_str = get_str_arg(args, "mode")?;
    let valid_path = config.sandbox().resolve_path(&path)?;

    let mode = u32::from_str_radix(&mode_str, 8).map_err(|_| {
        MCSError::InvalidParams(format!(
            "Invalid mode: {mode_str}. Use octal format (e.g. 644, 755)"
        ))
    })?;

    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt;
        let perm = cap_std::fs::Permissions::from_mode(mode);
        config.sandbox().set_permissions(&path, perm).await?;
    }

    #[cfg(not(unix))]
    {
        let _ = mode;
        return Err(MCSError::FilesystemError(
            "Permission changes are not supported on this platform".into(),
        ));
    }

    Ok(json!({ "success": true, "path": valid_path.to_string_lossy(), "mode": mode_str }))
}

pub async fn get_disk_usage(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = config.sandbox().resolve_path(&path)?;

    let root = valid_path.clone();
    let follow = config.server.follow_symlinks;

    let usage =
        tokio::task::spawn_blocking(move || -> std::result::Result<(u64, u64, u64), String> {
            let mut total_size = 0u64;
            let mut file_count = 0u64;
            let mut dir_count = 0u64;

            let walker = WalkDir::new(&root).follow_links(follow).into_iter();

            for entry in walker.filter_map(|e| e.ok()) {
                if entry.file_type().is_dir() {
                    dir_count += 1;
                } else if entry.file_type().is_file()
                    && let Ok(meta) = entry.metadata()
                {
                    total_size += meta.len();
                    file_count += 1;
                }
            }

            Ok((total_size, file_count, dir_count))
        })
        .await
        .map_err(|e| MCSError::FilesystemError(format!("Disk usage task failed: {e}")))?
        .map_err(MCSError::FilesystemError)?;

    Ok(json!({
        "path": valid_path.to_string_lossy(),
        "totalSize": usage.0,
        "fileCount": usage.1,
        "directoryCount": usage.2,
    }))
}

pub async fn create_symlink(args: Option<&Value>, config: &Config) -> Result<Value> {
    let source = get_str_arg(args, "source")?;
    let link_path = get_str_arg(args, "linkPath")?;

    let valid_source = config.sandbox().resolve_path(&source)?;
    let valid_link = config.sandbox().resolve_destination_path(&link_path)?;

    config.sandbox().create_symlink(&source, &link_path).await?;

    Ok(json!({
        "success": true,
        "source": valid_source.to_string_lossy(),
        "linkPath": valid_link.to_string_lossy(),
    }))
}

pub async fn read_file_range(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let offset = get_i64_arg(args, "offset")?;
    let length = get_i64_arg(args, "length")?;

    if offset < 0 || length < 0 {
        return Err(MCSError::InvalidParams(
            "offset and length must be non-negative".into(),
        ));
    }

    let valid_path = config.sandbox().resolve_path(&path)?;

    let max_size = config.max_file_size;
    let path_clone = valid_path.clone();
    let content =
        tokio::task::spawn_blocking(move || -> std::result::Result<(String, i64), String> {
            let meta = std::fs::metadata(&path_clone)
                .map_err(|e| format!("Cannot get file metadata: {e}"))?;
            let file_size = meta.len() as i64;
            if offset >= file_size {
                return Err(format!("Offset {offset} exceeds file size {file_size}"));
            }
            let actual = (offset as u64)
                .saturating_add(length as u64)
                .min(file_size as u64)
                .saturating_sub(offset as u64);
            if actual > max_size {
                return Err(format!(
                    "Requested range {actual} exceeds maximum allowed size {max_size}"
                ));
            }
            let mut file =
                std::fs::File::open(&path_clone).map_err(|e| format!("Cannot open file: {e}"))?;
            file.seek(std::io::SeekFrom::Start(offset as u64))
                .map_err(|e| format!("Cannot seek: {e}"))?;
            let mut buf = Vec::with_capacity(actual as usize);
            file.take(actual)
                .read_to_end(&mut buf)
                .map_err(|e| format!("Cannot read range: {e}"))?;
            Ok((String::from_utf8_lossy(&buf).into_owned(), actual as i64))
        })
        .await
        .map_err(|e| MCSError::FilesystemError(format!("read_file_range task failed: {e}")))?
        .map_err(MCSError::FilesystemError)?;

    Ok(json!({
        "content": content.0,
        "offset": offset,
        "length": content.1,
        "path": valid_path.to_string_lossy(),
    }))
}

pub async fn compress_gzip(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let level = get_opt_i64(args, "level").unwrap_or(6);
    let level = level.clamp(0, 9) as u32;
    let output = get_opt_str(args, "output");

    let valid_path = config.sandbox().resolve_path(&path)?;
    let output_path = resolve_output(&valid_path, output.as_deref(), ".gz", config)?;

    if output_path == valid_path {
        return Err(MCSError::InvalidParams(
            "Output path must differ from source".into(),
        ));
    }

    let src = valid_path.clone();
    let dst = output_path.clone();
    let (original_size, compressed_size) =
        tokio::task::spawn_blocking(move || -> std::result::Result<(u64, u64), String> {
            let meta =
                std::fs::metadata(&src).map_err(|e| format!("Cannot get source metadata: {e}"))?;
            let original_size = meta.len();
            let mut input =
                std::fs::File::open(&src).map_err(|e| format!("Cannot open file: {e}"))?;
            let output = std::fs::File::create(&dst)
                .map_err(|e| format!("Cannot create output file: {e}"))?;
            let mut encoder = GzEncoder::new(output, Compression::new(level));
            std::io::copy(&mut input, &mut encoder)
                .map_err(|e| format!("gzip compression failed: {e}"))?;
            let output_file = encoder
                .finish()
                .map_err(|e| format!("gzip compression finalize failed: {e}"))?;
            let size = output_file
                .metadata()
                .map_err(|e| format!("Cannot get output metadata: {e}"))?
                .len();
            Ok((original_size, size))
        })
        .await
        .map_err(|e| MCSError::FilesystemError(format!("Compression task failed: {e}")))?
        .map_err(MCSError::FilesystemError)?;

    let ratio = compute_ratio(original_size, compressed_size);

    Ok(json!({
        "success": true,
        "source": valid_path.to_string_lossy(),
        "output": output_path.to_string_lossy(),
        "algorithm": "gzip",
        "level": level,
        "originalSize": original_size,
        "compressedSize": compressed_size,
        "ratio": ratio,
    }))
}

/// Async writer with a byte limit — decompression bomb protection.
struct AsyncLimitedWriter<W: AsyncWrite + Unpin> {
    inner: W,
    written: u64,
    limit: u64,
}

impl<W: AsyncWrite + Unpin> AsyncLimitedWriter<W> {
    const fn new(inner: W, limit: u64) -> Self {
        Self {
            inner,
            written: 0,
            limit,
        }
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for AsyncLimitedWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let new_total = self.written.saturating_add(buf.len() as u64);
        if new_total > self.limit {
            return Poll::Ready(Err(std::io::Error::other(format!(
                "Decompressed output exceeds maximum allowed size of {} bytes",
                self.limit
            ))));
        }
        match Pin::new(&mut self.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(count)) => {
                self.written += count as u64;
                Poll::Ready(Ok(count))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub async fn decompress_gzip(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let output = get_opt_str(args, "output");

    let valid_path = config.sandbox().resolve_path(&path)?;
    let output_path = resolve_decompress_output(&valid_path, output.as_deref(), ".gz", config)?;

    if output_path == valid_path {
        return Err(MCSError::InvalidParams(
            "Output path must differ from source".into(),
        ));
    }

    let src = valid_path.clone();
    let dst = output_path.clone();
    let max_out = config.max_decompressed_size;

    let reader = tokio::io::BufReader::new(
        tokio::fs::File::open(&src)
            .await
            .map_err(|e| MCSError::FilesystemError(format!("Cannot open compressed file: {e}")))?,
    );
    let mut decoder = AsyncGzipDecoder::new(reader);
    let output = tokio::fs::File::create(&dst)
        .await
        .map_err(|e| MCSError::FilesystemError(format!("Cannot create output file: {e}")))?;
    let mut writer = AsyncLimitedWriter::new(tokio::io::BufWriter::new(output), max_out);
    tokio::io::copy(&mut decoder, &mut writer)
        .await
        .map_err(|e| MCSError::FilesystemError(format!("gzip decompression failed: {e}")))?;
    let decompressed_size = writer.written;

    Ok(json!({
        "success": true,
        "source": valid_path.to_string_lossy(),
        "output": output_path.to_string_lossy(),
        "algorithm": "gzip",
        "decompressedSize": decompressed_size,
    }))
}

pub async fn compress_zstd(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let level = get_opt_i64(args, "level").unwrap_or(3);
    let level = level.clamp(1, 22) as i32;
    let output = get_opt_str(args, "output");

    let valid_path = config.sandbox().resolve_path(&path)?;
    let output_path = resolve_output(&valid_path, output.as_deref(), ".zst", config)?;

    if output_path == valid_path {
        return Err(MCSError::InvalidParams(
            "Output path must differ from source".into(),
        ));
    }

    let src = valid_path.clone();
    let dst = output_path.clone();
    let lvl = level;
    let (original_size, compressed_size) =
        tokio::task::spawn_blocking(move || -> std::result::Result<(u64, u64), String> {
            let meta =
                std::fs::metadata(&src).map_err(|e| format!("Cannot get source metadata: {e}"))?;
            let original_size = meta.len();
            let mut input =
                std::fs::File::open(&src).map_err(|e| format!("Cannot open file: {e}"))?;
            let output = std::fs::File::create(&dst)
                .map_err(|e| format!("Cannot create output file: {e}"))?;
            let mut encoder = zstd::stream::Encoder::new(output, lvl)
                .map_err(|e| format!("Cannot create zstd encoder: {e}"))?;
            std::io::copy(&mut input, &mut encoder)
                .map_err(|e| format!("zstd compression failed: {e}"))?;
            let output_file = encoder
                .finish()
                .map_err(|e| format!("zstd compression finalize failed: {e}"))?;
            let size = output_file
                .metadata()
                .map_err(|e| format!("Cannot get output metadata: {e}"))?
                .len();
            Ok((original_size, size))
        })
        .await
        .map_err(|e| MCSError::FilesystemError(format!("Compression task failed: {e}")))?
        .map_err(MCSError::FilesystemError)?;

    let ratio = compute_ratio(original_size, compressed_size);

    Ok(json!({
        "success": true,
        "source": valid_path.to_string_lossy(),
        "output": output_path.to_string_lossy(),
        "algorithm": "zstd",
        "level": level,
        "originalSize": original_size,
        "compressedSize": compressed_size,
        "ratio": ratio,
    }))
}

fn hash_bytes(alg: &str, data: &[u8]) -> std::result::Result<String, String> {
    match alg {
        "sha256" => {
            let mut hasher = Sha256::new();
            hasher.update(data);
            Ok(hex::encode(hasher.finalize()))
        }
        "sha512" => {
            let mut hasher = Sha512::new();
            hasher.update(data);
            Ok(hex::encode(hasher.finalize()))
        }
        "md5" => {
            let mut hasher = md5::Md5::new();
            hasher.update(data);
            Ok(hex::encode(hasher.finalize()))
        }
        "blake3" => Ok(blake3::hash(data).to_hex().to_string()),
        _ => Err(format!("Unsupported hash algorithm: {alg}")),
    }
}

#[allow(clippy::cast_precision_loss)]
fn compute_ratio(original: u64, compressed: u64) -> Option<f64> {
    if original > 0 {
        let cs = compressed as f64;
        let os = original as f64;
        Some((cs / os * 100.0 * 100.0).round() / 100.0)
    } else {
        None
    }
}

pub async fn decompress_zstd(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let output = get_opt_str(args, "output");

    let valid_path = config.sandbox().resolve_path(&path)?;
    let output_path = resolve_decompress_output(&valid_path, output.as_deref(), ".zst", config)?;

    if output_path == valid_path {
        return Err(MCSError::InvalidParams(
            "Output path must differ from source".into(),
        ));
    }

    let src = valid_path.clone();
    let dst = output_path.clone();
    let max_out = config.max_decompressed_size;

    let reader = tokio::io::BufReader::new(
        tokio::fs::File::open(&src)
            .await
            .map_err(|e| MCSError::FilesystemError(format!("Cannot open compressed file: {e}")))?,
    );
    let mut decoder = AsyncZstdDecoder::new(reader);
    let output = tokio::fs::File::create(&dst)
        .await
        .map_err(|e| MCSError::FilesystemError(format!("Cannot create output file: {e}")))?;
    let mut writer = AsyncLimitedWriter::new(tokio::io::BufWriter::new(output), max_out);
    tokio::io::copy(&mut decoder, &mut writer)
        .await
        .map_err(|e| MCSError::FilesystemError(format!("zstd decompression failed: {e}")))?;
    let decompressed_size = writer.written;

    Ok(json!({
        "success": true,
        "source": valid_path.to_string_lossy(),
        "output": output_path.to_string_lossy(),
        "algorithm": "zstd",
        "decompressedSize": decompressed_size,
    }))
}

pub async fn compress_tar(args: Option<&Value>, config: &Config) -> Result<Value> {
    let source = get_str_arg(args, "source")?;
    let output = get_str_arg(args, "output")?;
    let compression = get_opt_str(args, "compression").unwrap_or_else(|| "none".to_string());

    let valid_source = config.sandbox().resolve_path(&source)?;
    let output_path = config.sandbox().resolve_destination_path(&output)?;

    if output_path == valid_source || output_path.starts_with(&valid_source) {
        return Err(MCSError::InvalidParams(
            "Output path must not be inside the source directory".into(),
        ));
    }

    let source_clone = valid_source.clone();
    let output_clone = output_path.clone();
    let comp_clone = compression.clone();
    let follow = config.server.follow_symlinks;
    let result = tokio::task::spawn_blocking(move || {
        let entries = collect_tar_entries(&source_clone, follow)?;
        create_tar_archive(&source_clone, &output_clone, &entries, &comp_clone)
    })
    .await
    .map_err(|e| MCSError::FilesystemError(format!("Tar task failed: {e}")))?
    .map_err(MCSError::FilesystemError)?;

    Ok(json!({
        "success": true,
        "source": valid_source.to_string_lossy(),
        "output": output_path.to_string_lossy(),
        "compression": compression,
        "entries": result.entries,
        "totalSize": result.total_size,
    }))
}

pub async fn decompress_tar(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let output_dir = get_str_arg(args, "outputDir")?;

    let valid_path = config.sandbox().resolve_path(&path)?;
    let output_path = config.sandbox().resolve_destination_path(&output_dir)?;

    let src = valid_path.clone();
    let dst = output_path.clone();
    let max_out = config.max_decompressed_size;
    let result =
        tokio::task::spawn_blocking(move || extract_tar_archive_streaming(&src, &dst, max_out))
            .await
            .map_err(|e| MCSError::FilesystemError(format!("Extract task failed: {e}")))?
            .map_err(MCSError::FilesystemError)?;

    Ok(json!({
        "success": true,
        "source": valid_path.to_string_lossy(),
        "outputDir": output_path.to_string_lossy(),
        "extracted": result.extracted,
        "totalSize": result.total_size,
    }))
}

struct TarResult {
    entries: u64,
    total_size: u64,
}

struct ExtractResult {
    extracted: u64,
    total_size: u64,
}

fn collect_tar_entries(
    source: &std::path::Path,
    follow_symlinks: bool,
) -> std::result::Result<Vec<PathBuf>, String> {
    let mut entries = Vec::new();
    if source.is_dir() {
        let walker = WalkDir::new(source)
            .follow_links(follow_symlinks)
            .into_iter()
            .filter_entry(|e| !is_hidden(e));
        for entry in walker.filter_map(|e| e.ok()) {
            if entry.path() != source {
                entries.push(entry.path().to_path_buf());
            }
        }
    } else {
        entries.push(source.to_path_buf());
    }
    Ok(entries)
}

fn create_tar_archive(
    source: &std::path::Path,
    output: &std::path::Path,
    entries: &[PathBuf],
    compression: &str,
) -> std::result::Result<TarResult, String> {
    let file = std::fs::File::create(output).map_err(|e| format!("Cannot create tar file: {e}"))?;

    let write: Box<dyn std::io::Write> = match compression {
        "gzip" | "gz" => Box::new(GzEncoder::new(file, Compression::default())),
        "zstd" | "zst" => {
            let enc = zstd::stream::Encoder::new(file, 3)
                .map_err(|e| format!("Cannot create zstd encoder: {e}"))?;
            Box::new(enc)
        }
        _ => Box::new(file),
    };

    let mut archive = tar::Builder::new(write);
    let mut total_size = 0u64;

    for path in entries {
        let relative = path.strip_prefix(source).unwrap_or(path);
        if path.is_dir() {
            archive
                .append_dir(relative, path)
                .map_err(|e| format!("Cannot add directory to tar: {e}"))?;
        } else {
            let metadata =
                std::fs::metadata(path).map_err(|e| format!("Cannot read file metadata: {e}"))?;
            total_size += metadata.len();
            let mut file =
                std::fs::File::open(path).map_err(|e| format!("Cannot open file for tar: {e}"))?;
            let mut header = tar::Header::new_ustar();
            header
                .set_path(relative)
                .map_err(|e| format!("Invalid tar path: {e}"))?;
            header.set_size(metadata.len());
            header.set_mtime(
                metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            );
            header.set_mode(metadata.permissions().mode());
            header.set_cksum();
            archive
                .append(&header, &mut file)
                .map_err(|e| format!("Cannot add file to tar: {e}"))?;
        }
    }

    let entries_count = entries.len() as u64;
    let _ = archive
        .into_inner()
        .map_err(|e| format!("Cannot finalize tar: {e}"))?;

    Ok(TarResult {
        entries: entries_count,
        total_size,
    })
}

fn extract_tar_archive_streaming(
    src: &std::path::Path,
    output: &std::path::Path,
    max_total: u64,
) -> std::result::Result<ExtractResult, String> {
    std::fs::create_dir_all(output).map_err(|e| format!("Cannot create output directory: {e}"))?;

    let file = std::fs::File::open(src).map_err(|e| format!("Cannot open tar file: {e}"))?;
    let mut file = std::io::BufReader::new(file);

    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(|e| format!("Cannot read magic bytes: {e}"))?;

    file.seek(std::io::SeekFrom::Start(0))
        .map_err(|e| format!("Cannot seek back: {e}"))?;
    let file = file.into_inner();

    let reader: Box<dyn std::io::Read> = if magic[..3] == [0x1f, 0x8b, 0x08] {
        Box::new(GzDecoder::new(file))
    } else if magic == [0x28, 0xb5, 0x2f, 0xfd] {
        Box::new(
            zstd::stream::Decoder::new(file)
                .map_err(|e| format!("Cannot create zstd decoder: {e}"))?,
        )
    } else {
        Box::new(file)
    };

    let mut archive = tar::Archive::new(reader);
    let mut extracted = 0u64;
    let mut total_size = 0u64;

    for entry in archive
        .entries()
        .map_err(|e| format!("Cannot read tar entries: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("Cannot read tar entry: {e}"))?;

        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err(
                "Tar archive contains symlink/hardlink entries, which are not allowed".to_string(),
            );
        }

        let path = entry
            .path()
            .map_err(|e| format!("Cannot read entry path: {e}"))?
            .to_path_buf();

        let target = if path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(format!("Unsafe tar path: {}", path.display()));
        } else {
            output.join(&path)
        };

        total_size = total_size.saturating_add(entry.size());
        if total_size > max_total {
            return Err(format!(
                "Tar extraction exceeds maximum allowed size of {max_total} bytes"
            ));
        }

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create parent directory: {e}"))?;
        }

        entry
            .unpack(&target)
            .map_err(|e| format!("Cannot unpack tar entry: {e}"))?;
        extracted += 1;
    }

    Ok(ExtractResult {
        extracted,
        total_size,
    })
}

fn resolve_output(
    source: &std::path::Path,
    explicit: Option<&str>,
    extension: &str,
    config: &Config,
) -> Result<PathBuf> {
    if let Some(out) = explicit {
        config.sandbox().resolve_destination_path(out)
    } else {
        let mut result = source.to_path_buf();
        let name = result
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        result.set_file_name(format!("{name}{extension}"));
        Ok(result)
    }
}

fn resolve_decompress_output(
    source: &std::path::Path,
    explicit: Option<&str>,
    extension: &str,
    config: &Config,
) -> Result<PathBuf> {
    if let Some(out) = explicit {
        config.sandbox().resolve_destination_path(out)
    } else {
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let stripped = name.strip_suffix(extension).unwrap_or(&name);
        let mut result = source.to_path_buf();
        result.set_file_name(stripped);
        Ok(result)
    }
}

// ── Helpers ──────────────────────────────────────────────

fn get_str_arg(args: Option<&Value>, name: &str) -> Result<String> {
    args.and_then(|a| a.get(name))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| MCSError::InvalidParams(format!("Missing required parameter: '{name}'")))
}

fn get_opt_str(args: Option<&Value>, name: &str) -> Option<String> {
    args.and_then(|a| a.get(name))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn get_i64_arg(args: Option<&Value>, name: &str) -> Result<i64> {
    args.and_then(|a| a.get(name))
        .and_then(|v| v.as_i64())
        .ok_or_else(|| MCSError::InvalidParams(format!("Missing required parameter: '{name}'")))
}

fn get_opt_i64(args: Option<&Value>, name: &str) -> Option<i64> {
    args.and_then(|a| a.get(name)).and_then(|v| v.as_i64())
}

fn get_opt_bool(args: Option<&Value>, name: &str) -> Option<bool> {
    args.and_then(|a| a.get(name)).and_then(|v| v.as_bool())
}

fn get_opt_str_array(args: Option<&Value>, name: &str) -> Vec<String> {
    args.and_then(|a| a.get(name))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn get_edits_arg(args: Option<&Value>) -> Result<Vec<Value>> {
    args.and_then(|a| a.get("edits"))
        .and_then(|v| v.as_array())
        .cloned()
        .ok_or_else(|| {
            MCSError::InvalidParams("Missing required parameter: 'edits' (array)".into())
        })
}

/// Largest char-boundary index `<= i`.
const fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest char-boundary index `>= i`.
const fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    let n = s.len();
    while i < n && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

fn detect_indent(content: &str) -> &'static str {
    let mut spaces = 0;
    let mut tabs = 0;
    for line in content.lines() {
        if line.starts_with('\t') {
            tabs += 1;
        } else if line.starts_with("  ") {
            spaces += 1;
        }
    }
    if tabs > spaces { "\t" } else { "    " }
}

fn normalize_whitespace(text: &str, indent: &str) -> String {
    if indent == "\t" {
        text.replace("    ", "\t")
    } else {
        text.replace('\t', "    ")
    }
}

/// Exact match against a static set of common binary file extensions.
fn is_binary_extension(ext: &str) -> bool {
    const BINARY_EXTS: &[&str] = &[
        "bin", "exe", "dll", "so", "dylib", "o", "class", "pyc", "jpg", "jpeg", "png", "gif",
        "bmp", "ico", "mp3", "mp4", "avi", "mov", "zip", "tar", "gz", "bz2", "xz", "zst", "7z",
        "rar", "pdf", "wasm",
    ];
    BINARY_EXTS.contains(&ext)
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| s.starts_with('.'))
        .unwrap_or(false)
}

/// Maximum directory recursion depth and total node budget for `directory_tree`,
/// guarding against stack overflow and unbounded output on pathological trees.
const MAX_TREE_DEPTH: usize = 64;
const MAX_TREE_NODES: usize = 100_000;

fn build_tree(
    root: &std::path::Path,
    current: &std::path::Path,
    exclude_globs: &[globset::GlobMatcher],
    depth: usize,
    nodes: &mut usize,
) -> Value {
    let relative = current.strip_prefix(root).unwrap_or(current);
    let relative_str = relative.to_string_lossy();

    if exclude_globs
        .iter()
        .any(|g| g.is_match(relative_str.as_ref()))
    {
        return Value::Null;
    }

    *nodes += 1;
    if *nodes > MAX_TREE_NODES {
        return Value::Null;
    }

    let metadata = match std::fs::metadata(current) {
        Ok(m) => m,
        Err(_) => return Value::Null,
    };

    let name = current
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    if metadata.is_dir() {
        if depth >= MAX_TREE_DEPTH {
            return json!({
                "name": name,
                "type": "directory",
                "children": [],
                "truncated": true,
            });
        }

        let mut children = Vec::new();
        let read_dir = match std::fs::read_dir(current) {
            Ok(d) => d,
            Err(_) => return Value::Null,
        };

        for entry in read_dir.flatten() {
            let path = entry.path();

            if let Some(name_str) = path.file_name().and_then(|n| n.to_str())
                && name_str.starts_with('.')
                && name_str != "."
            {
                continue;
            }

            let child = build_tree(root, &path, exclude_globs, depth + 1, nodes);
            if !child.is_null() {
                children.push(child);
            }
        }

        json!({
            "name": name,
            "type": "directory",
            "children": children,
        })
    } else if metadata.is_file() {
        json!({ "name": name, "type": "file" })
    } else {
        Value::Null
    }
}
