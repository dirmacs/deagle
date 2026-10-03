//! Fast regex text search via ripgrep library crates.
//!
//! Searches files for regex patterns with ripgrep-grade performance.
//! Complements structural search (ast-grep) and semantic search (ares-vector).
//!
//! Requires the `text-search` feature flag.

use deagle_core::{Language, Result};
use grep_matcher::Matcher;
use grep_regex::RegexMatcher;
use grep_searcher::{Searcher, Sink, SinkMatch};
use serde::Serialize;
use std::io;
use std::path::Path;

/// A text search match with location info.
///
/// Offsets use two different units on purpose; do not conflate them.
///
/// * [`byte_offset`](TextMatch::byte_offset) counts **bytes** from the start
///   of the file, so `&content[byte_offset..]` starts at the match. Source
///   files are not guaranteed to be ASCII, so a byte offset is not a column.
/// * [`column`](TextMatch::column) counts **characters** (Unicode scalar
///   values), 1-indexed like [`line_number`](TextMatch::line_number), so it
///   matches what an editor shows and what `rg --column` reports. A match at
///   the start of a line is column 1.
///
/// One `TextMatch` is produced per matching *line*, as before. If a line holds
/// several matches, both fields point at the first one on that line.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct TextMatch {
    /// File path
    pub file_path: String,
    /// Line number (1-indexed)
    pub line_number: u64,
    /// Column of the match within its line, 1-indexed and counted in
    /// characters (not bytes). See the type docs.
    pub column: u64,
    /// Byte offset of the match from the start of the file, 0-indexed.
    /// See the type docs.
    pub byte_offset: u64,
    /// Matched line content (trimmed)
    pub line: String,
}

/// Search a single file for a regex pattern.
///
/// Offsets are relative to the start of `content`, so pass the whole file for
/// a `TextMatch::byte_offset` that indexes the file.
pub fn search_file(path: &Path, content: &[u8], pattern: &str) -> Result<Vec<TextMatch>> {
    let matcher = RegexMatcher::new(pattern)
        .map_err(|e| deagle_core::DeagleError::Other(format!("Invalid regex: {}", e)))?;

    let file_path = path.to_string_lossy().to_string();

    let mut searcher = Searcher::new();
    let mut sink = TextMatchSink::new(&matcher, file_path);
    searcher
        .search_slice(&matcher, content, &mut sink)
        .map_err(|e| deagle_core::DeagleError::Other(format!("Search error: {}", e)))?;

    Ok(sink.into_matches())
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
    let entries = std::fs::read_dir(dir).map_err(deagle_core::DeagleError::Io)?;

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.')
                || name == "target"
                || name == "node_modules"
                || name == "vendor"
            {
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
        let mut sink = TextMatchSink::new(matcher, file_str);
        let _ = searcher.search_slice(matcher, &content, &mut sink);
        results.extend(sink.into_matches());
    }
    Ok(())
}

/// Collects [`TextMatch`]es, including where inside the file each one landed.
///
/// The `sinks::UTF8` closure only receives `(line_number, line)`, so it cannot
/// report a position within the line. `Sink::matched` receives a `SinkMatch`,
/// which carries the absolute byte offset of the line the match starts on;
/// the matcher then locates the pattern inside that line.
struct TextMatchSink<'m> {
    matcher: &'m RegexMatcher,
    file_path: String,
    matches: Vec<TextMatch>,
}

impl<'m> TextMatchSink<'m> {
    fn new(matcher: &'m RegexMatcher, file_path: String) -> Self {
        Self {
            matcher,
            file_path,
            matches: Vec::new(),
        }
    }

    fn into_matches(self) -> Vec<TextMatch> {
        self.matches
    }
}

