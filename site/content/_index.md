+++
title = "deagle — Rust-native Code Intelligence"
template = "index.html"
+++

# deagle

Rust-native code intelligence. Single binary, no Docker, no external services.

## What's New in v0.1.1

- **4 language parsers**: Rust, Python, Go, TypeScript/JavaScript
- **Incremental indexing**: SHA-256 file hashing — skips unchanged files on re-index
- **FTS5 keyword search**: BM25-ranked full-text search across entity names and content
- **Fuzzy search**: skim-powered fuzzy matching with score ranking
- **MCP server**: 5 tools for Claude Code and editor integration
- **Structural AST search**: ast-grep patterns like `$X.unwrap()` or `fn $NAME() { $$$ }`
- **Regex text search**: ripgrep-powered fast text search with language filtering

## Install

```bash
cargo install deagle-cli
```

## Quick Start

```bash
deagle map .                    # index your project
deagle search "Config" --fuzzy  # fuzzy search
deagle sg '$X.unwrap()'         # find unwrap calls
deagle rg 'TODO' --lang rust    # find TODOs
deagle loc .                    # count lines of code
```

## Architecture

| Crate | Purpose |
|-------|---------|
| [deagle-core](https://crates.io/crates/deagle-core) | Graph types, SQLite storage, search (substring/fuzzy/FTS5) |
| [deagle-parse](https://crates.io/crates/deagle-parse) | Tree-sitter parsers + ast-grep + ripgrep |
| [deagle-cli](https://crates.io/crates/deagle-cli) | CLI binary with 6 commands |
| [deagle-server](https://crates.io/crates/deagle-server) | HTTP API + MCP server |

Built by [DIRMACS](https://dirmacs.com). MIT licensed.
