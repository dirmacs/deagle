<p align="center">
  <img src="docs/img/deagle-logo.svg" width="128" alt="deagle — Rust-native Code Intelligence">
</p>

<h1 align="center">deagle</h1>

<p align="center">
  Rust-native code intelligence. Single binary. Runs anywhere Rust runs.<br>
  Built by <a href="https://dirmacs.com">DIRMACS</a>. <strong><a href="https://dirmacs.github.io/deagle">Documentation</a></strong>
</p>

## Features

- **7 language parsers**: Rust, Python, Go, TypeScript/JavaScript, Java, C, C++ (tree-sitter)
- **3 search modes**: substring, fuzzy (skim), keyword (FTS5 BM25)
- **Structural AST search**: `deagle sg '$X.unwrap()'` (ast-grep)
- **Regex text search**: `deagle rg 'TODO|FIXME'` (ripgrep)
- **Incremental indexing**: SHA-256 file hashing, skip unchanged files
- **MCP server**: 6 tools for Claude Code/editor integration (search, keyword, stats, map, sg, rg)
- **HTTP API**: 6 REST endpoints matching CLI
- **LOC counting**: per-language breakdown (tokei)
- **Graph DB**: SQLite WAL mode with nodes, edges, file hashes, FTS5
- **Parallel parsing**: rayon for multi-core indexing, batch inserts
- **Single binary** — runs anywhere Rust runs, zero runtime dependencies

## Performance

Measured with [hyperfine](https://github.com/sharkdp/hyperfine) on a single VPS (release build):

| Codebase | Files | Entities | Edges | Time |
|----------|-------|----------|-------|------|
| dstack (small) | 14 | 296 | 282 | **125ms** |
| deagle (medium) | 15 | 400 | 385 | **339ms** |
| ARES (large) | 94 | 3,486 | 3,392 | **2.2s** |

SQLite WAL mode + batch prepared statements + rayon parallel parsing.

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

## Autonomous agent use cases

Deagle powers code intelligence in autonomous agent sessions using the
[ralph-loop](https://github.com/dirmacs/dstack/tree/main/plugin/skills/ralph-loop)
pattern from [dstack](https://github.com/dirmacs/dstack). The loop rotates
through deagle commands to find work when the main task queue is empty:

| Action | Command | What it finds |
|---|---|---|
| Dead code hunt | `deagle sg "pub fn \$NAME"` + reference check | unused public functions |
| Architecture audit | `deagle search "" --kind struct` | oversized types, SRP violations |
| Coupling analysis | `deagle search "" --kind import` | import hotspots, circular deps |
| Test gap detection | `deagle search <fn>` filtered against test files | untested public APIs |
| Cross-repo impact | `deagle sg` across multiple indexed repos | shared pattern drift |

Graph-first exploration (vs grepping raw files) keeps autonomous agents
from burning tokens on random file reads. A ralph loop running on a
100K-LOC codebase can produce 20+ actionable intel reports per hour
using `deagle stats` → `deagle sg` → `deagle search` pipelines.

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

## DIRMACS ecosystem

| Project | What |
|---------|------|
| [openeruka](https://github.com/dirmacs/openeruka) | OSS self-hosted memory server — SQLite-backed Eruka-compatible backend |
| [eruka-mcp](https://github.com/dirmacs/eruka-mcp) | MCP server for Eruka — anti-hallucination context memory for AI agents |
| [pawan](https://github.com/dirmacs/pawan) | Rust-first CLI coding agent — uses deagle for code intelligence |
| [ares](https://github.com/dirmacs/ares) | Multi-agent runtime — RAG, tool calling, MCP, workflows |
| [dstack](https://github.com/dirmacs/dstack) | Dev stack tooling — project scaffolding, swarm harness, CI audit gates |

## License

MIT
