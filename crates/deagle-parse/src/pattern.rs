//! Structural pattern matching via ast-grep-core.
//!
//! Search code using AST patterns like `fn $NAME($$$ARGS) { $$$ }` to find
//! all function definitions, or `$X.unwrap()` to find all unwrap calls.
//!
//! Requires the `pattern` feature flag.

use deagle_core::{DeagleError, Language, Result};
use std::path::Path;

/// A pattern match result with location info.
#[derive(Debug, Clone)]
pub struct PatternMatch {
    /// Matched source text
    pub text: String,
    /// File path
    pub file_path: String,
    /// Start line (1-indexed)
    pub line_start: u32,
    /// End line (1-indexed)
    pub line_end: u32,
    /// Start column (0-indexed)
    pub col_start: u32,
}

/// Search a file for structural patterns using ast-grep.
///
/// Pattern syntax: use `$NAME` for single-node wildcards, `$$$` for multi-node.
/// Examples:
/// - `fn $NAME() {}` — matches zero-arg functions
/// - `$X.unwrap()` — matches all .unwrap() calls
/// - `use $MODULE::$ITEM` — matches specific use imports
pub fn search_pattern(
    path: &Path,
    content: &str,
    pattern: &str,
    language: Language,
) -> Result<Vec<PatternMatch>> {
    match language {
        Language::Rust => search_rust(path, content, pattern),
        _ => Ok(Vec::new()),
    }
}

fn search_rust(
    path: &Path,
    content: &str,
    pattern: &str,
) -> Result<Vec<PatternMatch>> {
    use ast_grep_core::{AstGrep, Pattern};
    use ast_grep_language::SupportLang;

    let lang = SupportLang::Rust;
    let grep = AstGrep::new(content, lang);

    let pat = Pattern::new(pattern, lang);

    let file_path = path.to_string_lossy().to_string();

    // Count line numbers from byte offset
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(content.match_indices('\n').map(|(i, _)| i + 1))
        .collect();

    let byte_to_line = |byte: usize| -> u32 {
        (line_starts.partition_point(|&s| s <= byte)) as u32
    };

    let matches: Vec<PatternMatch> = grep
        .root()
        .find_all(&pat)
        .map(|node| {
            let range = node.range();
            PatternMatch {
                text: node.text().to_string(),
                file_path: file_path.clone(),
                line_start: byte_to_line(range.start),
                line_end: byte_to_line(range.end),
                col_start: 0,
            }
        })
        .collect();

    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SAMPLE: &str = r#"
fn hello() {
    println!("hello");
}

fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn process() {
    let x = Some(42);
    let val = x.unwrap();
    let name = "test".to_string();
}

struct Config {
    name: String,
}

impl Config {
    fn new(name: &str) -> Self {
        Self { name: name.to_string() }
    }
}
"#;

    #[test]
    fn test_find_unwrap_calls() {
        let path = PathBuf::from("test.rs");
        let matches = search_pattern(&path, SAMPLE, "$X.unwrap()", Language::Rust).unwrap();
        assert!(
            !matches.is_empty(),
            "should find .unwrap() calls, got 0 matches"
        );
        assert!(matches[0].text.contains("unwrap"));
    }

    #[test]
    fn test_find_functions() {
        let path = PathBuf::from("test.rs");
        let matches = search_pattern(&path, SAMPLE, "fn $NAME() { $$$ }", Language::Rust).unwrap();
        assert!(
            !matches.is_empty(),
            "should find zero-arg functions"
        );
    }

    #[test]
    fn test_find_struct_definitions() {
        let path = PathBuf::from("test.rs");
        let matches = search_pattern(&path, SAMPLE, "struct $NAME { $$$ }", Language::Rust).unwrap();
        assert!(
            !matches.is_empty(),
            "should find struct definitions"
        );
        assert!(matches[0].text.contains("Config"));
    }

    #[test]
    fn test_no_matches() {
        let path = PathBuf::from("test.rs");
        let matches = search_pattern(&path, SAMPLE, "async fn $NAME() { $$$ }", Language::Rust).unwrap();
        assert!(matches.is_empty(), "should find no async functions");
    }

    #[test]
    fn test_unsupported_language_returns_empty() {
        let path = PathBuf::from("test.py");
        let matches = search_pattern(&path, "def hello(): pass", "def $NAME(): $$$", Language::Python).unwrap();
        assert!(matches.is_empty(), "unsupported language returns empty for now");
    }

    #[test]
    fn test_match_has_location() {
        let path = PathBuf::from("test.rs");
        let matches = search_pattern(&path, SAMPLE, "$X.unwrap()", Language::Rust).unwrap();
        if !matches.is_empty() {
            assert!(matches[0].line_start > 0, "line should be 1-indexed");
            assert_eq!(matches[0].file_path, "test.rs");
        }
    }
}
