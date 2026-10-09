//! deagle CLI — Rust-native code intelligence.
//!
//! Commands:
//! - `deagle map <DIR>` — index a codebase into the graph
//! - `deagle search <QUERY>` — search for symbols
//! - `deagle stats` — show graph statistics

use clap::{Parser, Subcommand};
use deagle_core::{Edge, EdgeKind, Freshness, GraphDb, Language};
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
        /// Refuse to answer from a stale index: re-hash every cited file and,
        /// if any is STALE (content changed since indexing), exit 2 without
        /// printing results; if any file's freshness could not be determined
        /// (unreadable file, or no hash row — zero-node files never get one),
        /// exit 3. A fully fresh index answers normally and exits 0.
        #[arg(long)]
        require_fresh: bool,
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
        /// Refuse to answer from a stale index: re-hash every cited file and,
        /// if any is STALE (content changed since indexing), exit 2 without
        /// printing results; if any file's freshness could not be determined
        /// (unreadable file, or no hash row — zero-node files never get one),
        /// exit 3. A fully fresh index answers normally and exits 0.
        #[arg(long)]
        require_fresh: bool,
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
        /// Print the match column: `file:line:column: text` (1-indexed
        /// characters, the same unit as `deagle rg --column`)
        #[arg(long)]
        column: bool,
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
        /// Print the match column: `file:line:column: text`
        #[arg(long)]
        column: bool,
        /// Print one JSON object per match (file_path, line_number, column,
        /// byte_offset, line) instead of the text format
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Map { dir, force } => cmd_map(&cli.db, &dir, force),
        Commands::Search {
            query,
            kind,
            fuzzy,
            lang,
            require_fresh,
            paths,
        } => cmd_search(
            &cli.db,
            &query,
            kind.as_deref(),
            fuzzy,
            lang.as_deref(),
            require_fresh,
            &paths,
        ),
        Commands::Keyword {
            query,
            require_fresh,
        } => cmd_keyword(&cli.db, &query, require_fresh),
        Commands::Stats { hint_path } => cmd_stats(&cli.db, hint_path.as_deref()),
        Commands::Loc { dir } => cmd_loc(&dir),
        #[cfg(feature = "pattern")]
        Commands::Sg {
            pattern,
            paths,
            column,
        } => paths
            .iter()
            .try_for_each(|path| cmd_grep(&pattern, path, column)),
        #[cfg(feature = "text-search")]
        Commands::Rg {
            pattern,
            paths,
            lang,
            column,
            json,
        } => paths
            .iter()
            .try_for_each(|path| cmd_rg(&pattern, path, lang.as_deref(), column, json)),
    };

    if let Err(e) = result {
        // Freshness failures carry their own exit code so a caller can tell
        // "the index is stale" (2) from "the index cannot be verified" (3)
        // from any ordinary failure (1). The verdict lines were already
        // printed by the command itself; here we only refuse to answer.
        if let Some(code) = e.strip_prefix("FRESHNESS_CHECK_FAILED:") {
            let code: i32 = code.parse().unwrap_or(1);
            eprintln!(
                "refusing to answer from an unverified index (--require-fresh); re-run `deagle map` first"
            );
            std::process::exit(code);
        }
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

    // Paths are keyed relative to the indexed root, so a second root makes the
    // same filename collide with a different root's rows: `remove_file` deletes
    // the other root's nodes and every later call finds the first root stale
    // again. Refuse the mismatch rather than corrupt the index -- see #6.
    let root = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let dir_str = root.to_string_lossy().to_string();

    if force {
        db.clear()
            .map_err(|e| format!("Failed to clear db: {}", e))?;
        db.metadata_set(deagle_core::INDEX_ROOT_KEY, &dir_str)
            .map_err(|e| format!("Failed to record index root: {}", e))?;
        eprintln!("Full re-index of {}...", dir_str);
    } else {
        match db
            .metadata_get(deagle_core::INDEX_ROOT_KEY)
            .map_err(|e| format!("Failed to read index root: {}", e))?
        {
            Some(stored) if stored != dir_str => {
                return Err(format!(
                    "this database was indexed from {}, not {}.\n\
                     Indexing a second root corrupts the first: paths are keyed \
                     relative to the root.\nRe-index the original root, or pass \
                     --force to clear and start from {}.",
                    stored, dir_str, dir_str
                ));
            }
            None => db
                .metadata_set(deagle_core::INDEX_ROOT_KEY, &dir_str)
                .map_err(|e| format!("Failed to record index root: {}", e))?,
            Some(_) => {}
        }
        eprintln!("Incremental index of {}...", dir_str);
    }

    // Collect file paths first (ignore-aware)
    let files: Vec<_> = ignore::WalkBuilder::new(dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build()
        .flatten()
        .filter(|e| e.path().is_file())
        .filter(|e| {
            let ext = e.path().extension().and_then(|x| x.to_str()).unwrap_or("");
            Language::from_extension(ext) != Language::Unknown
        })
        .collect();

    // Pre-filter: check hashes sequentially (SQLite not thread-safe), then parse in parallel
    let files_to_parse: Vec<_> = files
        .iter()
        .filter(|entry| {
            if force {
                return true;
            }
            let path = entry.path();
            let rel_path = path.strip_prefix(dir).unwrap_or(path);
            let rel_str = rel_path.to_string_lossy();
            let content = match std::fs::read_to_string(path) {
                Ok(c) if !c.is_empty() => c,
                _ => return false,
            };
            db.needs_reindex(&rel_str, &content).unwrap_or(true)
        })
        .collect();

    // Parse changed files in parallel with rayon
    let results: Vec<_> = files_to_parse
        .par_iter()
        .filter_map(|entry| {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let lang = Language::from_extension(ext);
            let content = std::fs::read_to_string(path).ok()?;
            if content.is_empty() {
                return None;
            }
            let rel_path = path.strip_prefix(dir).unwrap_or(path);
            let rel_str = rel_path.to_string_lossy().to_string();

            deagle_parse::parse_file_with_edges(rel_path, &content, lang)
                .ok()
                .map(|r| (rel_str, content, r))
        })
        .collect();

    // Batch insert into DB (single transaction per file for speed)
    let mut file_count = 0;
    let mut node_count = 0;
    let mut edge_count = 0;

    for (rel_path, content, result) in &results {
        if result.nodes.is_empty() {
            continue;
        }

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
        let resolved_edges: Vec<(i64, i64, EdgeKind)> = result
            .edges
            .iter()
            .filter(|(from_idx, to_idx, _)| {
                *from_idx < db_ids.len()
                    && *to_idx < db_ids.len()
                    && db_ids[*from_idx] > 0
                    && db_ids[*to_idx] > 0
            })
            .map(|(from_idx, to_idx, kind)| (db_ids[*from_idx], db_ids[*to_idx], *kind))
            .collect();
        edge_count += resolved_edges.len();

        if !resolved_edges.is_empty() {
            // Insert edges in their own batch (nodes already committed)
            for (from_id, to_id, kind) in &resolved_edges {
                let _ = db.insert_edge(&Edge {
                    from_id: *from_id,
                    to_id: *to_id,
                    kind: *kind,
                    confidence: 1.0,
                });
            }
        }
    }

    let total_files = files.len();
    let skipped = total_files - file_count;
    if skipped > 0 {
        eprintln!(
            "Indexed {} files ({} unchanged, skipped), {} entities, {} edges",
            file_count, skipped, node_count, edge_count
        );
    } else {
        eprintln!(
            "Indexed {} files, {} entities, {} edges",
            file_count, node_count, edge_count
        );
    }
    eprintln!("Database: {}", db_path.display());
    Ok(())
}

/// Whether a node stored under `stored` lies inside the caller's `given` path.
///
/// Stored paths are keyed RELATIVE to the index root -- that is why `cmd_map`
/// refuses to index a second root, since the two keyspaces would be mixed. So
/// a caller path has to be brought into that same keyspace before comparing.
///
/// The previous predicate compared an absolute path against a relative stored
/// path, which can never match; the reason `.../repo/src` appeared to work was a
/// fallback that tested the last path component as a bare substring. That made a
/// path this index does not contain byte-identical to one that does, so a wrong
/// subject reported itself as an absent symbol.
fn path_matches(stored: &str, given: &Path, root: Option<&Path>) -> bool {
    let given = given.to_string_lossy();
    let given = given.trim_end_matches('/');
    if given.is_empty() {
        return true;
    }
    // Reduce the caller's path into index-key space when the root is known.
    let rel = match root {
        Some(root) => Path::new(&given)
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| given.to_string()),
        None => given.to_string(),
    };
    let rel = rel.trim_start_matches("./").trim_end_matches('/');
    if stored == rel || stored.starts_with(&format!("{rel}/")) {
        return true;
    }
    // A directory prefix in either keyspace, e.g. "src" for "src/lib.rs".
    stored.starts_with(&rel.to_string()) || rel.ends_with(stored)
}

