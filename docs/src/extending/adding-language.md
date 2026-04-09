# Adding a Language

To add a new language parser to deagle:

## 1. Add the tree-sitter grammar

In `crates/deagle-parse/Cargo.toml`:

```toml
[dependencies]
tree-sitter-python = "0.23"
```

## 2. Create the parser module

Create `crates/deagle-parse/src/python_parser.rs`:

```rust
use deagle_core::{DeagleError, Language, Node, NodeKind, Result};
use std::path::Path;

pub fn parse(path: &Path, content: &str) -> Result<Vec<Node>> {
    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_python::LANGUAGE;
    parser.set_language(&language.into()).map_err(|e| {
        DeagleError::Parse {
            file: path.display().to_string(),
            message: format!("Failed to set language: {}", e),
        }
    })?;

    let tree = parser.parse(content, None).ok_or_else(|| DeagleError::Parse {
        file: path.display().to_string(),
        message: "Failed to parse".into(),
    })?;

    let mut nodes = Vec::new();
    // Extract definitions from tree...
    Ok(nodes)
}
```

## 3. Wire it into the dispatcher

In `crates/deagle-parse/src/lib.rs`:

```rust
pub mod python_parser;

pub fn parse_file(path: &Path, content: &str, language: Language) -> Result<Vec<Node>> {
    match language {
        Language::Rust => rust_parser::parse(path, content),
        Language::Python => python_parser::parse(path, content),
        _ => Ok(Vec::new()),
    }
}
```

## 4. Add tests

Write tests with representative source code for the language, verifying that all expected entity types are extracted.
