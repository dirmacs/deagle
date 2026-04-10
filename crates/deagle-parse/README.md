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

- `pattern` — structural AST search via ast-grep (`search_pattern`)
- `text-search` — regex text search via ripgrep library crates

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
