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
pub mod java_parser;
pub mod c_parser;

#[cfg(feature = "pattern")]
pub mod pattern;

#[cfg(feature = "text-search")]
pub mod text_search;

use deagle_core::{Language, Node, Result};
use std::path::Path;

pub use rust_parser::ParseResult;

/// Truncate a string to at most `max_bytes`, respecting UTF-8 char boundaries.
pub(crate) fn truncate_content(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // Find the last char boundary at or before max_bytes
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

/// Parse a source file and extract code entities.
pub fn parse_file(path: &Path, content: &str, language: Language) -> Result<Vec<Node>> {
    match language {
        Language::Rust => rust_parser::parse(path, content),
        Language::Python => python_parser::parse(path, content),
        Language::Go => go_parser::parse(path, content),
        Language::TypeScript | Language::JavaScript => typescript_parser::parse(path, content),
        Language::Java => java_parser::parse(path, content),
        Language::C => c_parser::parse(path, content),
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
        Language::Java => java_parser::parse_with_edges(path, content),
        Language::C => c_parser::parse_with_edges(path, content),
        _ => Ok(ParseResult { nodes: Vec::new(), edges: Vec::new() }),
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_content;

    #[test]
    fn test_truncate_ascii_short() {
        assert_eq!(truncate_content("hello", 500), "hello");
    }

    #[test]
    fn test_truncate_ascii_exact() {
        let s = "a".repeat(500);
        assert_eq!(truncate_content(&s, 500), s);
    }

    #[test]
    fn test_truncate_ascii_long() {
        let s = "a".repeat(600);
        let result = truncate_content(&s, 500);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 503); // 500 + "..."
    }

    #[test]
    fn test_truncate_multibyte_at_boundary() {
        // "→" is 3 bytes (E2 86 92). Place it so byte 500 falls inside it.
        let mut s = "x".repeat(499); // 499 ASCII bytes
        s.push('→'); // bytes 499..502
        s.push_str("after");
        // Truncating at 500 would split "→". Should back up to 499.
        let result = truncate_content(&s, 500);
        assert!(result.ends_with("..."));
        assert!(!result.contains('→'), "should not include partial char");
        assert_eq!(&result[..499], &"x".repeat(499));
    }

    #[test]
    fn test_truncate_emoji_boundary() {
        // "🦀" is 4 bytes. Place it at the cut point.
        let mut s = "a".repeat(498);
        s.push('🦀'); // bytes 498..502
        s.push_str("tail");
        let result = truncate_content(&s, 500);
        assert!(result.ends_with("..."));
        // Should back up to byte 498
        assert_eq!(&result[..498], &"a".repeat(498));
    }

    #[test]
    fn test_truncate_all_multibyte() {
        // All 2-byte chars: "é" = 2 bytes
        let s: String = std::iter::repeat('é').take(300).collect(); // 600 bytes
        let result = truncate_content(&s, 500);
        assert!(result.ends_with("..."));
        // 500 bytes / 2 bytes per char = 250 chars, perfectly aligned
        assert_eq!(result.chars().filter(|c| *c == 'é').count(), 250);
    }

    #[test]
    fn test_truncate_empty() {
        assert_eq!(truncate_content("", 500), "");
    }

    #[test]
    fn test_truncate_zero_max() {
        assert_eq!(truncate_content("hello", 0), "...");
    }
}

/// Detect language from file path and parse.
pub fn parse_auto(path: &Path, content: &str) -> Result<Vec<Node>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = Language::from_extension(ext);
    parse_file(path, content, lang)
}
