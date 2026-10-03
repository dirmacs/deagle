# deagle-parse

Multi-language tree-sitter code parser for [deagle](https://github.com/dirmacs/deagle).

## Supported Languages

| Language | Crate | Entities |
|----------|-------|----------|
| Rust | tree-sitter-rust | functions, methods, structs, enums, traits, imports, constants, modules |
| Python | tree-sitter-python | functions, methods, classes, imports, constants (UPPER_CASE), decorators |
| Go | tree-sitter-go | functions, methods, structs, interfaces, type aliases, imports, constants |
| TypeScript/JS | tree-sitter-typescript | functions, arrow functions, methods, classes, interfaces, enums, type aliases, imports |

## Features

> **Feature flags:** `deagle-parse` ships with **no default features**. Enable `pattern` for ast-grep structural search and `text-search` for ripgrep-grade regex search. Without either, only tree-sitter parsing is available — `deagle_parse::pattern` and `deagle_parse::text_search` are `#[cfg]`-gated, so a consumer that forgets them gets an `unresolved import` error rather than a silent no-op.

| Feature | Default | Enables |
|---------|---------|---------|
| `pattern` | off | structural AST search via ast-grep (`search_pattern`) |
| `text-search` | off | regex text search via ripgrep library crates (`text_search`) |

```toml
[dependencies]
# tree-sitter parsing only
deagle-parse = "0.3"
# ...plus structural and regex search
deagle-parse = { version = "0.3", features = ["pattern", "text-search"] }
```

## Usage

```rust
use deagle_parse::{parse_file, parse_file_with_edges};
use deagle_core::Language;

let nodes = parse_file(path, content, Language::Rust)?;
let result = parse_file_with_edges(path, content, Language::Python)?;
// result.nodes + result.edges (CONTAINS relationships)
```

## License

MIT