fn describe_index(db: &GraphDb, db_path: &Path) -> String {
    let root = db
        .metadata_get(deagle_core::INDEX_ROOT_KEY)
        .ok()
        .flatten()
        .unwrap_or_else(|| "<no root recorded>".to_string());
    match std::fs::metadata(db_path).and_then(|m| m.modified()) {
        Ok(modified) => match modified.elapsed() {
            Ok(age) => format!(
                "index root: {root}  (database last modified {})",
                human_age(age)
            ),
            Err(_) => format!("index root: {root}"),
        },
        Err(_) => format!("index root: {root}"),
    }
}

/// A coarse age, not a date. Nothing here needs a calendar, and the only
/// timestamp available is a file's, so "how long ago" is the honest unit.
fn human_age(age: std::time::Duration) -> String {
    let secs = age.as_secs();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3_600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3_600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Enriched with a per-file freshness verdict — the unit `search`/`keyword`
/// print, so a coordinate can never again be emitted without its provenance.
struct ScoredNode {
    node: deagle_core::Node,
    freshness: deagle_core::FreshnessRow,
}

/// Exit code for `--require-fresh` when at least one cited file is STALE.
const EXIT_STALE: i32 = 2;
/// Exit code for `--require-fresh` when freshness could not be derived for at
/// least one cited file (unreadable file, or no hash row — a file the index
/// never hashed is unknown, not clean).
const EXIT_UNVERIFIABLE: i32 = 3;

/// Resolve the index-root-relative `stored` path against `root` for hashing.
///
/// The root recorded in the index is authoritative: it is where `map` read
/// the file from, and keys are relative to it. With no root recorded — an
/// index that predates root tracking — the file may live anywhere; the probe
/// then answers could-not-determine rather than guess a filesystem location,
/// because a guess that misses would manufacture a verdict.
fn resolve_for_hash(stored: &str, root: Option<&Path>) -> Option<PathBuf> {
    let p = Path::new(stored);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    root.map(|r| r.join(p))
}

/// Freshness of every file a result set cites: one re-hash per cited file,
/// never a whole-index scan. Results are grouped by stored path so a file is
/// hashed once however many results cite it; results are returned in their
/// original order, each carrying its file's verdict.
fn freshness_for_results(db: &GraphDb, results: &[deagle_core::Node]) -> Vec<ScoredNode> {
    let root = db
        .metadata_get(deagle_core::INDEX_ROOT_KEY)
        .ok()
        .flatten()
        .map(PathBuf::from);
    let mut by_file: std::collections::HashMap<String, deagle_core::FreshnessRow> =
        std::collections::HashMap::new();
    for node in results {
        if !by_file.contains_key(&node.file_path) {
            let row = match resolve_for_hash(&node.file_path, root.as_deref()) {
                Some(on_disk) => db.file_freshness(&node.file_path, &on_disk),
                None => deagle_core::FreshnessRow {
                    stored_path: node.file_path.clone(),
                    on_disk_path: PathBuf::from(&node.file_path),
                    verdict: Freshness::CouldNotDetermine,
                    indexed_at: None,
                    file_mtime: None,
                },
            };
            by_file.insert(node.file_path.clone(), row);
        }
    }
    results
        .iter()
        .map(|node| ScoredNode {
            node: node.clone(),
            freshness: by_file[&node.file_path].clone(),
        })
        .collect()
}

/// One verdict token per result row. `fresh` prints blank — a fresh row is
/// the normal case and must not add noise; the summary line says "all fresh"
/// once. Stale and unknown rows name themselves, because those are the rows a
/// reader must not trust at the cited line numbers.
fn verdict_label(v: Freshness) -> &'static str {
    match v {
        Freshness::Fresh => "",
        Freshness::Stale => "STALE",
        Freshness::CouldNotDetermine => "UNKNOWN",
    }
}

