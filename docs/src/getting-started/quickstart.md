# Quickstart

```bash
# Index your project
deagle map /path/to/project

# Search for symbols
deagle search "handler"
deagle search "Config" --kind struct

# View stats
deagle stats
```

The database is stored at `.deagle/graph.db` by default. Override with `--db path/to/db`.
