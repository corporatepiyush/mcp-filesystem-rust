# AGENTS.md — mcp-filesystem

## Project Overview

High-performance Rust MCP (Model Context Protocol) server for filesystem access with parallel async I/O, glob search, MIME detection, encryption, CSV manipulation, compression, and secure path sandboxing. Edition 2024, strict clippy.

## Build & Test

| Command | Purpose |
|---|---|
| `cargo build` | Build all targets |
| `cargo test` | Run all 84 tests (34 unit + 50 integration) |
| `cargo clippy` | Zero-warnings lint check |
| `cargo test --test integration <test_name>` | Single integration test |

## Project Structure

```
src/
├── main.rs              # Binary entry point
├── lib.rs               # Library root, Args struct (clap), mod declarations
├── config.rs            # Config, AccessMode, ServerConfig, CLI arg parsing
├── errors.rs            # MCSError enum, Result<T> alias
├── protocol.rs          # JsonRpcRequest/Response serde
├── server.rs            # TCP+stdio server loop, JSON-RPC dispatch
├── http.rs              # HTTP/2 + SSE transport
├── tools.rs             # Tool registry (tools.json), is_write_tool
├── structures.rs        # Custom data structures (PathTrie, RingBuffer, LruCache, SortedVec, BloomFilter) + utility algorithms
├── validation.rs        # Path sandboxing via PathTrie + symlink checks
└── actions/
    ├── mod.rs           # Re-exports action modules
    ├── files.rs         # Core file ops (read/write/edit/move/copy/delete/grep/search/tail/head/archive/hash)
    ├── csv.rs           # CSV operations (create/read/update/add/remove rows & columns)
    └── crypto.rs        # Encryption/decryption (AES-256-GCM, ChaCha20-Poly1305, ML-KEM)
tests/
└── integration.rs       # 41 integration tests
```

## Dependency Guidelines

### Current crate choices (don't change without rationale)

| Category | Crate | Reason |
|---|---|---|
| Async runtime | `tokio` (full) | Industry standard |
| HTTP/2 + SSE | `axum` + `hyper` | Modern async HTTP |
| Encryption | `aes-gcm`, `chacha20poly1305`, `ml-kem` | Symmetric AEAD + pure-Rust post-quantum KEM (FIPS 203) |
| Hashing | `sha2`, `blake3`, `md-5` | Standard hashing |
| Zero-copy I/O | `memmap2` | DMA-like memory-mapped file reads (sendfile-style) |
| CSV | `csv` | De facto standard |
| Compression | `flate2`, `zstd`, `tar` | Gzip + Zstd + tar |
| MIME detection | `infer` (magic bytes) | Active maintenance, accurate |
| Logging | `tracing` + `tracing-subscriber` | Structured async logging |
| CLI | `clap` (derive) | Idiomatic Rust CLI |
| Path validation utils | `walkdir`, `globset`, `filetime`, `same-file` | Filesystem traversal |
| Errors | `thiserror`, `anyhow` | Idiomatic error handling |
| Allocator | `mimalloc` | Performance |

### Removed / Replaced

| Old | New | Reason |
|---|---|---|
| `pqcrypto-kyber` | `pqcrypto-mlkem` | RUSTSEC-2024-0381, Kyber → ML-KEM standard |
| `pqcrypto-mlkem`, `pqcrypto-traits` | `ml-kem` | RUSTSEC-2026-0161/0162/0163 (PQClean archived/unmaintained); pure Rust, no C/FFI build dep, FIPS 203 wire-compatible |
| `rsa` (RSA-OAEP) | — (removed) | Deprecated by CNSA 2.0 / NIST IR 8547 for 2026; only source of RUSTSEC-2023-0071 (Marvin, no fix). PQ KEM (ML-KEM) is the replacement. Hybrid X-Wing pending stable crate |
| `once_cell` | `std::sync::LazyLock` | Stabilized in std (edition 2024) |
| `mime_guess` | `infer` | Unmaintained, infer uses magic bytes |

## Conventions

### Code style
- **Edition 2024**, no `extern crate`.
- **No comments in code** unless documenting public API intent. Avoid inline `//` comments on implementation.
- **Use `json!` macro** from serde_json for response construction.
- **`Result<T>` is `std::result::Result<T, MCSError>`** (defined in `errors.rs`).
- **`get_str_arg`, `get_opt_str`** helpers from `files.rs` for arg extraction.

### Clippy
- Lint config in `Cargo.toml` `[lints.clippy]` — ~30 explicit warnings. Maintain zero warnings.
- Run `cargo clippy` before every commit.

### Action functions
- All action functions are `pub async fn action_name(args: Option<&Value>, config: &Config) -> Result<Value>`.
- Extract args at top, validate path immediately via `validation::validate_path`.
- Return `json!({ ... })` wrapped in `Ok(...)`.

### Error handling
- Use `MCSError` variants exclusively (no raw `String` errors in public API).
- `FilesystemError(String)` for general FS failures.
- `InvalidParams(String)` for bad args.
- `PathNotAllowed` / `PathNotFound` for path validation failures.
- Use `.map_err(|e| MCSError::FilesystemError(format!(...)))` to wrap external errors.

### Path validation
- Always call `validation::validate_path(path, &config.allowed_directories, config.server.follow_symlinks)` as first step.
- Uses a `PathTrie` for efficient prefix matching.
- When `follow_symlinks=false`, every path component is checked for symlinks before canonicalization.

### Testing
- Unit tests inline in each module under `#[cfg(test)] mod tests { ... }`.
- Integration tests in `tests/integration.rs` — filesystem-based, run in temp directories.
- Use `#[tokio::test]` for async tests.

### Release profile
- `opt-level=3`, `lto="fat"`, `codegen-units=1`, `strip=true`, `panic="abort"`.
