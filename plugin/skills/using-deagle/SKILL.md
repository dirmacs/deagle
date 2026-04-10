---
name: using-deagle
description: Deagle code intelligence — index, search, and analyze codebases with tree-sitter parsing, fuzzy/FTS5 search, ast-grep patterns
---

# Using deagle

Rust-native code intelligence. 4 language parsers, 3 search modes, incremental indexing.

## Commands

| Command | Description |
|---------|-------------|
| `deagle map [DIR] [--force]` | Index a codebase (incremental by default) |
| `deagle search QUERY [--fuzzy] [--kind K]` | Search entities (substring, fuzzy, or by kind) |
| `deagle sg PATTERN` | Structural AST search (ast-grep patterns) |
| `deagle rg PATTERN [--lang L]` | Regex text search (ripgrep) |
| `deagle loc [DIR]` | Lines of code by language (tokei) |
| `deagle stats` | Graph database statistics |

## Supported Languages

Rust, Python, Go, TypeScript/JavaScript

## Search Modes

- **Substring**: `deagle search "handler"` — case-insensitive LIKE match
- **Fuzzy**: `deagle search "hndlr" --fuzzy` — skim-powered fuzzy ranking
- **Keyword (FTS5)**: Full-text BM25 search across names + content (via GraphDb::keyword_search)
- **Structural**: `deagle sg '$X.unwrap()'` — ast-grep AST patterns
- **Regex**: `deagle rg 'TODO|FIXME'` — ripgrep file content search

## MCP Tools

`deagle_search`, `deagle_stats`, `deagle_map`, `deagle_sg`, `deagle_rg`

## Tips

- First run `deagle map .` to build the index
- Use `--fuzzy` when you're not sure of the exact name
- Use `deagle sg` for structural patterns (find all unwraps, find all impl blocks)
- Re-running `deagle map .` only re-indexes changed files (incremental)
