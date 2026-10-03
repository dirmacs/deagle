//! deagle-mcp — MCP server for code intelligence.
//!
//! Exposes deagle capabilities as MCP tools for Claude Code, Cursor, etc.
//! Tools: search, stats, map, sg (structural grep), rg (regex grep)
//!
//! Run: `deagle-mcp` (communicates via stdio)

use deagle_core::{GraphDb, Language};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{wrapper::Json as McpJson, wrapper::Parameters},
    model::{Implementation, ServerCapabilities},
    schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

struct DeagleMcp {
    db: Mutex<GraphDb>,
    root_dir: PathBuf,
}

// --- Parameter types ---

#[derive(Deserialize, schemars::JsonSchema)]
struct SearchParams {
    /// Search query (substring match on entity names)
    query: String,
    /// Optional filter by entity kind (function, struct, class, method, etc.)
    kind: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct MapParams {
    /// Directory to index (defaults to root_dir)
    dir: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PatternParams {
    /// Pattern to search for (AST pattern for sg, regex for rg)
    pattern: String,
    /// Directory to search (defaults to root_dir)
    dir: Option<String>,
}

/// `deagle_stats` takes no arguments.
///
/// A dedicated empty struct rather than `serde_json::Value`: MCP requires every
/// tool's `inputSchema` to have a root `type: "object"`, and the schema schemars
/// derives for `serde_json::Value` is `{}`, which rmcp rejects at registration.
#[derive(Deserialize, schemars::JsonSchema)]
struct StatsParams {}

// --- Response types ---

#[derive(Serialize, schemars::JsonSchema)]
struct SearchResult {
    name: String,
    kind: String,
    language: String,
    file_path: String,
    line_start: u32,
}

#[derive(Serialize, schemars::JsonSchema)]
struct SearchOutput {
    results: Vec<SearchResult>,
    count: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct StatsOutput {
    nodes: usize,
    edges: usize,
    db_path: String,
}

#[derive(Serialize, schemars::JsonSchema)]
struct MapOutput {
    files: usize,
    entities: usize,
    edges: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct GrepMatch {
    file: String,
    line: u32,
    /// 1-indexed character column of the match within its line. Absent only
    /// when the producer does not compute one. Both `deagle_rg` and `deagle_sg`
    /// fill this in, with the same unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<u64>,
    /// Byte offset of the match from the start of the file. Filled by
    /// `deagle_rg`; `deagle_sg` omits it because `pattern::PatternMatch`
    /// carries no byte offset — absent there means not carried, not stale.
    #[serde(skip_serializing_if = "Option::is_none")]
    byte_offset: Option<u64>,
    text: String,
}

#[derive(Serialize, schemars::JsonSchema)]
struct GrepOutput {
    matches: Vec<GrepMatch>,
    count: usize,
}

#[tool_router]
impl DeagleMcp {
    fn new(db: GraphDb, root_dir: PathBuf) -> Self {
        Self {
            db: Mutex::new(db),
            root_dir,
        }
    }

    #[tool(
        name = "deagle_search",
        description = "Search for code entities (functions, structs, classes, methods, traits, imports) by name in the indexed codebase. Returns matching entities with file locations."
    )]
    fn search(&self, Parameters(params): Parameters<SearchParams>) -> McpJson<SearchOutput> {
        let db = self.db.lock().unwrap();
        let results = match db.search_nodes(&params.query) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Search error: {}", e);
                return McpJson(SearchOutput {
                    results: vec![],
                    count: 0,
                });
            }
        };

        let filtered: Vec<SearchResult> = results
            .into_iter()
            .filter(|n| {
                params
                    .kind
                    .as_ref()
                    .is_none_or(|k| n.kind.to_string() == *k)
            })
            .map(|n| SearchResult {
                name: n.name,
                kind: n.kind.to_string(),
                language: n.language.to_string(),
                file_path: n.file_path,
                line_start: n.line_start,
            })
            .collect();

        let count = filtered.len();
        McpJson(SearchOutput {
            results: filtered,
            count,
        })
    }

