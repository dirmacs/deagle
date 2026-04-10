# deagle-cli

CLI for [deagle](https://github.com/dirmacs/deagle) code intelligence.

## Install

```bash
cargo install deagle-cli
```

## Commands

```
deagle map [DIR] [--force]     Index a codebase (incremental by default)
deagle search QUERY [--fuzzy]  Search entities by name
deagle sg PATTERN              Structural AST search (ast-grep)
deagle rg PATTERN [--lang L]   Regex text search (ripgrep)
deagle loc [DIR]               Count lines of code (tokei)
deagle stats                   Show graph statistics
```

## Examples

```bash
deagle map .                          # incremental index
deagle map . --force                  # full re-index
deagle search "Config" --fuzzy        # fuzzy search
deagle sg '$X.unwrap()'               # find all unwrap calls
deagle rg "TODO|FIXME" --lang rust    # find TODOs in Rust files
deagle loc .                          # LOC by language
```

## License

MIT
