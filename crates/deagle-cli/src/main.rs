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
    },
    /// Search for symbols by name
    Search {
        /// Search query (substring match)
        query: String,
        /// Filter by entity kind
        #[arg(long)]
        kind: Option<String>,
    },
    /// Show graph statistics
    Stats,
    /// Structural pattern search (powered by ast-grep)
    #[cfg(feature = "pattern")]
    Grep {
        /// AST pattern (e.g., "$X.unwrap()", "fn $NAME() { $$$ }")
        pattern: String,
        /// Directory to search
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Map { dir } => cmd_map(&cli.db, &dir),
        Commands::Search { query, kind } => cmd_search(&cli.db, &query, kind.as_deref()),
        Commands::Stats => cmd_stats(&cli.db),
        #[cfg(feature = "pattern")]
        Commands::Grep { pattern, dir } => cmd_grep(&pattern, &dir),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn cmd_map(db_path: &Path, dir: &Path) -> Result<(), String> {
    // Ensure db directory exists
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create db dir: {}", e))?;
    }

    let db = GraphDb::open(db_path).map_err(|e| format!("Failed to open db: {}", e))?;
    db.clear().map_err(|e| format!("Failed to clear db: {}", e))?;

    eprintln!("Indexing {}...", dir.display());

    let mut file_count = 0;
    let mut node_count = 0;
    walk_and_parse(dir, dir, &db, &mut file_count, &mut node_count)
        .map_err(|e| format!("Failed to index: {}", e))?;

    eprintln!("Indexed {} files, {} entities", file_count, node_count);
    eprintln!("Database: {}", db_path.display());
    Ok(())
}

fn walk_and_parse(
    root: &Path,
    dir: &Path,
    db: &GraphDb,
    file_count: &mut usize,
    node_count: &mut usize,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;

    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();

        // Skip hidden dirs, target/, node_modules/, .git/
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "target" || name == "node_modules" || name == "vendor" {
                continue;
            }
            walk_and_parse(root, &path, db, file_count, node_count)?;
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = Language::from_extension(ext);
        if lang == Language::Unknown {
            continue;
        }

        let content = std::fs::read_to_string(&path).unwrap_or_default();
        if content.is_empty() {
            continue;
        }

        let rel_path = path.strip_prefix(root).unwrap_or(&path);
        match deagle_parse::parse_file(rel_path, &content, lang) {
            Ok(nodes) => {
                for node in &nodes {
                    let _ = db.insert_node(node);
                }
                *node_count += nodes.len();
                *file_count += 1;
            }
            Err(e) => {
                eprintln!("  Warning: {} — {}", rel_path.display(), e);
            }
        }
    }
    Ok(())
}

fn cmd_search(db_path: &Path, query: &str, kind: Option<&str>) -> Result<(), String> {
    let db = GraphDb::open(db_path).map_err(|e| format!("Failed to open db: {}", e))?;
    let results = db.search_nodes(query).map_err(|e| format!("Search failed: {}", e))?;

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
fn grep_walk(root: &Path, dir: &Path, pattern: &str, total: &mut usize) -> Result<(), String> {
    use deagle_parse::pattern::search_pattern;
    let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "target" || name == "node_modules" || name == "vendor" {
                continue;
            }
            grep_walk(root, &path, pattern, total)?;
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = Language::from_extension(ext);
        if lang == Language::Unknown {
            continue;
        }

        let content = std::fs::read_to_string(&path).unwrap_or_default();
        if content.is_empty() {
            continue;
        }

        let rel_path = path.strip_prefix(root).unwrap_or(&path);
        if let Ok(matches) = search_pattern(rel_path, &content, pattern, lang) {
            for m in &matches {
                println!("{}:{}: {}", m.file_path, m.line_start, m.text.lines().next().unwrap_or(""));
                *total += 1;
            }
        }
    }
    Ok(())
}
