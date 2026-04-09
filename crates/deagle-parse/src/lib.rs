//! deagle-parse — tree-sitter based code parser.
//!
//! Extracts code entities (functions, structs, traits, impls, imports)
//! from source files using tree-sitter grammars.
//!
//! ## Feature Flags
//!
//! - `pattern` — structural pattern matching via [ast-grep-core](https://crates.io/crates/ast-grep-core)

pub mod rust_parser;

#[cfg(feature = "pattern")]
pub mod pattern;

use deagle_core::{Language, Node, Result};
use std::path::Path;

/// Parse a source file and extract code entities.
pub fn parse_file(path: &Path, content: &str, language: Language) -> Result<Vec<Node>> {
    match language {
        Language::Rust => rust_parser::parse(path, content),
        _ => Ok(Vec::new()), // Other languages TODO
    }
}

/// Detect language from file path and parse.
pub fn parse_auto(path: &Path, content: &str) -> Result<Vec<Node>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = Language::from_extension(ext);
    parse_file(path, content, lang)
}
