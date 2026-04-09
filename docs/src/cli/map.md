# deagle map

Index a codebase into the graph database.

```bash
deagle map [DIR]
```

**Arguments:**
- `DIR` — Directory to index (default: `.`)

**Behavior:**
1. Recursively walks the directory
2. Skips: hidden dirs, `target/`, `node_modules/`, `vendor/`
3. Detects language from file extension
4. Parses each file with tree-sitter
5. Extracts: functions, structs, enums, traits, methods, constants, imports
6. Stores entities in SQLite graph database

**Example:**
```
$ deagle map /opt/ares
Indexing /opt/ares...
Indexed 90 files, 1247 entities
Database: .deagle/graph.db
```
