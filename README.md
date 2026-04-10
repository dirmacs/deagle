<p align="center">
  <img src="docs/img/deagle-logo.svg" width="128" alt="deagle — Rust-native Code Intelligence">
</p>

<h1 align="center">deagle</h1>

<p align="center">
  Rust-native code intelligence. Single binary, no Docker, no external services.<br>
  Built by <a href="https://dirmacs.com">DIRMACS</a>. <strong><a href="https://dirmacs.github.io/deagle">Documentation</a></strong>
</p>

## Features

- **6 language parsers**: Rust, Python, Go, TypeScript/JavaScript, Java, C (tree-sitter)
- **3 search modes**: substring, fuzzy (skim), keyword (FTS5 BM25)
- **Structural AST search**: `deagle sg '$X.unwrap()'` (ast-grep)
- **Regex text search**: `deagle rg 'TODO|FIXME'` (ripgrep)
- **Incremental indexing**: SHA-256 file hashing, skip unchanged files
- **MCP server**: 6 tools for Claude Code/editor integration (search, keyword, stats, map, sg, rg)
- **HTTP API**: 6 REST endpoints matching CLI
- **LOC counting**: per-language breakdown (tokei)
- **Graph DB**: SQLite with nodes, edges, file hashes, FTS5
- **Parallel parsing**: rayon for multi-core indexing
- **Single binary** — no Docker, no external services

## Install

```bash
cargo install deagle
```

## Usage

```bash
# Index a codebase (incremental — skips unchanged files)
deagle map /path/to/project

# Force full re-index
deagle map . --force

# Search for symbols
deagle search "handler"
deagle search "Config" --kind struct
deagle search "proc" --fuzzy          # fuzzy match (skim)

# Structural AST search (ast-grep patterns)
deagle sg '$X.unwrap()'               # find all unwrap calls
deagle sg 'fn $NAME($$$) { $$$ }'     # find all functions
deagle sg 'struct $S { $$$FIELDS }'   # find all structs

# Regex text search (ripgrep)
deagle rg 'TODO|FIXME' --lang rust

# Lines of code
deagle loc .

# Graph statistics
deagle stats
```

## Architecture

```
deagle-core    — Graph types + SQLite storage + fuzzy/FTS5 search + incremental hashing
deagle-parse   — Tree-sitter parsers (Rust, Python, Go, TypeScript/JS) + ast-grep + ripgrep
deagle     — CLI: map, search, sg, rg, loc, stats (6 commands)
deagle-server  — HTTP API (Axum) + MCP server (rmcp) for editor integration
```

## MCP Server

For Claude Code, Cursor, or any MCP-compatible editor:

```bash
deagle-mcp  # stdio transport
```

Tools: `deagle_search`, `deagle_stats`, `deagle_map`, `deagle_sg`, `deagle_rg`

## Supported Languages

| Language | Parser | Entities |
|----------|--------|----------|
| Rust | tree-sitter-rust | functions, methods, structs, enums, traits, imports, constants, modules |
| Python | tree-sitter-python | functions, methods, classes, imports, constants, decorators |
| Go | tree-sitter-go | functions, methods, structs, interfaces, type aliases, imports, constants |
| TypeScript/JS | tree-sitter-typescript | functions, arrow functions, methods, classes, interfaces, enums, type aliases, imports |

## License

MIT