    #[tool(
        name = "deagle_keyword",
        description = "Full-text keyword search using FTS5 BM25 ranking. Searches entity names and source content. Returns results ranked by relevance."
    )]
    fn keyword(&self, Parameters(params): Parameters<SearchParams>) -> McpJson<SearchOutput> {
        let db = self.db.lock().unwrap();
        let results = match db.keyword_search(&params.query) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Keyword search error: {}", e);
                return McpJson(SearchOutput {
                    results: vec![],
                    count: 0,
                });
            }
        };

        let filtered: Vec<SearchResult> = results
            .into_iter()
            .filter(|n| {
                params
                    .kind
                    .as_ref()
                    .is_none_or(|k| n.kind.to_string() == *k)
            })
            .map(|n| SearchResult {
                name: n.name,
                kind: n.kind.to_string(),
                language: n.language.to_string(),
                file_path: n.file_path,
                line_start: n.line_start,
            })
            .collect();

        let count = filtered.len();
        McpJson(SearchOutput {
            results: filtered,
            count,
        })
    }

    #[tool(
        name = "deagle_stats",
        description = "Show graph database statistics — total nodes (code entities) and edges (relationships) in the index."
    )]
    fn stats(&self, Parameters(_): Parameters<StatsParams>) -> McpJson<StatsOutput> {
        let db = self.db.lock().unwrap();
        let nodes = db.node_count().unwrap_or(0);
        let edges = db.edge_count().unwrap_or(0);
        let db_path = db
            .path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "in-memory".to_string());
        McpJson(StatsOutput {
            nodes,
            edges,
            db_path,
        })
    }

    #[tool(
        name = "deagle_map",
        description = "Index a codebase directory into the graph database. Parses source files (Rust, Python) and extracts entities and relationships. Replaces any existing index."
    )]
    fn map(&self, Parameters(params): Parameters<MapParams>) -> McpJson<MapOutput> {
        let dir = params
            .dir
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root_dir.clone());

        let db = self.db.lock().unwrap();
        let _ = db.clear();

        let files: Vec<_> = ignore::WalkBuilder::new(&dir)
            .hidden(true)
            .git_ignore(true)
            .build()
            .flatten()
            .filter(|e| e.path().is_file())
            .filter(|e| {
                let ext = e.path().extension().and_then(|x| x.to_str()).unwrap_or("");
                Language::from_extension(ext) != Language::Unknown
            })
            .collect();

        let mut file_count = 0usize;
        let mut node_count = 0usize;
        let mut edge_count = 0usize;

        for entry in &files {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let lang = Language::from_extension(ext);
            let content = match std::fs::read_to_string(path) {
                Ok(c) if !c.is_empty() => c,
                _ => continue,
            };
            let rel = path.strip_prefix(&dir).unwrap_or(path);
            if let Ok(result) = deagle_parse::parse_file_with_edges(rel, &content, lang) {
                if result.nodes.is_empty() {
                    continue;
                }
                file_count += 1;
                let mut db_ids = Vec::new();
                for node in &result.nodes {
                    match db.insert_node(node) {
                        Ok(id) => db_ids.push(id),
                        Err(_) => db_ids.push(-1),
                    }
                }
                node_count += result.nodes.len();
                for &(from_idx, to_idx, ref kind) in &result.edges {
                    if from_idx < db_ids.len() && to_idx < db_ids.len() {
                        let from_id = db_ids[from_idx];
                        let to_id = db_ids[to_idx];
                        if from_id > 0 && to_id > 0 {
                            let _ = db.insert_edge(&deagle_core::Edge {
                                from_id,
                                to_id,
                                kind: *kind,
                                confidence: 1.0,
                            });
                            edge_count += 1;
                        }
                    }
                }
            }
        }

        McpJson(MapOutput {
            files: file_count,
            entities: node_count,
            edges: edge_count,
        })
    }

    #[tool(
        name = "deagle_sg",
        description = "Structural AST pattern search (powered by ast-grep). Find code matching structural patterns like '$X.unwrap()', 'fn $NAME() { $$$ }', 'struct $S { $$$FIELDS }'. Returns file locations and matched text."
    )]
    fn sg(&self, Parameters(params): Parameters<PatternParams>) -> McpJson<GrepOutput> {
        let dir = params
            .dir
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root_dir.clone());
        let mut matches = Vec::new();

        let walker = ignore::WalkBuilder::new(&dir)
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
            let content = match std::fs::read_to_string(path) {
                Ok(c) if !c.is_empty() => c,
                _ => continue,
            };
            let rel = path.strip_prefix(&dir).unwrap_or(path);
            if let Ok(ms) =
                deagle_parse::pattern::search_pattern(rel, &content, &params.pattern, lang)
            {
                for m in ms {
                    matches.push(GrepMatch {
                        file: m.file_path,
                        line: m.line_start,
                        column: Some(m.col_start as u64),
                        byte_offset: None,
                        text: m.text.lines().next().unwrap_or("").to_string(),
                    });
                }
            }
        }

        let count = matches.len();
        McpJson(GrepOutput { matches, count })
    }

    #[tool(
        name = "deagle_rg",
        description = "Fast regex text search across source files (powered by ripgrep). Searches file contents for regex patterns. Returns matching lines with file locations."
    )]
    fn rg(&self, Parameters(params): Parameters<PatternParams>) -> McpJson<GrepOutput> {
        let dir = params
            .dir
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root_dir.clone());

        let results = match deagle_parse::text_search::search_directory(&dir, &params.pattern, None)
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Search error: {}", e);
                return McpJson(GrepOutput {
                    matches: vec![],
                    count: 0,
                });
            }
        };

        let matches: Vec<GrepMatch> = results
            .into_iter()
            .map(|m| GrepMatch {
                file: m.file_path,
                line: m.line_number as u32,
                column: Some(m.column),
                byte_offset: Some(m.byte_offset),
                text: m.line,
            })
            .collect();

        let count = matches.len();
        McpJson(GrepOutput { matches, count })
    }
}

