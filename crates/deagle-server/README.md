# deagle-server

HTTP API + MCP server for [deagle](https://github.com/dirmacs/deagle) code intelligence.

## Binaries

- `deagle-serve` — Axum HTTP API (default port 3500)
- `deagle-mcp` — MCP server for Claude Code/editor integration (stdio)

## HTTP API

```
GET  /health              Health check
GET  /api/search?q=NAME   Search entities
GET  /api/stats            Graph statistics
POST /api/map              Index a directory
POST /api/sg               Structural AST search
POST /api/rg               Regex text search
```

## MCP Tools

| Tool | Description |
|------|-------------|
| `deagle_search` | Search code entities by name |
| `deagle_stats` | Graph database statistics |
| `deagle_map` | Index a codebase |
| `deagle_sg` | Structural AST pattern search |
| `deagle_rg` | Regex text search |

## Usage

```bash
# HTTP server
DEAGLE_PORT=3500 deagle-serve

# MCP server (Claude Code)
deagle-mcp
```

## License

MIT
