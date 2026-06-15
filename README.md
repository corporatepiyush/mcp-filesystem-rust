# mcp-filesystem

[![Crates.io](https://img.shields.io/crates/v/mcp-filesystem.svg)](https://crates.io/crates/mcp-filesystem)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

A high-performance [Model Context Protocol (MCP)](https://modelcontextprotocol.io) server for filesystem access, written in Rust on the Tokio async runtime.

It exposes a rich set of filesystem tools — reads, writes, edits, search, hashing, compression, encryption, and CSV manipulation — over **stdio**, **TCP** (JSON-RPC), and **HTTP/2 + SSE** transports, all behind a strict path sandbox.

## Features

- **Parallel async I/O** built on Tokio with the `mimalloc` allocator and zero-copy memory-mapped reads.
- **Secure path sandboxing** — every path is validated against an allow-list (via a `PathTrie`) with symlink-escape protection.
- **Multiple transports** — stdio (for MCP clients), TCP JSON-RPC, and HTTP/2 with Server-Sent Events.
- **Access modes** — `unrestricted` or `readonly` (write tools are rejected in readonly mode).
- **41 tools**, including:
  - **Files**: read/write/edit, copy/move/delete, directory listing & trees, metadata, permissions, disk usage, symlinks, ranged reads.
  - **Search**: glob `search_files` and content `grep_files`.
  - **Hashing**: SHA-256, SHA-512, BLAKE3, MD5.
  - **Compression**: gzip, zstd, and tar archives.
  - **Encryption**: AES-256-GCM, ChaCha20-Poly1305, RSA-OAEP, and post-quantum ML-KEM; plus key generation.
  - **CSV**: create/read and row/column/cell manipulation with ranged reads.
- **Media detection** via content inspection and magic-byte sniffing.

## Installation

From [crates.io](https://crates.io/crates/mcp-filesystem):

```sh
cargo install mcp-filesystem
```

Or build from source:

```sh
git clone https://github.com/corporatepiyush/mcp-filesystem-rust
cd mcp-filesystem-rust
cargo build --release
```

## Usage

### stdio mode (for MCP clients)

```sh
mcp-filesystem --stdio --directories /path/to/allowed/dir
```

### Network mode (TCP + HTTP)

```sh
mcp-filesystem --directories /path/to/allowed/dir --port 3000 --http-port 3001
```

### Example MCP client configuration

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "mcp-filesystem",
      "args": ["--stdio", "--directories", "/path/to/allowed/dir"]
    }
  }
}
```

### CLI options

| Flag | Default | Description |
|---|---|---|
| `-d, --directories <DIR>` | — | Directories to allow access to (repeatable) |
| `-H, --host <HOST>` | `127.0.0.1` | Server host |
| `-p, --port <PORT>` | `3000` | TCP server port |
| `--http-port <PORT>` | `3001` | HTTP server port |
| `-l, --log-level <LEVEL>` | `info` | Log level |
| `--max-file-size <MB>` | `100` | Maximum file size (MB) for reads |
| `--stdio` | `false` | Run in stdio mode for MCP compatibility |
| `--access-mode <MODE>` | `unrestricted` | `unrestricted` or `readonly` |
| `--follow-symlinks` | `false` | Follow symbolic links |
| `--request-timeout <SECS>` | `30` | Request timeout in seconds |

## Development

```sh
cargo build      # Build all targets
cargo test       # Run the full test suite (34 unit + 50 integration)
cargo clippy     # Zero-warnings lint check
```

## License

Licensed under the [Apache-2.0](LICENSE) license.