/// Print the freshness summary line the issue asks for and return the exit
/// code the query should use: 2 when any cited file is stale, 3 when any is
/// unverifiable, 0 when every cited file is fresh. The line goes to stderr so
/// piped consumers see only the rows, and it is impossible to silence: every
/// answered query emits exactly one.
fn report_freshness(rows: &[ScoredNode], require_fresh: bool) -> Result<(), String> {
    // Report per FILE, not per result row: several results can cite the same
    // file, and printing the verdict once per row would make one stale file
    // read as many. The per-row STATE column already names the file on every
    // row; the summary names each distinct file once.
    let mut stale: Vec<&ScoredNode> = Vec::new();
    let mut unknown: Vec<&ScoredNode> = Vec::new();
    let mut seen_stale: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut seen_unknown: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for r in rows {
        match r.freshness.verdict {
            Freshness::Stale => {
                if seen_stale.insert(r.freshness.stored_path.as_str()) {
                    stale.push(r);
                }
            }
            Freshness::CouldNotDetermine => {
                if seen_unknown.insert(r.freshness.stored_path.as_str()) {
                    unknown.push(r);
                }
            }
            Freshness::Fresh => {}
        }
    }

    if !stale.is_empty() {
        eprintln!();
        for r in &stale {
            let indexed = r
                .freshness
                .indexed_at
                .as_deref()
                .unwrap_or("<no indexed_at recorded>");
            let now = match r.freshness.file_mtime.and_then(|m| m.elapsed().ok()) {
                Some(age) => {
                    format!(
                        "{} (file modified {})",
                        r.freshness.stored_path,
                        human_age(age)
                    )
                }
                None => r.freshness.stored_path.clone(),
            };
            eprintln!(
                "STALE (indexed {}, now {}) — results may be wrong; re-run deagle map",
                indexed, now
            );
        }
    }
    if !unknown.is_empty() {
        eprintln!();
        for r in &unknown {
            eprintln!(
                "FRESHNESS UNKNOWN: {} — freshness unverifiable (unreadable file, or no recorded hash)",
                r.freshness.stored_path
            );
        }
    }

    if stale.is_empty() && unknown.is_empty() {
        eprintln!("freshness: all cited files FRESH (content matches the index)");
        return Ok(());
    }
    if !require_fresh {
        return Ok(());
    }
    if !stale.is_empty() {
        return Err(format!("FRESHNESS_CHECK_FAILED:{}", EXIT_STALE));
    }
    Err(format!("FRESHNESS_CHECK_FAILED:{}", EXIT_UNVERIFIABLE))
}

