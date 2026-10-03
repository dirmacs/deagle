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
deagle rg PATTERN [--lang L]     Regex text search (ripgrep)
                             [--column] [--json]
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
deagle rg "TODO" --column              # file:line:column: text
deagle rg "TODO" --json                # one JSON object per match
deagle loc .                          # LOC by language
```

## Match locations

`deagle rg` reports where each match landed:

- `--column` prints `file:line:column: text`. The column is 1-indexed and
  counted in characters, like `line`.
- `--json` prints one JSON object per match with `file_path`, `line_number`,
  `column`, `byte_offset` and `line`. `byte_offset` counts **bytes** from the
  start of the file, so `content[byte_offset..]` starts at the match — it is
  not a column.

## License

MIT
