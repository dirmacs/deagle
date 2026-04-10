//! deagle CLI — Rust-native code intelligence.
//!
//! Commands:
//! - `deagle map <DIR>` — index a codebase into the graph
//! - `deagle search <QUERY>` — search for symbols
//! - `deagle stats` — show graph statistics

use clap::{Parser, Subcommand};
use deagle_core::{GraphDb, Language};
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
    },
    /// Full-text keyword search (BM25 ranked via FTS5)
    Keyword {
        /// Search query (searches entity names and content)
        query: String,
    },
    /// Show graph statistics
    Stats,
    /// Structural AST pattern search (powered by ast-grep)
    #[cfg(feature = "pattern")]
    Sg {
        /// AST pattern (e.g., "$X.unwrap()", "fn $NAME() { $$$ }")
        pattern: String,
        /// Directory to search
        #[arg(default_value = ".")]
        dir: PathBuf,
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
        /// Directory to search
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Filter by language (e.g., "rust", "python")
        #[arg(long)]
        lang: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Map { dir, force } => cmd_map(&cli.db, &dir, force),
        Commands::Search { query, kind, fuzzy } => cmd_search(&cli.db, &query, kind.as_deref(), fuzzy),
        Commands::Keyword { query } => cmd_keyword(&cli.db, &query),
        Commands::Stats => cmd_stats(&cli.db),
        Commands::Loc { dir } => cmd_loc(&dir),
        #[cfg(feature = "pattern")]
        Commands::Sg { pattern, dir } => cmd_grep(&pattern, &dir),
        #[cfg(feature = "text-search")]
        Commands::Rg { pattern, dir, lang } => cmd_rg(&pattern, &dir, lang.as_deref()),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn cmd_map(db_path: &Path, dir: &Path, force: bool) -> Result<(), String> {
    use deagle_core::Edge;
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

    // Insert into DB sequentially (SQLite is single-writer)
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

        // Insert nodes and track their DB IDs
        let mut db_ids = Vec::new();
        for node in &result.nodes {
            match db.insert_node(node) {
                Ok(id) => db_ids.push(id),
                Err(_) => db_ids.push(-1),
            }
        }
        node_count += result.nodes.len();

        // Store file hash for incremental indexing
        let _ = db.store_file_hash(rel_path, content);

        // Insert edges using DB IDs
        for &(from_idx, to_idx, ref kind) in &result.edges {
            if from_idx < db_ids.len() && to_idx < db_ids.len() {
                let from_id = db_ids[from_idx];
                let to_id = db_ids[to_idx];
                if from_id > 0 && to_id > 0 {
                    let _ = db.insert_edge(&Edge {
                        from_id, to_id, kind: *kind, confidence: 1.0,
                    });
                    edge_count += 1;
                }
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

fn cmd_search(db_path: &Path, query: &str, kind: Option<&str>, fuzzy: bool) -> Result<(), String> {
    let db = GraphDb::open(db_path).map_err(|e| format!("Failed to open db: {}", e))?;
    let results = if fuzzy {
        db.fuzzy_search_nodes(query).map_err(|e| format!("Search failed: {}", e))?
    } else {
        db.search_nodes(query).map_err(|e| format!("Search failed: {}", e))?
    };

    let results: Vec<_> = if let Some(k) = kind {
        results.into_iter().filter(|n| n.kind.to_string() == k).collect()
    } else {
        results
    };

    if results.is_empty() {
        eprintln!("No results for '{}'", query);
        return Ok(());
    }

    println!("{:<30} {:<12} {:<10} {}", "NAME", "KIND", "LANG", "LOCATION");
    println!("{}", "-".repeat(80));
    for node in &results {
        println!(
            "{:<30} {:<12} {:<10} {}:{}",
            node.name, node.kind, node.language, node.file_path, node.line_start,
        );
    }
    println!("\n{} result(s)", results.len());
    Ok(())
}

fn cmd_keyword(db_path: &Path, query: &str) -> Result<(), String> {
    let db = GraphDb::open(db_path).map_err(|e| format!("Failed to open db: {}", e))?;
    let results = db.keyword_search(query).map_err(|e| format!("Keyword search failed: {}", e))?;

    if results.is_empty() {
        eprintln!("No keyword matches for '{}'", query);
        return Ok(());
    }

    println!("{:<30} {:<12} {:<10} {}", "NAME", "KIND", "LANG", "LOCATION");
    println!("{}", "-".repeat(80));
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

fn cmd_stats(db_path: &Path) -> Result<(), String> {
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
        eprintln!("No matches found");
    } else {
        eprintln!("\n{} match(es)", total);
    }
    Ok(())
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
fn cmd_rg(pattern: &str, dir: &Path, lang: Option<&str>) -> Result<(), String> {
    use deagle_parse::text_search::search_directory;

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

    let matches = search_directory(dir, pattern, lang_filter)
        .map_err(|e| format!("Search failed: {}", e))?;

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