#[tool_handler]
impl ServerHandler for DeagleMcp {
    fn get_info(&self) -> rmcp::model::InitializeResult {
        rmcp::model::InitializeResult::new(
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(Implementation::new("deagle", env!("CARGO_PKG_VERSION")))
        .with_instructions("Deagle code intelligence — search, map, and analyze codebases. Use deagle_map first to index, then deagle_search/deagle_sg/deagle_rg to query.")
    }
}

#[tokio::main]
async fn main() {
    // MCP servers MUST NOT write to stdout — only stderr
    eprintln!("deagle-mcp v{} starting...", env!("CARGO_PKG_VERSION"));

    let db_path = std::env::var("DEAGLE_DB").unwrap_or_else(|_| ".deagle/graph.db".to_string());
    let root_dir = std::env::var("DEAGLE_ROOT").unwrap_or_else(|_| ".".to_string());

    let db_dir = Path::new(&db_path).parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(db_dir).ok();

    let db = GraphDb::open(Path::new(&db_path)).expect("Failed to open graph database");

    let server = DeagleMcp::new(db, PathBuf::from(root_dir));

    let transport = rmcp::transport::io::stdio();
    let _server = server.serve(transport).await.expect("MCP server failed");

    eprintln!("deagle-mcp shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `deagle_rg` tool must report where each match landed: a 1-indexed
    /// character column, and a byte offset that indexes the file.
    #[test]
    fn rg_reports_column_and_byte_offset() {
        let dir = std::env::temp_dir().join(format!("deagle-rg-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "// ééé // TODO\n").unwrap();

        let mcp = DeagleMcp::new(GraphDb::in_memory().unwrap(), dir.clone());
        let out = mcp.rg(Parameters(PatternParams {
            pattern: "TODO".to_string(),
            dir: Some(dir.to_string_lossy().into_owned()),
        }));
        let on_disk = std::fs::read(dir.join("a.rs")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(out.0.count, 1);
        let m = &out.0.matches[0];
        // "é" is two bytes, so the character column (11) and the byte offset
        // (13) disagree -- that is the distinction under test.
        assert_eq!(m.column, Some(11));
        assert_eq!(m.byte_offset, Some(13));
        assert!(
            on_disk[13..].starts_with(b"TODO"),
            "byte_offset must index the file at the match"
        );
    }

    /// Structural search now reports the same 1-indexed character column as
    /// `rg`, and still omits `byte_offset`, which it genuinely does not carry.
    #[test]
    fn sg_reports_column_and_omits_byte_offset() {
        let dir = std::env::temp_dir().join(format!("deagle-sg-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.rs"),
            "fn main() { let x = Some(1).unwrap(); }\n",
        )
        .unwrap();

        let mcp = DeagleMcp::new(GraphDb::in_memory().unwrap(), dir.clone());
        let out = mcp.sg(Parameters(PatternParams {
            pattern: "$X.unwrap()".to_string(),
            dir: Some(dir.to_string_lossy().into_owned()),
        }));
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(out.0.count > 0, "fixture should match the AST pattern");
        let m = &out.0.matches[0];
        let json = serde_json::to_value(m).unwrap();

        // The match node is `Some(1).unwrap()`; derive the expected 1-indexed
        // character column from the fixture rather than hand-counting it.
        let line = "fn main() { let x = Some(1).unwrap(); }";
        let expected = line.find("Some(1)").unwrap() as u64 + 1;
        assert_eq!(m.column, Some(expected), "sg must report a measured column");
        assert_eq!(json["column"], expected);
        assert!(
            json.get("byte_offset").is_none(),
            "pattern::PatternMatch carries no byte offset; omitting is honest"
        );
    }
}
