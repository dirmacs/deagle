//! deagle-parse — tree-sitter based code parser.
//!
//! Extracts code entities (functions, structs, traits, impls, imports)
//! from source files using tree-sitter grammars.
//!
//! ## Feature Flags
//!
//! - `pattern` — structural pattern matching via [ast-grep-core](https://crates.io/crates/ast-grep-core)

pub mod rust_parser;
pub mod python_parser;
pub mod go_parser;
pub mod typescript_parser;

#[cfg(feature = "pattern")]
pub mod pattern;

#[cfg(feature = "text-search")]
pub mod text_search;

use deagle_core::{Language, Node, Result};
use std::path::Path;

pub use rust_parser::ParseResult;

/// Parse a source file and extract code entities.
pub fn parse_file(path: &Path, content: &str, language: Language) -> Result<Vec<Node>> {
    match language {
        Language::Rust => rust_parser::parse(path, content),
        Language::Python => python_parser::parse(path, content),
        Language::Go => go_parser::parse(path, content),
        Language::TypeScript | Language::JavaScript => typescript_parser::parse(path, content),
        _ => Ok(Vec::new()),
    }
}

/// Parse with edge extraction — returns nodes and relationship tuples.
pub fn parse_file_with_edges(path: &Path, content: &str, language: Language) -> Result<ParseResult> {
    match language {
        Language::Rust => rust_parser::parse_with_edges(path, content),
        Language::Python => python_parser::parse_with_edges(path, content),
        Language::Go => go_parser::parse_with_edges(path, content),
        Language::TypeScript | Language::JavaScript => typescript_parser::parse_with_edges(path, content),
        _ => Ok(ParseResult { nodes: Vec::new(), edges: Vec::new() }),
    }
}

/// Detect language from file path and parse.
pub fn parse_auto(path: &Path, content: &str) -> Result<Vec<Node>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = Language::from_extension(ext);
    parse_file(path, content, lang)
}
