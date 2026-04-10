# deagle Plugin

Rust-native code intelligence for AI coding assistants.

## Commands

- `deagle map [DIR] [--force]` — index a codebase (incremental)
- `deagle search QUERY [--fuzzy] [--kind K]` — search entities
- `deagle sg PATTERN` — structural AST search (ast-grep)
- `deagle rg PATTERN [--lang L]` — regex text search (ripgrep)
- `deagle loc [DIR]` — lines of code (tokei)
- `deagle stats` — graph statistics

## Languages

Rust, Python, Go, TypeScript/JavaScript

## Search Modes

Substring, fuzzy (skim), keyword (FTS5 BM25), structural (ast-grep), regex (ripgrep)

## MCP Tools

`deagle_search`, `deagle_stats`, `deagle_map`, `deagle_sg`, `deagle_rg`
