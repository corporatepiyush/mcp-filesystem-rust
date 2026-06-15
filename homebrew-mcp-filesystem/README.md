# Homebrew Tap for mcp-filesystem

This directory contains the Homebrew formula for installing mcp-filesystem.

## Installation

### Option 1: From Local Repository (Development)

If you have cloned the mcp-filesystem repository:

```bash
brew tap corporatepiyush/mcp-filesystem /path/to/mcp-filesystem/homebrew-mcp-filesystem
brew install mcp-filesystem
```

### Option 2: From Separate Tap Repository (When Available)

Once published to a separate Homebrew tap:

```bash
brew tap corporatepiyush/mcp-filesystem
brew install mcp-filesystem
```

### Option 3: Direct Install (from source)

```bash
cargo install --git https://github.com/corporatepiyush/mcp-filesystem-rust
```

## Update

To update to the latest version:

```bash
brew upgrade mcp-filesystem
```

## Uninstall

```bash
brew uninstall mcp-filesystem
brew untap corporatepiyush/mcp-filesystem
```

## Development

To test the formula locally:

```bash
cd homebrew-mcp-filesystem
brew tap corporatepiyush/mcp-filesystem .
brew install corporatepiyush/mcp-filesystem/mcp_filesystem
```
