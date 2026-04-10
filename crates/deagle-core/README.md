# deagle-core

Core types and SQLite graph storage for [deagle](https://github.com/dirmacs/deagle) code intelligence.

## Features

- **Graph model**: nodes (functions, classes, modules) + edges (calls, imports, contains, inherits)
- **SQLite storage**: `GraphDb` with insert, search, fuzzy search (skim), keyword search (FTS5)
- **Incremental indexing**: SHA-256 file hashing, skip unchanged files on re-index
- **Semantic search**: optional `ares-vector` integration (`semantic` feature flag)

## Usage

```rust
use deagle_core::{GraphDb, Node, NodeKind, Language};

let db = GraphDb::open(Path::new("graph.db"))?;
db.insert_node(&node)?;
let results = db.search_nodes("process")?;        // substring
let fuzzy = db.fuzzy_search_nodes("proc")?;        // fuzzy (skim)
let keywords = db.keyword_search("authenticate")?; // FTS5 BM25
```

## License

MIT
