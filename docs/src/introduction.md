# Deagle

Rust-native code intelligence. Single binary, no Docker, no external services.

Deagle indexes your codebase into a queryable graph using tree-sitter parsing and SQLite storage. Search symbols, trace relationships, and analyze architecture — all from a single binary.

## Why Deagle?

| | Deagle | ix |
|---|---|---|
| Language | Rust | TypeScript + Python |
| Storage | SQLite (embedded) | ArangoDB (Docker) |
| Memory | ~10MB | ~3GB |
| Dependencies | None | Docker, ArangoDB, Node.js |
| Install | `cargo install deagle-cli` | Docker Compose |

## Architecture

```
deagle-core    — Graph types (Node, Edge) + SQLite storage
deagle-parse   — Tree-sitter parsers per language
deagle-cli     — CLI commands (map, search, stats)
```

Built by [DIRMACS](https://dirmacs.com). Part of the DIRMACS open-source Rust stack.
