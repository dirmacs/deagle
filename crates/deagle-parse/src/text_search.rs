//! Fast regex text search via ripgrep library crates.
//!
//! Searches files for regex patterns with ripgrep-grade performance.
//! Complements structural search (ast-grep) and semantic search (ares-vector).
//!
//! Requires the `text-search` feature flag.

use deagle_core::{Language, Result};
use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::Searcher;
use std::path::Path;

/// A text search match with location info.
#[derive(Debug, Clone)]
pub struct TextMatch {
    /// File path
    pub file_path: String,
    /// Line number (1-indexed)
    pub line_number: u64,
    /// Matched line content (trimmed)
    pub line: String,
}

/// Search a single file for a regex pattern.
pub fn search_file(path: &Path, content: &[u8], pattern: &str) -> Result<Vec<TextMatch>> {
    let matcher = RegexMatcher::new(pattern)
        .map_err(|e| deagle_core::DeagleError::Other(format!("Invalid regex: {}", e)))?;

    let file_path = path.to_string_lossy().to_string();
    let mut matches = Vec::new();

    let mut searcher = Searcher::new();
    searcher
        .search_slice(
            &matcher,
            content,
            UTF8(|line_num, line| {
                matches.push(TextMatch {
                    file_path: file_path.clone(),
                    line_number: line_num,
                    line: line.trim_end().to_string(),
                });
                Ok(true)
            }),
        )
        .map_err(|e| deagle_core::DeagleError::Other(format!("Search error: {}", e)))?;

    Ok(matches)
}

/// Search a directory recursively for a regex pattern.
pub fn search_directory(
    root: &Path,
    pattern: &str,
    language_filter: Option<Language>,
) -> Result<Vec<TextMatch>> {
    let matcher = RegexMatcher::new(pattern)
        .map_err(|e| deagle_core::DeagleError::Other(format!("Invalid regex: {}", e)))?;

    let mut all_matches = Vec::new();
    walk_search(root, root, &matcher, language_filter, &mut all_matches)?;
    Ok(all_matches)
}

fn walk_search(
    root: &Path,
    dir: &Path,
    matcher: &RegexMatcher,
    lang_filter: Option<Language>,
    results: &mut Vec<TextMatch>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir)
        .map_err(deagle_core::DeagleError::Io)?;

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "target" || name == "node_modules" || name == "vendor" {
                continue;
            }
            walk_search(root, &path, matcher, lang_filter, results)?;
            continue;
        }

        // Language filter
        if let Some(filter) = lang_filter {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if Language::from_extension(ext) != filter {
                continue;
            }
        }

        let content = match std::fs::read(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let rel_path = path.strip_prefix(root).unwrap_or(&path);
        let file_str = rel_path.to_string_lossy().to_string();

        let mut searcher = Searcher::new();
        let _ = searcher.search_slice(
            matcher,
            &content,
            UTF8(|line_num, line| {
                results.push(TextMatch {
                    file_path: file_str.clone(),
                    line_number: line_num,
                    line: line.trim_end().to_string(),
                });
                Ok(true)
            }),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_search_file_finds_pattern() {
        let content = b"fn hello() {\n    println!(\"world\");\n}\n// TODO: fix this\n";
        let path = PathBuf::from("test.rs");
        let matches = search_file(&path, content, "TODO").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line_number, 4);
        assert!(matches[0].line.contains("TODO"));
    }

    #[test]
    fn test_search_file_regex() {
        let content = b"let x = 42;\nlet y = 100;\nlet z = 7;\n";
        let path = PathBuf::from("test.rs");
        let matches = search_file(&path, content, r"let \w+ = \d{3}").unwrap();
        assert_eq!(matches.len(), 1); // only y = 100 has 3 digits
        assert!(matches[0].line.contains("100"));
    }

    #[test]
    fn test_search_file_no_matches() {
        let content = b"fn main() {}\n";
        let path = PathBuf::from("test.rs");
        let matches = search_file(&path, content, "FIXME").unwrap();
        assert!(matches.is_empty());
    }

    #[test]
    fn test_search_file_multiple_matches() {
        let content = b"// TODO: first\nfn work() {}\n// TODO: second\n// TODO: third\n";
        let path = PathBuf::from("test.rs");
        let matches = search_file(&path, content, "TODO").unwrap();
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn test_invalid_regex_returns_error() {
        let content = b"test\n";
        let path = PathBuf::from("test.rs");
        let result = search_file(&path, content, "[invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_case_insensitive_pattern() {
        let content = b"let Result = Ok(42);\ntype result = i32;\n";
        let path = PathBuf::from("test.rs");
        let matches = search_file(&path, content, "(?i)result").unwrap();
        assert_eq!(matches.len(), 2);
    }
}
