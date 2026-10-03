//! Structural pattern matching via ast-grep-core.
//!
//! Search code using AST patterns like `fn $NAME($$$ARGS) { $$$ }` to find
//! all function definitions, or `$X.unwrap()` to find all unwrap calls.
//!
//! Requires the `pattern` feature flag.

use crate::char_column;
use deagle_core::{Language, Result};
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
    /// Start column of the match within `line_start`, **1-indexed and counted
    /// in characters** — the same unit as `text_search::TextMatch::column` and
    /// computed by the same helper, so an `sg` column and an `rg` column mean
    /// the same thing.
    ///
    /// This field was hardcoded to `0` for every match. It was documented as
    /// 0-indexed; it is now 1-indexed to match the rest of the crate, which is
    /// safe because it never carried a usable value to begin with.
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

fn search_rust(path: &Path, content: &str, pattern: &str) -> Result<Vec<PatternMatch>> {
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

    let line_index_of = |byte: usize| -> usize { line_starts.partition_point(|&s| s <= byte) };

    let byte_to_line = |byte: usize| -> u32 { line_index_of(byte) as u32 };

    // 1-indexed character column within the line the match starts on.
    let byte_to_col = |byte: usize| -> u32 {
        let line_start = line_starts[line_index_of(byte) - 1];
        char_column(&content[line_start..], byte - line_start) as u32
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
                col_start: byte_to_col(range.start),
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
        assert!(!matches.is_empty(), "should find zero-arg functions");
    }

    #[test]
    fn test_find_struct_definitions() {
        let path = PathBuf::from("test.rs");
        let matches =
            search_pattern(&path, SAMPLE, "struct $NAME { $$$ }", Language::Rust).unwrap();
        assert!(!matches.is_empty(), "should find struct definitions");
        assert!(matches[0].text.contains("Config"));
    }

    #[test]
    fn test_no_matches() {
        let path = PathBuf::from("test.rs");
        let matches =
            search_pattern(&path, SAMPLE, "async fn $NAME() { $$$ }", Language::Rust).unwrap();
        assert!(matches.is_empty(), "should find no async functions");
    }

    #[test]
    fn test_unsupported_language_returns_empty() {
        let path = PathBuf::from("test.py");
        let matches = search_pattern(
            &path,
            "def hello(): pass",
            "def $NAME(): $$$",
            Language::Python,
        )
        .unwrap();
        assert!(
            matches.is_empty(),
            "unsupported language returns empty for now"
        );
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

    /// The match node is the whole `Some(1).unwrap()` expression, so the column
    /// points at `Some(1)`.
    #[test]
    fn test_col_start_is_one_indexed() {
        let src = "fn main() {\n    let v = Some(1).unwrap();\n}\n";
        let all =
            search_pattern(&PathBuf::from("t.rs"), src, "$X.unwrap()", Language::Rust).unwrap();
        assert_eq!(all[0].line_start, 2);
        assert_eq!(all[0].col_start, 13, "column is 1-indexed within the line");
    }

    #[test]
    fn test_col_start_is_not_hardcoded_zero() {
        let src = "fn main() {\n    let v = Some(1).unwrap();\n}\n";
        let all =
            search_pattern(&PathBuf::from("t.rs"), src, "$X.unwrap()", Language::Rust).unwrap();
        assert_ne!(
            all[0].col_start, 0,
            "col_start must be measured, not a literal"
        );
    }

    /// A match starting a line is column 1, matching `line_start`'s indexing.
    #[test]
    fn test_col_start_at_line_start_is_one() {
        let src = "Some(1).unwrap();\n";
        let all =
            search_pattern(&PathBuf::from("t.rs"), src, "$X.unwrap()", Language::Rust).unwrap();
        assert_eq!(all[0].col_start, 1);
    }

    /// Multi-byte prefix on the match's own line. Both units are derived from
    /// the same anchor — the remainder of the line starting at the reported
    /// column — so the comparison cannot be skewed by hand-counting, and the
    /// fixture's own assertion proves the two units actually differ here.
    #[test]
    fn test_col_start_counts_characters_not_bytes() {
        let src = "fn main() {\n    let s = \"café ✅\"; let v = Some(1).unwrap();\n}\n";
        let all =
            search_pattern(&PathBuf::from("t.rs"), src, "$X.unwrap()", Language::Rust).unwrap();
        let m = &all[0];
        let line = src.lines().nth(m.line_start as usize - 1).unwrap();
        let tail: String = line.chars().skip(m.col_start as usize - 1).collect();
        assert!(tail.starts_with(&m.text), "column must point at the match");

        let byte_col = line.len() - tail.len() + 1;
        let char_col = line.chars().count() - tail.chars().count() + 1;
        assert_ne!(byte_col, char_col, "fixture must separate the two units");
        assert_eq!(
            m.col_start as usize, char_col,
            "column is the character column"
        );
        assert_ne!(m.col_start as usize, byte_col, "must not be a byte column");
    }

    /// The assertion that fails before the fix: with `col_start` hardcoded to 0
    /// the column points at the start of the line, not at the match.
    #[test]
    fn test_col_start_locates_the_match() {
        let src = "fn a() {}\n// ééé comment\n    let v = Some(1).unwrap();\n";
        let all =
            search_pattern(&PathBuf::from("t.rs"), src, "$X.unwrap()", Language::Rust).unwrap();
        let m = &all[0];
        assert_eq!(m.line_start, 3);
        let line = src.lines().nth(m.line_start as usize - 1).unwrap();
        let from_column: String = line.chars().skip(m.col_start as usize - 1).collect();
        assert!(
            from_column.starts_with(&m.text),
            "column {} must point at {:?}, line tail is {:?}",
            m.col_start,
            m.text,
            from_column
        );
        assert!(
            !line.starts_with(&m.text),
            "fixture check: the match must not begin the line"
        );
    }
}
