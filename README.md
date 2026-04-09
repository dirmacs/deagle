# deagle

Rust-native code intelligence. Single binary, no Docker, no external services.

Built by [DIRMACS](https://dirmacs.com). Replaces [ix](https://github.com/ix-infrastructure/Ix) with a pure Rust implementation.

## Features

- Tree-sitter based multi-language parsing (Rust first, Python/Go/TS next)
- SQLite-backed code graph (functions, structs, traits, imports, calls, edges)
- CLI commands: `map`, `search`, `stats`
- Single binary — no Docker, no ArangoDB, no external services
- Uses DIRMACS OSS stack: `ares-vector` for semantic search (planned)

## Usage

```bash
# Index a codebase
deagle map /path/to/project

# Search for symbols
deagle search "handler"
deagle search "Config" --kind struct

# View graph statistics
deagle stats
```

## Architecture

```
deagle-core    — Graph types (Node, Edge, NodeKind) + SQLite storage
deagle-parse   — Tree-sitter parsers per language
deagle-cli     — CLI commands (map, search, stats)
```

## License

MIT
