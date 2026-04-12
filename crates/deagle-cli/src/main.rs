//! deagle CLI — Rust-native code intelligence.
//!
//! Commands:
//! - `deagle map <DIR>` — index a codebase into the graph
//! - `deagle search <QUERY>` — search for symbols
//! - `deagle stats` — show graph statistics

use clap::{Parser, Subcommand};
use deagle_core::{Edge, EdgeKind, GraphDb, Language};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "deagle")]
#[command(about = "Rust-native code intelligence — map, search, explain")]
#[command(version)]
struct Cli {
    /// Path to the graph database
    #[arg(long, default_value = ".deagle/graph.db", global = true)]
    db: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Index a codebase into the graph database
    Map {
        /// Directory to index
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Force full re-index (skip incremental hash check)
        #[arg(long)]
        force: bool,
    },
    /// Search for symbols by name
    Search {
        /// Search query (substring match, or fuzzy with --fuzzy)
        query: String,
        /// Filter by entity kind
        #[arg(long)]
        kind: Option<String>,
        /// Use fuzzy matching (ranked by score) instead of substring
        #[arg(long)]
        fuzzy: bool,
        /// Filter by language (e.g., "rust", "python", "go", "typescript")
        #[arg(long, short = 'l')]
        lang: Option<String>,
        /// Directory/file paths to scope the search (default: use graph DB).
        /// When paths are provided and no graph.db exists, falls back to
        /// ripgrep-style text search scoped to those paths.
        #[arg(num_args = 0..)]
        paths: Vec<PathBuf>,
    },
    /// Full-text keyword search (BM25 ranked via FTS5)
    Keyword {
        /// Search query (searches entity names and content)
        query: String,
    },
    /// Show graph statistics
    Stats {
        /// Ignored positional arg — kept for friendlier UX when users type
        /// `deagle stats <path>` expecting per-file stats. Prints a hint
        /// pointing at `deagle keyword` instead of erroring.
        #[arg(hide = true)]
        hint_path: Option<PathBuf>,
    },
    /// Structural AST pattern search (powered by ast-grep)
    #[cfg(feature = "pattern")]
    Sg {
        /// AST pattern (e.g., "$X.unwrap()", "fn $NAME() { $$$ }")
        pattern: String,
        /// Directory(ies) to search (default: current directory)
        #[arg(num_args = 0.., default_value = ".")]
        paths: Vec<PathBuf>,
    },
    /// Count lines of code by language (powered by tokei)
    Loc {
        /// Directory to count
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Fast regex text search (powered by ripgrep)
    #[cfg(feature = "text-search")]
    Rg {
        /// Regex pattern
        pattern: String,
        /// Directory(ies) to search (default: current directory)
        #[arg(num_args = 0.., default_value = ".")]
        paths: Vec<PathBuf>,
        /// Filter by language (e.g., "rust", "python")
        #[arg(long)]
        lang: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Map { dir, force } => cmd_map(&cli.db, &dir, force),
        Commands::Search { query, kind, fuzzy, lang, paths } => {
            cmd_search(&cli.db, &query, kind.as_deref(), fuzzy, lang.as_deref(), &paths)
        }
        Commands::Keyword { query } => cmd_keyword(&cli.db, &query),
        Commands::Stats { hint_path } => cmd_stats(&cli.db, hint_path.as_deref()),
        Commands::Loc { dir } => cmd_loc(&dir),
        #[cfg(feature = "pattern")]
        Commands::Sg { pattern, paths } => {
            paths.iter().try_for_each(|path| cmd_grep(&pattern, path))
        },
        #[cfg(feature = "text-search")]
        Commands::Rg { pattern, paths, lang } => {
            paths.iter().try_for_each(|path| cmd_rg(&pattern, path, lang.as_deref()))
        },
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn cmd_map(db_path: &Path, dir: &Path, force: bool) -> Result<(), String> {
    use rayon::prelude::*;

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create db dir: {}", e))?;
    }

    let db = GraphDb::open(db_path).map_err(|e| format!("Failed to open db: {}", e))?;

    if force {
        db.clear().map_err(|e| format!("Failed to clear db: {}", e))?;
        eprintln!("Full re-index of {}...", dir.display());
    } else {
        eprintln!("Incremental index of {}...", dir.display());
    }

    // Collect file paths first (ignore-aware)
    let files: Vec<_> = ignore::WalkBuilder::new(dir)
        .hidden(true).git_ignore(true).git_global(true).git_exclude(true)
        .build()
        .flatten()
        .filter(|e| e.path().is_file())
        .filter(|e| {
            let ext = e.path().extension().and_then(|x| x.to_str()).unwrap_or("");
            Language::from_extension(ext) != Language::Unknown
        })
        .collect();

    // Pre-filter: check hashes sequentially (SQLite not thread-safe), then parse in parallel
    let files_to_parse: Vec<_> = files.iter().filter(|entry| {
        if force { return true; }
        let path = entry.path();
        let rel_path = path.strip_prefix(dir).unwrap_or(path);
        let rel_str = rel_path.to_string_lossy();
        let content = match std::fs::read_to_string(path) {
            Ok(c) if !c.is_empty() => c,
            _ => return false,
        };
        db.needs_reindex(&rel_str, &content).unwrap_or(true)
    }).collect();

    // Parse changed files in parallel with rayon
    let results: Vec<_> = files_to_parse.par_iter().filter_map(|entry| {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = Language::from_extension(ext);
        let content = std::fs::read_to_string(path).ok()?;
        if content.is_empty() { return None; }
        let rel_path = path.strip_prefix(dir).unwrap_or(path);
        let rel_str = rel_path.to_string_lossy().to_string();

        deagle_parse::parse_file_with_edges(rel_path, &content, lang)
            .ok()
            .map(|r| (rel_str, content, r))
    }).collect();

    // Batch insert into DB (single transaction per file for speed)
    let mut file_count = 0;
    let mut node_count = 0;
    let mut edge_count = 0;

    for (rel_path, content, result) in &results {
        if result.nodes.is_empty() { continue; }

        // Incremental: remove old data for this file before re-inserting
        if !force {
            let _ = db.remove_file(rel_path);
        }

        file_count += 1;
        node_count += result.nodes.len();

        // Batch insert nodes — returns their DB IDs
        let db_ids = match db.insert_batch(&result.nodes, &[]) {
            Ok(ids) => ids,
            Err(_) => continue,
        };

        // Store file hash for incremental indexing
        let _ = db.store_file_hash(rel_path, content);

        // Collect resolved edges and batch insert
        let resolved_edges: Vec<(i64, i64, EdgeKind)> = result.edges.iter()
            .filter(|(from_idx, to_idx, _)| {
                *from_idx < db_ids.len() && *to_idx < db_ids.len()
                    && db_ids[*from_idx] > 0 && db_ids[*to_idx] > 0
            })
            .map(|(from_idx, to_idx, kind)| (db_ids[*from_idx], db_ids[*to_idx], *kind))
            .collect();
        edge_count += resolved_edges.len();

        if !resolved_edges.is_empty() {
            // Insert edges in their own batch (nodes already committed)
            for (from_id, to_id, kind) in &resolved_edges {
                let _ = db.insert_edge(&Edge {
                    from_id: *from_id, to_id: *to_id, kind: *kind, confidence: 1.0,
                });
            }
        }
    }

    let total_files = files.len();
    let skipped = total_files - file_count;
    if skipped > 0 {
        eprintln!("Indexed {} files ({} unchanged, skipped), {} entities, {} edges", file_count, skipped, node_count, edge_count);
    } else {
        eprintln!("Indexed {} files, {} entities, {} edges", file_count, node_count, edge_count);
    }
    eprintln!("Database: {}", db_path.display());
    Ok(())
}

fn cmd_search(
    db_path: &Path,
    query: &str,
    kind: Option<&str>,
    fuzzy: bool,
    lang: Option<&str>,
    paths: &[PathBuf],
) -> Result<(), String> {
    // If the graph DB doesn't exist, give an actionable message.
    // When paths are provided, fall back to ripgrep text search instead of erroring.
    if !db_path.exists() {
        if !paths.is_empty() {
            // Fallback: paths provided — do ripgrep-style text search
            eprintln!(
                "note: no graph database found at '{}' — falling back to text search.\n\
                 Run `deagle map <DIR>` to build the graph for faster, structured results.",
                db_path.display()
            );
            #[cfg(feature = "text-search")]
            {
                use deagle_parse::text_search::search_directory;
                for path in paths {
                    let lang_filter = lang.map(|l| Language::from_extension(match l {
                        "rust" | "rs" => "rs",
                        "python" | "py" => "py",
                        "go" => "go",
                        "typescript" | "ts" => "ts",
                        "javascript" | "js" => "js",
                        other => other,
                    }));
                    match search_directory(path, query, lang_filter) {
                        Ok(matches) if !matches.is_empty() => {
                            for m in &matches {
                                println!("{}:{}: {}", m.file_path, m.line_number, m.line);
                            }
                            eprintln!("\n{} text match(es) in {}", matches.len(), path.display());
                        }
                        Ok(_) => eprintln!("No matches in {}", path.display()),
                        Err(e) => eprintln!("text search error: {}", e),
                    }
                }
                return Ok(());
            }
            #[cfg(not(feature = "text-search"))]
            return Err(format!(
                "no graph database at '{}'. Run `deagle map <DIR>` first.",
                db_path.display()
            ));
        }
        return Err(format!(
            "no graph database found at '{}'.\n\
             Run `deagle map <DIR>` to build the graph, then retry.\n\
             Tip: `deagle map .` indexes the current directory.",
            db_path.display()
        ));
    }

    let db = GraphDb::open(db_path).map_err(|e| format!("Failed to open db: {}", e))?;
    let results = if fuzzy {
        db.fuzzy_search_nodes(query).map_err(|e| format!("Search failed: {}", e))?
    } else {
        db.search_nodes(query).map_err(|e| format!("Search failed: {}", e))?
    };

    // Apply kind filter
    let results: Vec<_> = if let Some(k) = kind {
        results.into_iter().filter(|n| n.kind.to_string() == k).collect()
    } else {
        results
    };

    // Apply language filter (--lang / -l)
    let results: Vec<_> = if let Some(l) = lang {
        let l_lower = l.to_lowercase();
        results.into_iter().filter(|n| {
            let lang_str = n.language.to_string(); // Display impl returns "rust", "python", etc.
            lang_str == l_lower
                || match l_lower.as_str() {
                    "rust" | "rs" => lang_str == "rust",
                    "python" | "py" => lang_str == "python",
                    "go" => lang_str == "go",
                    "typescript" | "ts" => lang_str == "typescript",
                    "javascript" | "js" => lang_str == "javascript",
                    _ => lang_str.starts_with(&l_lower),
                }
        }).collect()
    } else {
        results
    };

    // Apply path scope filter (positional paths).
    // The graph stores paths relative to the indexed root (e.g. "crates/foo/src/bar.rs").
    // Users may pass absolute paths (/opt/eruka/crates) or relative ones.
    // Match if:
    //   (a) stored path starts with the given path, OR
    //   (b) stored path contains any component of the given path as a substring
    let results: Vec<_> = if !paths.is_empty() {
        results.into_iter().filter(|n| {
            paths.iter().any(|p| {
                let p_str = p.to_string_lossy();
                // Strip trailing slash for comparison
                let p_norm = p_str.trim_end_matches('/');
                // (a) exact prefix match (handles relative paths)
                n.file_path.starts_with(p_norm)
                    // (b) the last N components of p appear anywhere in n.file_path
                    || p.components().last().map(|c| {
                        let last = c.as_os_str().to_string_lossy();
                        n.file_path.contains(last.as_ref())
                    }).unwrap_or(false)
                    // (c) given path is a suffix of stored path
                    || n.file_path.ends_with(p_norm)
            })
        }).collect()
    } else {
        results
    };

    if results.is_empty() {
        eprintln!("No results for '{}'", query);
        return Ok(());
    }

    println!("{:<30} {:<12} {:<10} LOCATION", "NAME", "KIND", "LANG");
    let sep = "-".repeat(80);
    println!("{sep}");
    for node in &results {
        println!(
            "{:<30} {:<12} {:<10} {}:{}",
            node.name, node.kind, node.language, node.file_path, node.line_start,
        );
    }
    println!("\n{} result(s)", results.len());
    Ok(())
}

/// Rewrite a human/agent-friendly keyword query into FTS5-safe syntax.
///
/// - `foo\|bar\|baz`  and `foo|bar|baz`  → `foo OR bar OR baz`
/// - strips characters FTS5 treats as operators (`"` `(` `)` `:` `*` `-`)
/// - collapses whitespace
/// - if only one term remains, returns it bare (no OR wrapping)
fn sanitize_fts5_query(raw: &str) -> String {
    // Treat both escaped-shell `\|` and bare `|` as alternation.
    let with_or = raw.replace("\\|", " OR ").replace('|', " OR ");
    // Drop FTS5-significant chars that commonly sneak in from code symbols.
    let cleaned: String = with_or
        .chars()
        .map(|c| match c {
            '"' | '(' | ')' | ':' | '*' => ' ',
            // Leading `-` means NOT in FTS5 — strip to avoid accidental negation.
            '-' => ' ',
            other => other,
        })
        .collect();
    // Collapse runs of whitespace.
    let collapsed: Vec<&str> = cleaned.split_whitespace().collect();
    collapsed.join(" ")
}

fn cmd_keyword(db_path: &Path, query: &str) -> Result<(), String> {
    let db = GraphDb::open(db_path).map_err(|e| format!("Failed to open db: {}", e))?;
    // FTS5 has its own query syntax — it doesn't understand regex alternation.
    // Agents (and humans) frequently pass `foo\|bar` or `foo|bar` expecting
    // "foo OR bar". Rewrite those into FTS5 OR syntax before the query runs
    // and strip characters FTS5 treats as operators so random input doesn't
    // crash the parser with `fts5: syntax error near "..."`.
    let sanitized = sanitize_fts5_query(query);
    let results = db.keyword_search(&sanitized).map_err(|e| format!("Keyword search failed: {}", e))?;

    if results.is_empty() {
        eprintln!("No keyword matches for '{}'", query);
        return Ok(());
    }

    println!("{:<30} {:<12} {:<10} LOCATION", "NAME", "KIND", "LANG");
    let sep = "-".repeat(80);
    println!("{sep}");
    for node in &results {
        println!(
            "{:<30} {:<12} {:<10} {}:{}",
            node.name, node.kind, node.language, node.file_path, node.line_start,
        );
    }
    println!("\n{} result(s) (BM25 ranked)", results.len());
    Ok(())
}

fn cmd_loc(dir: &Path) -> Result<(), String> {
    use tokei::{Config, Languages};

    let config = Config::default();
    let mut languages = Languages::new();
    languages.get_statistics(&[dir], &[], &config);

    if languages.is_empty() {
        eprintln!("No recognized source files in {}", dir.display());
        return Ok(());
    }

    println!("{:<20} {:>8} {:>8} {:>8} {:>8}", "LANGUAGE", "FILES", "CODE", "COMMENTS", "BLANKS");
    println!("{}", "-".repeat(60));

    let mut total_files = 0usize;
    let mut total_code = 0usize;
    let mut total_comments = 0usize;
    let mut total_blanks = 0usize;

    let mut sorted: Vec<_> = languages.iter().collect();
    sorted.sort_by(|a, b| b.1.code.cmp(&a.1.code));

    for (lang_type, lang) in &sorted {
        if lang.code == 0 && lang.comments == 0 {
            continue;
        }
        let files = lang.reports.len();
        println!(
            "{:<20} {:>8} {:>8} {:>8} {:>8}",
            format!("{}", lang_type), files, lang.code, lang.comments, lang.blanks
        );
        total_files += files;
        total_code += lang.code;
        total_comments += lang.comments;
        total_blanks += lang.blanks;
    }

    println!("{}", "-".repeat(60));
    println!(
        "{:<20} {:>8} {:>8} {:>8} {:>8}",
        "TOTAL", total_files, total_code, total_comments, total_blanks
    );
    Ok(())
}

fn cmd_stats(db_path: &Path, hint_path: Option<&Path>) -> Result<(), String> {
    if let Some(p) = hint_path {
        eprintln!(
            "note: `deagle stats` is global graph info and ignores positional paths.\n\
             for per-file inspection try: deagle keyword \"{}\"",
            p.file_stem().and_then(|s| s.to_str()).unwrap_or("<name>"),
        );
    }
    let db = GraphDb::open(db_path).map_err(|e| format!("Failed to open db: {}", e))?;
    let nodes = db.node_count().map_err(|e| e.to_string())?;
    let edges = db.edge_count().map_err(|e| e.to_string())?;

    println!("Database: {}", db_path.display());
    println!("Nodes:    {}", nodes);
    println!("Edges:    {}", edges);
    Ok(())
}

#[cfg(feature = "pattern")]
fn cmd_grep(pattern: &str, dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Err(format!("Directory not found: {}", dir.display()));
    }

    eprintln!("Searching for pattern: {}", pattern);

    let mut total = 0;
    grep_walk(dir, dir, pattern, &mut total)?;

    if total == 0 {
        // AST pattern returned nothing. Try two fallbacks:
        // 1. If pattern looks like an incomplete declaration (no braces/parens),
        //    suggest the completed form.
        // 2. Fall back to ripgrep text search with the same string.
        let hint = suggest_pattern_completion(pattern);
        if let Some(ref completed) = hint {
            eprintln!("note: ast pattern found 0 matches. Trying completed form: {}", completed);
            let mut total2 = 0;
            grep_walk(dir, dir, completed, &mut total2)?;
            if total2 > 0 {
                eprintln!("\n{} match(es) (with completed pattern '{}')", total2, completed);
                eprintln!("tip: use `deagle sg \"{}\"` next time for direct match.", completed);
                return Ok(());
            }
        }

        // Final fallback: plain text search
        eprintln!(
            "No AST matches found.\n\
             tip: AST patterns need full syntax, e.g. `pub enum Foo {{ $$$ }}`\n\
             Falling back to text search for '{}':", pattern
        );
        #[cfg(feature = "text-search")]
        {
            use deagle_parse::text_search::search_directory;
            if let Ok(matches) = search_directory(dir, pattern, None) {
                if matches.is_empty() {
                    eprintln!("No text matches either.");
                } else {
                    for m in &matches {
                        println!("{}:{}: {}", m.file_path, m.line_number, m.line);
                    }
                    eprintln!("\n{} text match(es)", matches.len());
                }
            }
        }
        #[cfg(not(feature = "text-search"))]
        eprintln!("No matches found. Enable the 'text-search' feature for ripgrep fallback.");
    } else {
        eprintln!("\n{} match(es)", total);
    }
    Ok(())
}

/// Suggest a completed ast-grep pattern when the user writes an incomplete declaration.
/// e.g. `pub enum Foo` → `pub enum Foo { $$$ }`
///      `pub struct Foo` → `pub struct Foo { $$$ }`
///      `fn foo` → `fn foo($$$) { $$$ }`
///      `pub fn foo` → `pub fn foo($$$) { $$$ }`
#[cfg(feature = "pattern")]
fn suggest_pattern_completion(pattern: &str) -> Option<String> {
    let t = pattern.trim();
    // Already has braces/parens — no completion needed
    if t.contains('{') || t.contains('(') { return None; }

    let words: Vec<&str> = t.split_whitespace().collect();
    match words.as_slice() {
        // `pub enum Foo` | `enum Foo`
        [.., "enum", _name] => Some(format!("{} {{ $$$ }}", t)),
        // `pub struct Foo` | `struct Foo`
        [.., "struct", _name] => Some(format!("{} {{ $$$ }}", t)),
        // `pub trait Foo` | `trait Foo`
        [.., "trait", _name] => Some(format!("{} {{ $$$ }}", t)),
        // `pub fn foo` | `fn foo` | `async fn foo`
        [.., "fn", _name] => Some(format!("{}($$$) {{ $$$ }}", t)),
        // `impl Foo` | `impl Trait for Foo`
        ["impl", ..] if !t.contains('{') => Some(format!("{} {{ $$$ }}", t)),
        _ => None,
    }
}

#[cfg(feature = "pattern")]
fn grep_walk(root: &Path, _dir: &Path, pattern: &str, total: &mut usize) -> Result<(), String> {
    use deagle_parse::pattern::search_pattern;

    let walker = ignore::WalkBuilder::new(root)
        .hidden(true).git_ignore(true).build();

    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = Language::from_extension(ext);
        if lang == Language::Unknown { continue; }

        let content = std::fs::read_to_string(path).unwrap_or_default();
        if content.is_empty() { continue; }

        let rel_path = path.strip_prefix(root).unwrap_or(path);
        if let Ok(matches) = search_pattern(rel_path, &content, pattern, lang) {
            for m in &matches {
                println!("{}:{}: {}", m.file_path, m.line_start, m.text.lines().next().unwrap_or(""));
                *total += 1;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "text-search")]
fn cmd_rg(pattern: &str, path: &Path, lang: Option<&str>) -> Result<(), String> {
    use deagle_parse::text_search::{search_directory, search_file};

    let lang_filter = lang.map(|l| Language::from_extension(match l {
        "rust" => "rs",
        "python" => "py",
        "go" => "go",
        "typescript" => "ts",
        "javascript" => "js",
        "java" => "java",
        "cpp" | "c++" => "cpp",
        "c" => "c",
        other => other,
    }));

    if !path.exists() {
        return Err(format!("Path not found: {}", path.display()));
    }

    let matches = if path.is_file() {
        // Single-file search: skip the directory walker so callers can
        // target specific files (matches ripgrep's `rg PAT file` UX).
        let content = std::fs::read(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        search_file(path, &content, pattern)
            .map_err(|e| format!("Search failed: {}", e))?
    } else {
        search_directory(path, pattern, lang_filter)
            .map_err(|e| format!("Search failed: {}", e))?
    };

    if matches.is_empty() {
        eprintln!("No matches for '{}'", pattern);
        return Ok(());
    }

    for m in &matches {
        println!("{}:{}: {}", m.file_path, m.line_number, m.line);
    }
    eprintln!("\n{} match(es)", matches.len());
    Ok(())
}
