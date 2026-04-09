# deagle search

Search for symbols by name.

```bash
deagle search <QUERY> [--kind KIND]
```

**Arguments:**
- `QUERY` — Substring to search for (case-insensitive)
- `--kind` — Filter by entity kind: `function`, `struct`, `enum`, `trait`, `method`, `constant`, `module`, `import`

**Example:**
```
$ deagle search "Config" --kind struct
NAME                           KIND         LANG       LOCATION
--------------------------------------------------------------------------------
Config                         struct       rust       src/config.rs:15
AppConfig                      struct       rust       src/app.rs:42

2 result(s)
```