impl Sink for TextMatchSink<'_> {
    type Error = io::Error;

    fn matched(
        &mut self,
        _searcher: &Searcher,
        mat: &SinkMatch<'_>,
    ) -> std::result::Result<bool, io::Error> {
        let line_number = match mat.line_number() {
            Some(line_number) => line_number,
            None => return Err(io::Error::other("line numbers not enabled")),
        };

        // `mat.bytes()` is the matching line including its terminator.
        let raw = mat.bytes();
        let line =
            std::str::from_utf8(raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // Search the line without its terminator so `$` and `\b` see the same
        // text the file does; fall back to the raw line for a pattern that
        // only matches the terminator itself.
        let body = trim_terminator(raw);
        let byte_in_line = self
            .matcher
            .find_at(body, 0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?
            .or(self
                .matcher
                .find_at(raw, 0)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?)
            .map(|m| m.start())
            .unwrap_or(0);

        self.matches.push(TextMatch {
            file_path: self.file_path.clone(),
            line_number,
            column: char_column(line, byte_in_line),
            byte_offset: mat.absolute_byte_offset() + byte_in_line as u64,
            line: line.trim_end().to_string(),
        });

        Ok(true)
    }
}

/// Drop a trailing `\n` or `\r\n` from a line's bytes.
fn trim_terminator(line: &[u8]) -> &[u8] {
    let body = line.strip_suffix(b"\n").unwrap_or(line);
    body.strip_suffix(b"\r").unwrap_or(body)
}

/// 1-indexed character column of `byte_offset` within `line`.
///
/// `byte_offset` is a byte index, so it is floored to a char boundary first:
/// a regex match always starts on one, but flooring keeps this total.
fn char_column(line: &str, byte_index: usize) -> u64 {
    let mut at = byte_index.min(line.len());
    while at > 0 && !line.is_char_boundary(at) {
        at -= 1;
    }
    line[..at].chars().count() as u64 + 1
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

    /// `byte_offset` must index the searched bytes, not the character count.
    #[test]
    fn test_byte_offset_locates_match_in_ascii_file() {
        let content = b"fn hello() {\n    // TODO: fix\n}\n";
        let matches = search_file(&PathBuf::from("test.rs"), content, "TODO").unwrap();
        assert_eq!(matches.len(), 1);
        let m = &matches[0];

        // "fn hello() {\n" is 13 bytes; "TODO" starts 7 bytes into line 2.
        assert_eq!(m.byte_offset, 20);
        assert!(
            content[m.byte_offset as usize..].starts_with(b"TODO"),
            "byte_offset {} does not point at the match in {:?}",
            m.byte_offset,
            std::str::from_utf8(content).unwrap()
        );
    }

    /// The bytes-vs-characters distinction, with a multi-byte prefix on the
    /// match's line and on the line before it.
    #[test]
    fn test_byte_offset_is_bytes_and_column_is_characters() {
        // Line 1 "// ✅ ok\n"  : 7 chars, 10 bytes.
        // Line 2 "// ééé // TODO\n": 13 bytes precede "TODO", but only 10
        // characters do.
        let content = "// ✅ ok\n// ééé // TODO\n".as_bytes();
        let matches = search_file(&PathBuf::from("test.rs"), content, "TODO").unwrap();
        assert_eq!(matches.len(), 1);
        let m = &matches[0];

        assert_eq!(m.line_number, 2);
        assert_eq!(
            m.byte_offset, 23,
            "byte_offset must be 10 (line 1) + 13 (bytes into line 2)"
        );
        assert!(
            content[m.byte_offset as usize..].starts_with(b"TODO"),
            "byte_offset must index the source bytes at the match"
        );

        // 1-indexed character column, not bytes and not 0-indexed.
        assert_eq!(m.column, 11, "column must be a 1-indexed char column");
        assert_ne!(m.column, 14, "column must not be a 1-indexed byte column");
        assert_ne!(m.column, 13, "column must not be a 0-indexed byte column");
        assert_ne!(m.column, 10, "column must not be a 0-indexed char column");
    }

    /// A match at the start of a line is column 1, matching `line_number`'s
    /// 1-indexing.
    #[test]
    fn test_column_is_one_indexed() {
        let content = b"TODO first\nlet x = 1;\nTODO second\n";
        let matches = search_file(&PathBuf::from("test.rs"), content, "TODO").unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!((matches[0].line_number, matches[0].column), (1, 1));
        assert_eq!(matches[0].byte_offset, 0);
        // "TODO first\n" + "let x = 1;\n" is 22 bytes.
        assert_eq!((matches[1].line_number, matches[1].column), (3, 1));
        assert_eq!(matches[1].byte_offset, 22);
    }

    /// Offsets accumulate across lines, and a CRLF terminator counts as two
    /// bytes without shifting the column.
    #[test]
    fn test_crlf_offsets() {
        let content = b"let a = 1;\r\n// TODO: x\r\n";
        let matches = search_file(&PathBuf::from("test.rs"), content, "TODO").unwrap();
        assert_eq!(matches.len(), 1);
        let m = &matches[0];
        assert_eq!(m.line_number, 2);
        assert_eq!(m.column, 4);
        assert_eq!(m.byte_offset, 15, "\"let a = 1;\\r\\n\" is 12 bytes");
        assert!(content[m.byte_offset as usize..].starts_with(b"TODO"));
        assert_eq!(m.line, "// TODO: x", "terminator is trimmed from the line");
    }

    /// Directory search reports file-relative offsets, not offsets into some
    /// accumulated buffer.
    #[test]
    fn test_search_directory_offsets_are_per_file() {
        let dir = std::env::temp_dir().join(format!("deagle-rg-offsets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "// ✅ a\nfn a() {}\n").unwrap();
        std::fs::write(dir.join("b.rs"), "// ✅ b\n// ééé // TODO\n").unwrap();

        let matches = search_directory(&dir, "TODO", None).unwrap();
        let on_disk = std::fs::read(dir.join("b.rs")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(matches.len(), 1, "only b.rs contains TODO");

        let m = &matches[0];
        assert!(
            on_disk[m.byte_offset as usize..].starts_with(b"TODO"),
            "offset must index this file's own bytes: {:?}",
            std::str::from_utf8(&on_disk).unwrap()
        );
        // "// ✅ b\n" is 9 bytes; "TODO" starts 13 bytes into line 2.
        assert_eq!(m.byte_offset, 22);
        assert_eq!(m.column, 11);
    }

    #[test]
    fn test_text_match_serializes_offsets() {
        let content = "// é // TODO\n".as_bytes();
        let matches = search_file(&PathBuf::from("test.rs"), content, "TODO").unwrap();
        let json = serde_json::to_value(&matches[0]).unwrap();
        assert_eq!(json["line_number"], 1);
        assert_eq!(json["column"], 9);
        assert_eq!(json["byte_offset"], 9);
    }
}