fn cmd_search(
    db_path: &Path,
    query: &str,
    kind: Option<&str>,
    fuzzy: bool,
    lang: Option<&str>,
    require_fresh: bool,
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
                    let lang_filter = lang.map(|l| {
                        Language::from_extension(match l {
                            "rust" | "rs" => "rs",
                            "python" | "py" => "py",
                            "go" => "go",
                            "typescript" | "ts" => "ts",
                            "javascript" | "js" => "js",
                            other => other,
                        })
                    });
                    match search_directory(path, query, lang_filter) {
                        Ok(matches) if !matches.is_empty() => {
                            print_text_matches(&matches, false, false);
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
    // Say which database is answering before the rows that depend on it.
    eprintln!("{}", describe_index(&db, db_path));
    let results = if fuzzy {
        db.fuzzy_search_nodes(query)
            .map_err(|e| format!("Search failed: {}", e))?
    } else {
        db.search_nodes(query)
            .map_err(|e| format!("Search failed: {}", e))?
    };

    // Apply kind filter
    let results: Vec<_> = if let Some(k) = kind {
        results
            .into_iter()
            .filter(|n| n.kind.to_string() == k)
            .collect()
    } else {
        results
    };

    // Apply language filter (--lang / -l)
    let results: Vec<_> = if let Some(l) = lang {
        let l_lower = l.to_lowercase();
        results
            .into_iter()
            .filter(|n| {
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
            })
            .collect()
    } else {
        results
    };

    // The index root, as recorded by `map`. Needed to interpret any path the
    // caller passes: stored keys are relative to it.
    let root: Option<std::path::PathBuf> = db
        .metadata_get(deagle_core::INDEX_ROOT_KEY)
        .ok()
        .flatten()
        .map(std::path::PathBuf::from);

    // Results before the path scope is applied, kept so an empty answer can say
    // WHICH filter produced it.
    let unfiltered = results.clone();

    // Apply path scope filter (positional paths).
    // The graph stores paths relative to the indexed root (e.g. "crates/foo/src/bar.rs").
    // Users may pass absolute paths (e.g. /path/to/project/crates) or relative ones.
    // Match if:
    //   (a) stored path starts with the given path, OR
    //   (b) stored path contains any component of the given path as a substring
    let results: Vec<_> = if !paths.is_empty() {
        results
            .into_iter()
            .filter(|n| {
                paths
                    .iter()
                    .any(|p| path_matches(&n.file_path, p, root.as_deref()))
            })
            .collect()
    } else {
        results
    };

    if results.is_empty() {
        // The two empties must not read alike. A query that matched nothing is
        // a fact about the symbol; a query that matched and whose hits the path
        // filter removed is a fact about the PATH, and reporting the first when
        // the second happened is how a wrong subject becomes a reported absence.
        if paths.is_empty() || unfiltered.is_empty() {
            eprintln!("No results for '{}'", query);
            return Ok(());
        }
        eprintln!(
            "No results for '{}': the query matched {} node(s), but none are under {}",
            query,
            unfiltered.len(),
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut seen: Vec<&str> = unfiltered.iter().map(|n| n.file_path.as_str()).collect();
        seen.sort_unstable();
        seen.dedup();
        eprintln!("indexed files holding a match (paths are keyed relative to the index root):");
        for p in seen.iter().take(10) {
            eprintln!("  {}", p);
        }
        if seen.len() > 10 {
            eprintln!("  ... and {} more", seen.len() - 10);
        }
        eprintln!(
            "if the path above is where you expected the match, this index does not contain it \
             (wrong subject) rather than the symbol being absent (wrong query)."
        );
        return Ok(());
    }

    // Every emitted coordinate is index-time data. Before printing one row,
    // re-hash the cited files and attach the verdict, so a coordinate can
    // never be emitted silently when its file has moved on.
    let scored = freshness_for_results(&db, &results);
    report_freshness(&scored, require_fresh)?;

    println!(
        "{:<30} {:<12} {:<10} {:<8} LOCATION",
        "NAME", "KIND", "LANG", "STATE"
    );
    let sep = "-".repeat(88);
    println!("{sep}");
    for r in &scored {
        println!(
            "{:<30} {:<12} {:<10} {:<8} {}:{}",
            r.node.name,
            r.node.kind,
            r.node.language,
            verdict_label(r.freshness.verdict),
            r.node.file_path,
            r.node.line_start,
        );
    }
    println!("\n{} result(s)", scored.len());
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

fn cmd_keyword(db_path: &Path, query: &str, require_fresh: bool) -> Result<(), String> {
    let db = GraphDb::open(db_path).map_err(|e| format!("Failed to open db: {}", e))?;
    // FTS5 has its own query syntax — it doesn't understand regex alternation.
    // Agents (and humans) frequently pass `foo\|bar` or `foo|bar` expecting
    // "foo OR bar". Rewrite those into FTS5 OR syntax before the query runs
    // and strip characters FTS5 treats as operators so random input doesn't
    // crash the parser with `fts5: syntax error near "..."`.
    let sanitized = sanitize_fts5_query(query);
    let results = db
        .keyword_search(&sanitized)
        .map_err(|e| format!("Keyword search failed: {}", e))?;

    if results.is_empty() {
        eprintln!("No keyword matches for '{}'", query);
        return Ok(());
    }

    let scored = freshness_for_results(&db, &results);
    report_freshness(&scored, require_fresh)?;

    println!(
        "{:<30} {:<12} {:<10} {:<8} LOCATION",
        "NAME", "KIND", "LANG", "STATE"
    );
    let sep = "-".repeat(88);
    println!("{sep}");
    for r in &scored {
        println!(
            "{:<30} {:<12} {:<10} {:<8} {}:{}",
            r.node.name,
            r.node.kind,
            r.node.language,
            verdict_label(r.freshness.verdict),
            r.node.file_path,
            r.node.line_start,
        );
    }
    println!("\n{} result(s) (BM25 ranked)", scored.len());
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

    println!(
        "{:<20} {:>8} {:>8} {:>8} {:>8}",
        "LANGUAGE", "FILES", "CODE", "COMMENTS", "BLANKS"
    );
    println!("{}", "-".repeat(60));

    let mut total_files = 0usize;
    let mut total_code = 0usize;
    let mut total_comments = 0usize;
    let mut total_blanks = 0usize;

    let mut sorted: Vec<_> = languages.iter().collect();
    sorted.sort_by_key(|a| std::cmp::Reverse(a.1.code));

    for (lang_type, lang) in &sorted {
        if lang.code == 0 && lang.comments == 0 {
            continue;
        }
        let files = lang.reports.len();
        println!(
            "{:<20} {:>8} {:>8} {:>8} {:>8}",
            format!("{}", lang_type),
            files,
            lang.code,
            lang.comments,
            lang.blanks
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
    println!("{}", describe_index(&db, db_path));
    println!("Nodes:    {}", nodes);
    println!("Edges:    {}", edges);
    Ok(())
}

#[cfg(feature = "pattern")]
fn cmd_grep(pattern: &str, dir: &Path, column: bool) -> Result<(), String> {
    if !dir.exists() {
        return Err(format!("Directory not found: {}", dir.display()));
    }

    eprintln!("Searching for pattern: {}", pattern);

    let mut total = 0;
    grep_walk(dir, dir, pattern, &mut total, column)?;

    if total == 0 {
        // AST pattern returned nothing. Try two fallbacks:
        // 1. If pattern looks like an incomplete declaration (no braces/parens),
        //    suggest the completed form.
        // 2. Fall back to ripgrep text search with the same string.
        let hint = suggest_pattern_completion(pattern);
        if let Some(ref completed) = hint {
            eprintln!(
                "note: ast pattern found 0 matches. Trying completed form: {}",
                completed
            );
            let mut total2 = 0;
            grep_walk(dir, dir, completed, &mut total2, column)?;
            if total2 > 0 {
                eprintln!(
                    "\n{} match(es) (with completed pattern '{}')",
                    total2, completed
                );
                eprintln!(
                    "tip: use `deagle sg \"{}\"` next time for direct match.",
                    completed
                );
                return Ok(());
            }
        }

        // Final fallback: plain text search
        eprintln!(
            "No AST matches found.\n\
             tip: AST patterns need full syntax, e.g. `pub enum Foo {{ $$$ }}`\n\
             Falling back to text search for '{}':",
            pattern
        );
        #[cfg(feature = "text-search")]
        {
            use deagle_parse::text_search::search_directory;
            if let Ok(matches) = search_directory(dir, pattern, None) {
                if matches.is_empty() {
                    eprintln!("No text matches either.");
                } else {
                    print_text_matches(&matches, false, false);
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
    if t.contains('{') || t.contains('(') {
        return None;
    }

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
fn grep_walk(
    root: &Path,
    _dir: &Path,
    pattern: &str,
    total: &mut usize,
    column: bool,
) -> Result<(), String> {
    use deagle_parse::pattern::search_pattern;

    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .build();

    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = Language::from_extension(ext);
        if lang == Language::Unknown {
            continue;
        }

        let content = std::fs::read_to_string(path).unwrap_or_default();
        if content.is_empty() {
            continue;
        }

        let rel_path = path.strip_prefix(root).unwrap_or(path);
        if let Ok(matches) = search_pattern(rel_path, &content, pattern, lang) {
            for m in &matches {
                let text = m.text.lines().next().unwrap_or("");
                if column {
                    println!("{}:{}:{}: {}", m.file_path, m.line_start, m.col_start, text);
                } else {
                    println!("{}:{}: {}", m.file_path, m.line_start, text);
                }
                *total += 1;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "text-search")]
fn cmd_rg(
    pattern: &str,
    path: &Path,
    lang: Option<&str>,
    column: bool,
    json: bool,
) -> Result<(), String> {
    use deagle_parse::text_search::{search_directory, search_file};

    let lang_filter = lang.map(|l| {
        Language::from_extension(match l {
            "rust" => "rs",
            "python" => "py",
            "go" => "go",
            "typescript" => "ts",
            "javascript" => "js",
            "java" => "java",
            "cpp" | "c++" => "cpp",
            "c" => "c",
            other => other,
        })
    });

    if !path.exists() {
        return Err(format!("Path not found: {}", path.display()));
    }

    let matches = if path.is_file() {
        // Single-file search: skip the directory walker so callers can
        // target specific files (matches ripgrep's `rg PAT file` UX).
        let content =
            std::fs::read(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        search_file(path, &content, pattern).map_err(|e| format!("Search failed: {}", e))?
    } else {
        search_directory(path, pattern, lang_filter).map_err(|e| format!("Search failed: {}", e))?
    };

    if matches.is_empty() {
        eprintln!("No matches for '{}'", pattern);
        return Ok(());
    }

    print_text_matches(&matches, column, json);
    eprintln!("\n{} match(es)", matches.len());
    Ok(())
}

/// Print `text_search` matches.
///
/// The default format is `file:line: text`. `--column` adds the 1-indexed
/// character column (`file:line:column: text`, as `rg --column` does);
/// `--json` emits one JSON object per match so a consumer gets `column` and
/// `byte_offset` without re-parsing the line itself.
#[cfg(feature = "text-search")]
fn print_text_matches(matches: &[deagle_parse::text_search::TextMatch], column: bool, json: bool) {
    for m in matches {
        if json {
            match serde_json::to_string(m) {
                Ok(line) => println!("{line}"),
                Err(e) => eprintln!("failed to serialize match: {e}"),
            }
        } else if column {
            println!("{}:{}:{}: {}", m.file_path, m.line_number, m.column, m.line);
        } else {
            println!("{}:{}: {}", m.file_path, m.line_number, m.line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deagle_core::GraphDb;

    /// A second root must be refused, not silently corrupt the index. Paths are
    /// keyed relative to the indexed root, so the same filename in two roots
    /// collides and `remove_file` deletes the other root's nodes.
    ///
    /// Re-indexing the *same* root is the must-pass control: without it, a guard
    /// that rejected everything would pass every rejection assertion while being
    /// useless.
    #[test]
    fn map_refuses_a_second_root() {
        let base = std::env::temp_dir().join(format!("deagle-cli-root-{}", std::process::id()));
        let a = base.join("a");
        let b = base.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("lib.rs"), "struct OnlyInA;\n").unwrap();
        std::fs::write(b.join("lib.rs"), "struct OnlyInB;\n").unwrap();
        let db_path = base.join("db/graph.db");

        // Control: the first root indexes cleanly.
        cmd_map(&db_path, &a, false).expect("first root must index");

        // Control: the SAME root again must succeed.
        cmd_map(&db_path, &a, false).expect("re-indexing the same root must not be refused");

        let nodes_after_a = {
            let db = GraphDb::open(&db_path).unwrap();
            db.node_count().unwrap()
        };
        assert!(nodes_after_a > 0, "root A's nodes are indexed");

        // A different root must be refused.
        let err = cmd_map(&db_path, &b, false).expect_err("a second root must be refused");
        assert!(
            err.contains(&a.canonicalize().unwrap().to_string_lossy().to_string()),
            "the error must name the root already indexed, got: {err}"
        );
        assert!(
            err.contains("--force"),
            "the error must name the escape, got: {err}"
        );

        // And the refusal must leave the first root intact.
        let db = GraphDb::open(&db_path).unwrap();
        assert_eq!(
            db.node_count().unwrap(),
            nodes_after_a,
            "a refused second root must not destroy the first root's nodes"
        );

        // `--force` is the documented escape: it clears, so a new root is fine.
        cmd_map(&db_path, &b, true).expect("--force permits a new root");

        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A path filter must be read in the index's own keyspace.
    ///
    /// Stored keys are relative to the index root, so an absolute caller path
    /// like `/repo/src` has to be reduced before it can match `src/lib.rs`. The
    /// old predicate compared the two directly and fell back to testing the last
    /// component as a substring, which made a path that does not exist
    /// indistinguishable from one that does — and silently dropped real hits.
    #[test]
    fn path_filter_understands_the_index_keyspace() {
        let root = Path::new("/tmp/proj");
        assert!(path_matches(
            "src/lib.rs",
            Path::new("/tmp/proj/src"),
            Some(root)
        ));
        assert!(path_matches(
            "src/lib.rs",
            Path::new("/tmp/proj/src/lib.rs"),
            Some(root)
        ));
        assert!(path_matches(
            "src/lib.rs",
            Path::new("/tmp/proj"),
            Some(root)
        ));
        assert!(path_matches(
            "src/lib.rs",
            Path::new("/tmp/proj/src/"),
            Some(root)
        ));
        assert!(path_matches("src/lib.rs", Path::new("src"), Some(root)));
    }

    /// The direction that matters for correctness: a path outside the indexed
    /// tree must NOT match, however similar it looks. This is what makes a wrong
    /// subject reportable instead of silently absent.
    #[test]
    fn a_path_outside_the_index_never_matches() {
        let root = Path::new("/tmp/proj");
        assert!(!path_matches(
            "src/lib.rs",
            Path::new("/opt/other/src"),
            Some(root)
        ));
        assert!(!path_matches(
            "src/lib.rs",
            Path::new("/nonexistent/path"),
            Some(root)
        ));
        assert!(!path_matches(
            "src/lib.rs",
            Path::new("/tmp/proj/srclib"),
            Some(root)
        ));
    }

    /// The provenance line must name the recorded root: that is the only thing
    /// that distinguishes two databases on one host. A database with no root
    /// recorded must say so rather than print a blank field.
    #[test]
    fn describe_index_names_the_recorded_root() {
        let base =
            std::env::temp_dir().join(format!("deagle-cli-provenance-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("lib.rs"), "struct OnlyHere;\n").unwrap();
        let db_path = base.join("db/graph.db");

        cmd_map(&db_path, &base, false).expect("index the fixture root");
        let db = GraphDb::open(&db_path).unwrap();
        let line = describe_index(&db, &db_path);
        assert!(
            line.contains("deagle-cli-provenance"),
            "the line must name the indexed root, got: {line}"
        );
        assert!(
            line.contains("last modified"),
            "it must also date the database, got: {line}"
        );

        // Control: a database with no root recorded says so, rather than
        // printing a blank that reads like a root.
        let bare_path = base.join("db/bare.db");
        let bare = GraphDb::open(&bare_path).unwrap();
        let bare_line = describe_index(&bare, &bare_path);
        assert!(
            bare_line.contains("<no root recorded>"),
            "an unrecorded root must be named as such, got: {bare_line}"
        );

        std::fs::remove_dir_all(&base).unwrap();
    }

    /// The provenance line only earns its keep if it can tell two mapped graphs
    /// apart -- the whole point of #11 is that a reader cannot otherwise tell
    /// which database answered. A single-root test cannot show that: the fixture's
    /// temp-dir prefix is shared by every path beneath it, so an implementation
    /// reporting the database's *location* passes while naming no root at all.
    ///
    /// So map two roots into two databases, and require each line to name its own
    /// root and not the other's. The root names appear nowhere in the database
    /// paths, so a line derived from the database path fails here.
    #[test]
    fn describe_index_distinguishes_two_mapped_roots() {
        let base = std::env::temp_dir().join(format!("deagle-cli-two-{}", std::process::id()));
        let zebra = base.join("zebra-root");
        let quokka = base.join("quokka-root");
        std::fs::create_dir_all(&zebra).unwrap();
        std::fs::create_dir_all(&quokka).unwrap();
        std::fs::write(zebra.join("lib.rs"), "struct OnlyInZebra;\n").unwrap();
        std::fs::write(quokka.join("lib.rs"), "struct OnlyInQuokka;\n").unwrap();

        // Databases live in a separate tree so no database path contains a root name.
        let db_zebra = base.join("dbs/a/graph.db");
        let db_quokka = base.join("dbs/b/graph.db");
        cmd_map(&db_zebra, &zebra, false).expect("index zebra");
        cmd_map(&db_quokka, &quokka, false).expect("index quokka");

        let line_zebra = describe_index(&GraphDb::open(&db_zebra).unwrap(), &db_zebra);
        let line_quokka = describe_index(&GraphDb::open(&db_quokka).unwrap(), &db_quokka);

        assert!(
            line_zebra.contains("zebra-root"),
            "zebra's line must name the root it was indexed from, got: {line_zebra}"
        );
        assert!(
            !line_zebra.contains("quokka"),
            "zebra's line must not name the other root, got: {line_zebra}"
        );
        assert!(
            line_quokka.contains("quokka-root"),
            "quokka's line must name the root it was indexed from, got: {line_quokka}"
        );
        assert!(
            !line_quokka.contains("zebra"),
            "quokka's line must not name the other root, got: {line_quokka}"
        );

        std::fs::remove_dir_all(&base).unwrap();
    }

    /// End-to-end over a real fixture directory: a fresh index must render
    /// every cited file FRESH and answer; after the file's content changes
    /// the same query must name it STALE; and a cited file with no hash row
    /// — the zero-node trap — must render could-not-determine, never stale.
    /// `report_freshness` is the seam the assertions hang on: it is what both
    /// `cmd_search` and `cmd_keyword` call, so testing it tests both.
    #[test]
    fn freshness_signal_discriminates_all_three_states() {
        let base = std::env::temp_dir().join(format!("deagle-cli-fresh-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("lib.rs"), "pub fn queryable() {}\n").unwrap();
        let db_path = base.join("db/graph.db");
        cmd_map(&db_path, &base, false).expect("index the fixture");

        let db = GraphDb::open(&db_path).unwrap();
        let results = db.search_nodes("queryable").unwrap();
        assert!(!results.is_empty(), "the fixture symbol must be indexed");

        // Fresh: the index matches the file. Exit 0, and every row is FRESH.
        let scored = freshness_for_results(&db, &results);
        assert_eq!(
            scored.len(),
            results.len(),
            "result order and count preserved"
        );
        assert!(
            scored
                .iter()
                .all(|r| r.freshness.verdict == Freshness::Fresh),
            "a just-indexed file must be fresh"
        );
        report_freshness(&scored, true).expect("a fresh index answers under --require-fresh");

        // Stale: change the file's content between index and query.
        std::fs::write(
            base.join("lib.rs"),
            "pub fn queryable() { println!(\"moved\"); }\n",
        )
        .unwrap();
        let scored = freshness_for_results(&db, &results);
        assert!(
            scored
                .iter()
                .all(|r| r.freshness.verdict == Freshness::Stale),
            "changed content must read stale"
        );
        // Without --require-fresh the query still answers (the signal is
        // advisory); with it the refusal names the stale exit code 2.
        report_freshness(&scored, false).expect("advisory mode still answers when stale");
        let err =
            report_freshness(&scored, true).expect_err("stale must refuse under --require-fresh");
        assert!(
            err.ends_with(&format!(":{}", EXIT_STALE)),
            "stale refusal must carry exit {}, got: {err}",
            EXIT_STALE
        );

        // The zero-node trap: a cited file whose hash row is absent must read
        // could-not-determine — never stale (nothing compares), never fresh
        // (nothing proven). Delete the row, leaving the nodes intact.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute("DELETE FROM file_hashes", []).unwrap();
        drop(conn);
        let scored = freshness_for_results(&db, &results);
        assert!(
            scored
                .iter()
                .all(|r| r.freshness.verdict == Freshness::CouldNotDetermine),
            "a node without a hash row is unverifiable — not stale"
        );
        let err = report_freshness(&scored, true)
            .expect_err("unverifiable must refuse under --require-fresh");
        assert!(
            err.ends_with(&format!(":{}", EXIT_UNVERIFIABLE)),
            "unverifiable refusal must carry exit {}, got: {err}",
            EXIT_UNVERIFIABLE
        );

        std::fs::remove_dir_all(&base).unwrap();
    }

    /// An unreadable cited file must not read as stale (no comparison ran)
    /// nor as fresh (nothing was proven) — it is could-not-determine, and
    /// `--require-fresh` must refuse it with exit 3.
    #[test]
    fn freshness_of_a_missing_file_is_unknown_not_stale() {
        let base = std::env::temp_dir().join(format!("deagle-cli-missing-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("lib.rs"), "pub fn washere() {}\n").unwrap();
        let db_path = base.join("db/graph.db");
        cmd_map(&db_path, &base, false).expect("index the fixture");

        std::fs::remove_file(base.join("lib.rs")).unwrap();

        let db = GraphDb::open(&db_path).unwrap();
        let results = db.search_nodes("washere").unwrap();
        let scored = freshness_for_results(&db, &results);
        assert!(
            scored
                .iter()
                .all(|r| r.freshness.verdict == Freshness::CouldNotDetermine),
            "a deleted file's freshness is unverifiable, not stale"
        );
        let err = report_freshness(&scored, true)
            .expect_err("a missing file must refuse under --require-fresh");
        assert!(
            err.ends_with(&format!(":{}", EXIT_UNVERIFIABLE)),
            "expected exit {}, got: {err}",
            EXIT_UNVERIFIABLE
        );

        std::fs::remove_dir_all(&base).unwrap();
    }

    /// Relative stored paths resolve against the recorded index root; an
    /// index with no recorded root cannot resolve a relative path, so the
    /// verdict is could-not-determine rather than a hash of the wrong file.
    #[test]
    fn resolve_for_hash_needs_the_root_for_relative_paths() {
        let root = Path::new("/idx/root");
        assert_eq!(
            resolve_for_hash("src/lib.rs", Some(root)),
            Some(PathBuf::from("/idx/root/src/lib.rs"))
        );
        assert_eq!(
            resolve_for_hash("/abs/lib.rs", Some(root)),
            Some(PathBuf::from("/abs/lib.rs")),
            "absolute paths resolve on their own"
        );
        assert_eq!(resolve_for_hash("src/lib.rs", None), None);
    }
}
