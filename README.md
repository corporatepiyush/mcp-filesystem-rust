# mcp-filesystem

[![Crates.io](https://img.shields.io/crates/v/mcp-filesystem.svg)](https://crates.io/crates/mcp-filesystem)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

A high-performance [Model Context Protocol (MCP)](https://modelcontextprotocol.io) server for filesystem access, written in Rust on the Tokio async runtime.

It exposes a rich set of filesystem tools — reads, writes, edits, search, hashing, compression, encryption, and CSV manipulation — over **stdio** and **HTTP** (`POST /rpc`) transports, all behind a strict path sandbox.

> **Tools are opt-in (2.2.0+).** No tools are exposed by default. Enable them one
> *category* at a time with `--enable-<category>` flags (or `--enable-all`). A
> server started with no enable flags advertises an **empty** tool list. See
> [Tool Exposure](#tool-exposure-opt-in-by-category).

> **TCP removed (2.2.0+).** The line-delimited TCP transport has been dropped;
> use **stdio** (for MCP clients) or **HTTP**.

> **MCP suite.** One of four high-performance MCP servers written in Rust —
> [mcp-postgres](https://github.com/corporatepiyush/mcp-pg-rust) ·
> [mcp-filesystem](https://github.com/corporatepiyush/mcp-filesystem-rust) ·
> [mcp-memory](https://github.com/corporatepiyush/mcp-memory) ·
> [mcp-web-search](https://github.com/corporatepiyush/mcp-web-search).
> All implement MCP protocol revision **`2025-11-25`**.

## Features

- **Parallel async I/O** built on Tokio with the `mimalloc` allocator and zero-copy memory-mapped reads.
- **Secure path sandboxing** — every path is validated against an allow-list (via a `PathTrie`) with symlink-escape protection.
- **Two transports** — stdio (for MCP clients) and an HTTP JSON-RPC endpoint (`POST /rpc`, `GET /health`).
- **Access modes** — `unrestricted` or `readonly` (write tools are rejected in readonly mode).
- **41 tools**, including:
  - **Files**: read/write/edit, copy/move/delete, directory listing & trees, metadata, permissions, disk usage, symlinks, ranged reads.
  - **Search**: glob `search_files` and content `grep_files`.
  - **Hashing**: SHA-256, SHA-512, BLAKE3, MD5.
  - **Compression**: gzip, zstd, and tar archives.
  - **Encryption**: AES-256-GCM, ChaCha20-Poly1305, and post-quantum ML-KEM-768/1024 (NIST FIPS 203); plus key generation.
  - **CSV**: create/read and row/column/cell manipulation with ranged reads.
- **Media detection** via content inspection and magic-byte sniffing.

## Installation

From [crates.io](https://crates.io/crates/mcp-filesystem):

```sh
cargo install mcp-filesystem
```

From Homebrew (macOS):

```sh
brew tap corporatepiyush/mcp-filesystem
brew install mcp-filesystem
```

> The Homebrew formula lives in [`homebrew-mcp-filesystem/`](homebrew-mcp-filesystem/). See its
> [README](homebrew-mcp-filesystem/README.md) for tapping from a local checkout or a dedicated tap repository.

Or build from source:

```sh
git clone https://github.com/corporatepiyush/mcp-filesystem-rust
cd mcp-filesystem-rust
cargo build --release
```

## Usage

Tools are opt-in — pass one or more `--enable-<category>` flags (or
`--enable-all`). Without them the server exposes no tools.

### stdio mode (for MCP clients)

```sh
mcp-filesystem --stdio --directories /path/to/allowed/dir --enable-read --enable-write
```

### HTTP mode

```sh
mcp-filesystem --directories /path/to/allowed/dir --http-port 3001 --enable-all
```

### Example MCP client configuration

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "mcp-filesystem",
      "args": ["--stdio", "--directories", "/path/to/allowed/dir", "--enable-all"]
    }
  }
}
```

### CLI options

| Flag | Default | Description |
|---|---|---|
| `-d, --directories <DIR>` | — | Directories to allow access to (repeatable) |
| `-H, --host <HOST>` | `127.0.0.1` | Server host (HTTP transport) |
| `--http-port <PORT>` | `3001` | HTTP server port |
| `-l, --log-level <LEVEL>` | `info` | Log level |
| `--max-file-size <MB>` | `100` | Maximum file size (MB) for reads |
| `--stdio` | `false` | Run in stdio mode for MCP compatibility |
| `--access-mode <MODE>` | `unrestricted` | `unrestricted` or `readonly` |
| `--follow-symlinks` | `false` | Follow symbolic links |
| `--request-timeout <SECS>` | `30` | Request timeout in seconds (enforced per request) |
| `--max-decompressed-size <MB>` | `1024` | Cap on decompression/extraction output (anti-bomb) |
| `--max-request-bytes <BYTES>` | `16777216` | Max size of a single stdio request line |
| `--auth-token <TOKEN>` | — | Bearer token required on HTTP (`Authorization` header) |
| `--tls-cert <PATH>` | — | PEM certificate chain to serve the HTTP transport over TLS (HTTPS). Requires `--tls-key` |
| `--tls-key <PATH>` | — | PEM private key matching `--tls-cert` |
| **Tool exposure** | | *(none enabled by default)* |
| `--enable-all` | `false` | Expose every category (overrides the flags below) |
| `--enable-read` | `false` | **Read**: read files, list/search/stat, hashes, disk usage |
| `--enable-write` | `false` | **Write**: write/edit, create dir, move/copy, perms, symlink |
| `--enable-delete` | `false` | **Delete**: delete file/directory |
| `--enable-compress` | `false` | **Compress**: gzip, zstd, tar (de)compression |
| `--enable-crypto` | `false` | **Crypto**: encrypt/decrypt files, key generation |
| `--enable-csv` | `false` | **CSV**: CSV read/write helpers |

### TLS (HTTPS)

The HTTP transport can be served over TLS (rustls, `ring` provider). Provide a
PEM certificate chain and private key — via `--tls-cert`/`--tls-key` or the
`MCP_TLS_CERT`/`MCP_TLS_KEY` environment variables — and the HTTP server speaks
HTTPS instead of plaintext. The two must be supplied together; otherwise the
server refuses to start. When neither is set, the HTTP transport stays plaintext
(the default).

### Tool Exposure (opt-in by category)

Every tool belongs to one of **6 categories**. **Nothing is exposed until you
enable its category** — disabled tools are hidden from `tools/list` and rejected
from `tools/call` as if they did not exist. This lets you grant an agent exactly
the surface area it needs (e.g. read-only `--enable-read`).

| Flag | Category | Tools |
|------|----------|-------|
| `--enable-read` | **Read** | `read_text_file`, `read_media_file`, `read_file_range`, `list_directory`, `list_directory_with_sizes`, `directory_tree`, `get_file_info`, `search_files`, `grep_files`, `hash_file`, `get_disk_usage`, `list_allowed_directories` |
| `--enable-write` | **Write** | `write_file`, `edit_file`, `create_directory`, `move_file`, `copy_file`, `set_permissions`, `create_symlink` |
| `--enable-delete` | **Delete** | `delete_file`, `delete_directory` |
| `--enable-compress` | **Compress** | `compress_gzip`/`decompress_gzip`, `compress_zstd`/`decompress_zstd`, `compress_tar`/`decompress_tar` |
| `--enable-crypto` | **Crypto** | `encrypt_file`, `decrypt_file`, `generate_key` |
| `--enable-csv` | **CSV** | `csv_create`, `csv_read`, `csv_add_row`, `csv_update_cell`, `csv_remove_row`, `csv_add_column`, `csv_remove_column`, `csv_rename_column`, `csv_read_column_values_range`, `csv_read_row_range`, `csv_select_column_range` |
| `--enable-all` | *(all)* | Every category. Overrides the individual flags. |

Category gating composes with `--access-mode readonly` (which additionally
blocks all write tools).

```bash
mcp-filesystem --http-port 3001 --tls-cert ./cert.pem --tls-key ./key.pem
```

## MCP Compliance

Implements the [Model Context Protocol](https://modelcontextprotocol.io) revision **`2025-11-25`** over JSON-RPC 2.0, via stdio or HTTP.

| Area | Support |
|---|---|
| Transports | stdio, HTTP (`POST /rpc`) |
| Protocol version | `2025-11-25`, negotiates down to `2025-06-18` / `2025-03-26` / `2024-11-05` |
| `initialize` | ✅ version negotiation + `instructions` |
| `tools/list`, `tools/call` | ✅ (41 tools) |
| `CallToolResult` | ✅ `content[]` + `structuredContent` + `isError`; `read_media_file` returns typed `image`/`audio` content |
| Capabilities advertised | `tools` only — nothing is advertised that isn't implemented |
| `resources` · `prompts` · `logging` · Streamable HTTP | ❌ roadmap — see [MIGRATION.md](./MIGRATION.md) |

Every `tools/call` returns a spec-compliant `CallToolResult`. The payload is
available as a machine-readable `structuredContent` object and as serialized
`text`; tool failures come back with `isError: true` (not as JSON-RPC protocol
errors) so the model can self-correct.

```json
{
  "content": [{ "type": "text", "text": "{\"content\":\"Hello, World!\",\"totalLines\":1}" }],
  "structuredContent": { "content": "Hello, World!", "totalLines": 1 },
  "isError": false
}
```

Upgrading from 1.x? The result shape changed — see **[MIGRATION.md](./MIGRATION.md)**.

## Security

- **Path sandboxing**: every path is canonicalized and checked against the allow-list. Symlink components are rejected unless `--follow-symlinks` is set; write destinations whose final component is a symlink are also rejected.
- **Network exposure**: the default bind is loopback (`127.0.0.1`). Binding to a non-loopback host without `--auth-token` logs a prominent warning — the allow-listed directories would otherwise be reachable, unauthenticated, over the network.
- **Resource limits**: request lines are size-capped (`--max-request-bytes`), requests are time-bounded (`--request-timeout`), decompression output is capped (`--max-decompressed-size`), and concurrent connections are bounded (`--max-connections`).
- **Cryptography posture (June 2026)**: the supported algorithms are deliberately limited to AES-256-GCM, ChaCha20-Poly1305, and post-quantum ML-KEM-768/1024 (FIPS 203). RSA-OAEP was removed — it is being deprecated by [CNSA 2.0](https://www.nsa.gov/Cybersecurity/Post-Quantum-Cryptography/) and [NIST IR 8547](https://csrc.nist.gov/pubs/ir/8547/ipd) and was the project's only source of an unfixable advisory (the `rsa` Marvin timing side-channel, [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)). `cargo audit` is clean with no acknowledged advisories. A hybrid `X25519 + ML-KEM-768` (X-Wing) mode is planned once a stable, audited Rust implementation is available — the current `x-wing` crate is still a pre-release.

## Development

```sh
cargo build      # Build all targets
cargo test       # Run the full test suite (unit + integration)
cargo clippy     # Zero-warnings lint check
```

## Versioning & Compatibility

Follows [Semantic Versioning](https://semver.org). The current line is **2.x**,
targeting MCP revision `2025-11-25`. The `2.0.0` release changed the `tools/call`
result shape to be spec-compliant — see **[MIGRATION.md](./MIGRATION.md)**.

| mcp-filesystem | MCP revision (default) | Negotiates |
|---|---|---|
| 2.x | `2025-11-25` | `2025-06-18`, `2025-03-26`, `2024-11-05` |
| ≤ 1.x | `2024-11-05` | — |

## License

Licensed under the [Apache-2.0](LICENSE) license.
