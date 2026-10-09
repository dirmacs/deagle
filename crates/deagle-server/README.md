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
POST /api/map              Index a directory into a graph (returns files, entities, edges)
POST /api/sg               Structural AST search
POST /api/rg               Regex text search
```

`GET /api/search` answers from the same index the CLI reads, and — since the
freshness signal landed for the CLI in
[#22](https://github.com/dirmacs/deagle/pull/22) — it now carries that signal
too, so the HTTP path can never silently serve stale coordinates:

- every result row has a `freshness` field: `"fresh"` (the stored hash matches
  the current file bytes), `"stale"` (both exist and differ — re-run
  `POST /api/map`), or `"unknown"` (the verdict could not be derived: the file
  is unreadable, or no hash row was recorded — zero-node files never get one,
  so `unknown` never means fresh and never means stale);
- the response has a `freshness` summary array with one entry per distinct
  cited file (`file_path`, `freshness`, `indexed_at` when a hash row exists,
  `file_mtime` as RFC 3339 when the file can be statted), sorted so `stale`
  and `unknown` files come first. The summary is `null` when nothing matched.

Both fields are additive: every pre-existing field keeps its meaning, and the
answer stays HTTP 200 — the verdict rides in the body, not the status code.

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
