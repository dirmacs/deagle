//! deagle-core — graph types and SQLite storage for code intelligence.
//!
//! Defines the code graph model: nodes (functions, classes, modules),
//! edges (calls, imports, contains, inherits), and SQLite-backed persistence.
//!
//! ## Feature Flags
//!
//! - `semantic` — enables semantic code search via [ares-vector](https://crates.io/crates/ares-vector)

#[cfg(feature = "semantic")]
pub mod semantic;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A node in the code graph — represents a code entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Node {
    /// Unique identifier (auto-generated)
    pub id: i64,
    /// Entity name (function name, class name, etc.)
    pub name: String,
    /// Entity kind
    pub kind: NodeKind,
    /// Programming language
    pub language: Language,
    /// Source file path (relative to repo root)
    pub file_path: String,
    /// Start line number (1-indexed)
    pub line_start: u32,
    /// End line number (1-indexed)
    pub line_end: u32,
    /// Optional source code excerpt
    pub content: Option<String>,
}

/// Kind of code entity.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    File,
    Module,
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Constant,
    Variable,
    TypeAlias,
    Import,
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::File => "file",
            Self::Module => "module",
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Interface => "interface",
            Self::Constant => "constant",
            Self::Variable => "variable",
            Self::TypeAlias => "type_alias",
            Self::Import => "import",
        };
        write!(f, "{}", s)
    }
}

/// Supported programming languages.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    Python,
    Go,
    TypeScript,
    JavaScript,
    Java,
    Cpp,
    C,
    Ruby,
    Unknown,
}

impl Language {
    /// Detect language from file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "rs" => Self::Rust,
            "py" => Self::Python,
            "go" => Self::Go,
            "ts" | "tsx" => Self::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "java" => Self::Java,
            "cpp" | "cc" | "cxx" | "hpp" => Self::Cpp,
            "c" | "h" => Self::C,
            "rb" | "rake" | "gemspec" => Self::Ruby,
            _ => Self::Unknown,
        }
    }

    /// File extensions for this language.
    pub fn extensions(&self) -> &[&str] {
        match self {
            Self::Rust => &["rs"],
            Self::Python => &["py"],
            Self::Go => &["go"],
            Self::TypeScript => &["ts", "tsx"],
            Self::JavaScript => &["js", "jsx", "mjs", "cjs"],
            Self::Java => &["java"],
            Self::Cpp => &["cpp", "cc", "cxx", "hpp"],
            Self::C => &["c", "h"],
            Self::Ruby => &["rb", "rake", "gemspec"],
            Self::Unknown => &[],
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Go => "go",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Java => "java",
            Self::Cpp => "cpp",
            Self::C => "c",
            Self::Ruby => "ruby",
            Self::Unknown => "unknown",
        };
        write!(f, "{}", s)
    }
}

/// An edge in the code graph — represents a relationship between entities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Edge {
    /// Source node ID
    pub from_id: i64,
    /// Target node ID
    pub to_id: i64,
    /// Relationship type
    pub kind: EdgeKind,
    /// Confidence score (0.0–1.0) for inferred edges
    pub confidence: f32,
}

/// Kind of relationship between code entities.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Function/method call
    Calls,
    /// Import/use statement
    Imports,
    /// Parent contains child (file→function, class→method)
    Contains,
    /// Class/struct inheritance
    Inherits,
    /// Interface/trait implementation
    Implements,
    /// Type reference (parameter type, return type, field type)
    References,
    /// Module/package dependency
    DependsOn,
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Calls => "calls",
            Self::Imports => "imports",
            Self::Contains => "contains",
            Self::Inherits => "inherits",
            Self::Implements => "implements",
            Self::References => "references",
            Self::DependsOn => "depends_on",
        };
        write!(f, "{}", s)
    }
}

/// Errors from deagle operations.
#[derive(Debug, thiserror::Error)]
pub enum DeagleError {
    #[cfg(feature = "sqlite")]
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Parse error in {file}: {message}")]
    Parse { file: String, message: String },
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, DeagleError>;

#[cfg(feature = "sqlite")]
/// SQLite-backed code graph database.
pub struct GraphDb {
    conn: rusqlite::Connection,
}

#[cfg(feature = "sqlite")]
impl GraphDb {
    /// Open or create a graph database at the given path.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        // WAL mode: concurrent reads during writes, faster for indexing workloads
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // Synchronous NORMAL: safe with WAL, 2-3x faster than FULL
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Create an in-memory graph database (for testing).
    pub fn in_memory() -> Result<Self> {
        let conn = rusqlite::Connection::open_in_memory()?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS nodes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                language TEXT NOT NULL,
                file_path TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                content TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_nodes_name ON nodes(name);
            CREATE INDEX IF NOT EXISTS idx_nodes_kind ON nodes(kind);
            CREATE INDEX IF NOT EXISTS idx_nodes_file ON nodes(file_path);

            CREATE TABLE IF NOT EXISTS edges (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_id INTEGER NOT NULL REFERENCES nodes(id),
                to_id INTEGER NOT NULL REFERENCES nodes(id),
                kind TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 1.0
            );
            CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id);
            CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_id);
            CREATE INDEX IF NOT EXISTS idx_edges_kind ON edges(kind);

            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS file_hashes (
                file_path TEXT PRIMARY KEY,
                content_hash TEXT NOT NULL,
                indexed_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS nodes_fts USING fts5(
                name, content, file_path,
                content='nodes',
                content_rowid='id'
            );
            "
        )?;
        Ok(())
    }

    /// Insert a node and return its ID.
    pub fn insert_node(&self, node: &Node) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO nodes (name, kind, language, file_path, line_start, line_end, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                node.name,
                node.kind.to_string(),
                node.language.to_string(),
                node.file_path,
                node.line_start,
                node.line_end,
                node.content,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        // Populate FTS5 index
        self.conn.execute(
            "INSERT INTO nodes_fts(rowid, name, content, file_path) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, node.name, node.content, node.file_path],
        )?;
        Ok(id)
    }

    /// Batch insert nodes and edges in a single transaction (much faster for indexing).
    pub fn insert_batch(&self, nodes: &[Node], edges: &[(i64, i64, EdgeKind)]) -> Result<Vec<i64>> {
        let tx = self.conn.unchecked_transaction()?;
        let mut ids = Vec::with_capacity(nodes.len());

        {
            let mut node_stmt = tx.prepare_cached(
                "INSERT INTO nodes (name, kind, language, file_path, line_start, line_end, content)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
            )?;
            let mut fts_stmt = tx.prepare_cached(
                "INSERT INTO nodes_fts(rowid, name, content, file_path) VALUES (?1, ?2, ?3, ?4)"
            )?;

            for node in nodes {
                node_stmt.execute(rusqlite::params![
                    node.name, node.kind.to_string(), node.language.to_string(),
                    node.file_path, node.line_start, node.line_end, node.content,
                ])?;
                let id = tx.last_insert_rowid();
                fts_stmt.execute(rusqlite::params![id, node.name, node.content, node.file_path])?;
                ids.push(id);
            }
        }

        {
            let mut edge_stmt = tx.prepare_cached(
                "INSERT INTO edges (from_id, to_id, kind, confidence) VALUES (?1, ?2, ?3, ?4)"
            )?;
            for (from_id, to_id, kind) in edges {
                edge_stmt.execute(rusqlite::params![from_id, to_id, kind.to_string(), 1.0])?;
            }
        }

        tx.commit()?;
        Ok(ids)
    }

    /// Full-text keyword search using FTS5 BM25 ranking.
    pub fn keyword_search(&self, query: &str) -> Result<Vec<Node>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.name, n.kind, n.language, n.file_path, n.line_start, n.line_end, n.content
             FROM nodes_fts f
             JOIN nodes n ON n.id = f.rowid
             WHERE nodes_fts MATCH ?1
             ORDER BY rank
             LIMIT 50"
        )?;
        let rows = stmt.query_map([query], |row| {
            Ok(Node {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(2)?))
                    .unwrap_or(NodeKind::Function),
                language: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(3)?))
                    .unwrap_or(Language::Unknown),
                file_path: row.get(4)?,
                line_start: row.get(5)?,
                line_end: row.get(6)?,
                content: row.get(7)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(DeagleError::from)
    }

    /// Insert an edge.
    pub fn insert_edge(&self, edge: &Edge) -> Result<()> {
        self.conn.execute(
            "INSERT INTO edges (from_id, to_id, kind, confidence) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![edge.from_id, edge.to_id, edge.kind.to_string(), edge.confidence],
        )?;
        Ok(())
    }

    /// Search nodes by name (case-insensitive substring match).
    pub fn search_nodes(&self, query: &str) -> Result<Vec<Node>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, kind, language, file_path, line_start, line_end, content
             FROM nodes WHERE name LIKE ?1 ORDER BY name"
        )?;
        let pattern = format!("%{}%", query);
        let rows = stmt.query_map([&pattern], |row| {
            Ok(Node {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(2)?))
                    .unwrap_or(NodeKind::Function),
                language: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(3)?))
                    .unwrap_or(Language::Unknown),
                file_path: row.get(4)?,
                line_start: row.get(5)?,
                line_end: row.get(6)?,
                content: row.get(7)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(DeagleError::from)
    }

    /// Fuzzy search nodes by name — ranked by match score (best first).
    pub fn fuzzy_search_nodes(&self, query: &str) -> Result<Vec<Node>> {
        use fuzzy_matcher::skim::SkimMatcherV2;
        use fuzzy_matcher::FuzzyMatcher;

        let matcher = SkimMatcherV2::default();

        // Get all nodes and score them
        let mut stmt = self.conn.prepare(
            "SELECT id, name, kind, language, file_path, line_start, line_end, content FROM nodes"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Node {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(2)?))
                    .unwrap_or(NodeKind::Function),
                language: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(3)?))
                    .unwrap_or(Language::Unknown),
                file_path: row.get(4)?,
                line_start: row.get(5)?,
                line_end: row.get(6)?,
                content: row.get(7)?,
            })
        })?;

        let all_nodes: Vec<Node> = rows.collect::<std::result::Result<Vec<_>, _>>()?;

        let mut scored: Vec<(i64, Node)> = all_nodes
            .into_iter()
            .filter_map(|node| {
                matcher.fuzzy_match(&node.name, query).map(|score| (score, node))
            })
            .collect();

        // Sort by score descending (best matches first)
        scored.sort_by(|a, b| b.0.cmp(&a.0));

        Ok(scored.into_iter().map(|(_, node)| node).collect())
    }

    /// Get all edges from a node (outgoing relationships).
    pub fn edges_from(&self, node_id: i64) -> Result<Vec<Edge>> {
        let mut stmt = self.conn.prepare(
            "SELECT from_id, to_id, kind, confidence FROM edges WHERE from_id = ?1"
        )?;
        let rows = stmt.query_map([node_id], |row| {
            Ok(Edge {
                from_id: row.get(0)?,
                to_id: row.get(1)?,
                kind: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(2)?))
                    .unwrap_or(EdgeKind::Calls),
                confidence: row.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(DeagleError::from)
    }

    /// Get total node count.
    pub fn node_count(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
        Ok(count as usize)
    }

    /// Get total edge count.
    pub fn edge_count(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
        Ok(count as usize)
    }

    /// Clear all data (for re-indexing).
    pub fn clear(&self) -> Result<()> {
        self.conn.execute_batch("DELETE FROM edges; DELETE FROM nodes_fts; DELETE FROM nodes; DELETE FROM file_hashes;")?;
        Ok(())
    }

    /// Get the database file path.
    pub fn path(&self) -> Option<PathBuf> {
        self.conn.path().map(PathBuf::from)
    }

    /// Compute SHA-256 hash of content (first 16 hex chars).
    pub fn content_hash(content: &str) -> String {
        use sha2::{Sha256, Digest};
        let hash = Sha256::digest(content.as_bytes());
        hash.iter().take(8).map(|b| format!("{:02x}", b)).collect()
    }

    /// Check if a file needs re-indexing (hash changed or new file).
    pub fn needs_reindex(&self, file_path: &str, content: &str) -> Result<bool> {
        let current_hash = Self::content_hash(content);
        let stored: Option<String> = self.conn.query_row(
            "SELECT content_hash FROM file_hashes WHERE file_path = ?1",
            [file_path],
            |row| row.get(0),
        ).ok();

        Ok(stored.as_deref() != Some(&current_hash))
    }

    /// Store file hash after indexing.
    pub fn store_file_hash(&self, file_path: &str, content: &str) -> Result<()> {
        let hash = Self::content_hash(content);
        self.conn.execute(
            "INSERT OR REPLACE INTO file_hashes (file_path, content_hash) VALUES (?1, ?2)",
            rusqlite::params![file_path, hash],
        )?;
        Ok(())
    }

    /// Remove nodes and edges for a specific file (for re-indexing).
    pub fn remove_file(&self, file_path: &str) -> Result<()> {
        // Get node IDs for this file
        let mut stmt = self.conn.prepare("SELECT id FROM nodes WHERE file_path = ?1")?;
        let ids: Vec<i64> = stmt.query_map([file_path], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        // Delete edges referencing these nodes
        for id in &ids {
            self.conn.execute("DELETE FROM edges WHERE from_id = ?1 OR to_id = ?1", [id])?;
        }
        // Delete nodes
        self.conn.execute("DELETE FROM nodes WHERE file_path = ?1", [file_path])?;
        // Delete hash
        self.conn.execute("DELETE FROM file_hashes WHERE file_path = ?1", [file_path])?;
        Ok(())
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_extension() {
        assert_eq!(Language::from_extension("rs"), Language::Rust);
        assert_eq!(Language::from_extension("py"), Language::Python);
        assert_eq!(Language::from_extension("go"), Language::Go);
        assert_eq!(Language::from_extension("ts"), Language::TypeScript);
        assert_eq!(Language::from_extension("tsx"), Language::TypeScript);
        assert_eq!(Language::from_extension("js"), Language::JavaScript);
        assert_eq!(Language::from_extension("java"), Language::Java);
        assert_eq!(Language::from_extension("cpp"), Language::Cpp);
        assert_eq!(Language::from_extension("c"), Language::C);
        assert_eq!(Language::from_extension("xyz"), Language::Unknown);
    }

    #[test]
    fn test_language_display() {
        assert_eq!(Language::Rust.to_string(), "rust");
        assert_eq!(Language::Python.to_string(), "python");
    }

    #[test]
    fn test_node_kind_display() {
        assert_eq!(NodeKind::Function.to_string(), "function");
        assert_eq!(NodeKind::Struct.to_string(), "struct");
        assert_eq!(NodeKind::TypeAlias.to_string(), "type_alias");
    }

    #[test]
    fn test_edge_kind_display() {
        assert_eq!(EdgeKind::Calls.to_string(), "calls");
        assert_eq!(EdgeKind::Imports.to_string(), "imports");
        assert_eq!(EdgeKind::Contains.to_string(), "contains");
    }

    #[test]
    fn test_graph_db_in_memory() {
        let db = GraphDb::in_memory().unwrap();
        assert_eq!(db.node_count().unwrap(), 0);
        assert_eq!(db.edge_count().unwrap(), 0);
    }

    #[test]
    fn test_insert_and_search_node() {
        let db = GraphDb::in_memory().unwrap();
        let node = Node {
            id: 0,
            name: "process_request".to_string(),
            kind: NodeKind::Function,
            language: Language::Rust,
            file_path: "src/handler.rs".to_string(),
            line_start: 42,
            line_end: 68,
            content: Some("pub fn process_request() {}".to_string()),
        };
        let id = db.insert_node(&node).unwrap();
        assert!(id > 0);
        assert_eq!(db.node_count().unwrap(), 1);

        let results = db.search_nodes("process").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "process_request");
        assert_eq!(results[0].kind, NodeKind::Function);
        assert_eq!(results[0].language, Language::Rust);
    }

    #[test]
    fn test_insert_edge_and_query() {
        let db = GraphDb::in_memory().unwrap();
        let n1 = Node {
            id: 0, name: "main".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "src/main.rs".into(),
            line_start: 1, line_end: 10, content: None,
        };
        let n2 = Node {
            id: 0, name: "handler".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "src/lib.rs".into(),
            line_start: 5, line_end: 20, content: None,
        };
        let id1 = db.insert_node(&n1).unwrap();
        let id2 = db.insert_node(&n2).unwrap();

        let edge = Edge {
            from_id: id1, to_id: id2,
            kind: EdgeKind::Calls, confidence: 1.0,
        };
        db.insert_edge(&edge).unwrap();
        assert_eq!(db.edge_count().unwrap(), 1);

        let edges = db.edges_from(id1).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_id, id2);
        assert_eq!(edges[0].kind, EdgeKind::Calls);
    }

    #[test]
    fn test_search_case_insensitive() {
        let db = GraphDb::in_memory().unwrap();
        let node = Node {
            id: 0, name: "MyStruct".into(), kind: NodeKind::Struct,
            language: Language::Rust, file_path: "src/types.rs".into(),
            line_start: 1, line_end: 5, content: None,
        };
        db.insert_node(&node).unwrap();

        let results = db.search_nodes("mystruct").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_clear_db() {
        let db = GraphDb::in_memory().unwrap();
        let node = Node {
            id: 0, name: "test".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "t.rs".into(),
            line_start: 1, line_end: 1, content: None,
        };
        db.insert_node(&node).unwrap();
        assert_eq!(db.node_count().unwrap(), 1);
        db.clear().unwrap();
        assert_eq!(db.node_count().unwrap(), 0);
    }

    #[test]
    fn test_node_serialization() {
        let node = Node {
            id: 1, name: "test_fn".into(), kind: NodeKind::Function,
            language: Language::Python, file_path: "app.py".into(),
            line_start: 10, line_end: 25, content: Some("def test_fn(): pass".into()),
        };
        let json = serde_json::to_string(&node).unwrap();
        let parsed: Node = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test_fn");
        assert_eq!(parsed.kind, NodeKind::Function);
        assert_eq!(parsed.language, Language::Python);
    }

    #[test]
    fn test_language_extensions() {
        assert!(Language::Rust.extensions().contains(&"rs"));
        assert!(Language::TypeScript.extensions().contains(&"tsx"));
        assert!(Language::Unknown.extensions().is_empty());
    }

    #[test]
    fn test_multiple_nodes_same_name() {
        let db = GraphDb::in_memory().unwrap();
        for file in &["a.rs", "b.rs", "c.rs"] {
            db.insert_node(&Node {
                id: 0, name: "new".into(), kind: NodeKind::Method,
                language: Language::Rust, file_path: file.to_string(),
                line_start: 1, line_end: 5, content: None,
            }).unwrap();
        }
        let results = db.search_nodes("new").unwrap();
        assert_eq!(results.len(), 3, "Should find all 3 nodes named 'new'");
    }

    #[test]
    fn test_search_empty_query() {
        let db = GraphDb::in_memory().unwrap();
        db.insert_node(&Node {
            id: 0, name: "hello".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "t.rs".into(),
            line_start: 1, line_end: 1, content: None,
        }).unwrap();
        // Empty pattern matches everything via LIKE '%%'
        let results = db.search_nodes("").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_edges_from_nonexistent_node() {
        let db = GraphDb::in_memory().unwrap();
        let edges = db.edges_from(999).unwrap();
        assert!(edges.is_empty());
    }

    #[test]
    fn test_multiple_edge_types() {
        let db = GraphDb::in_memory().unwrap();
        let id1 = db.insert_node(&Node {
            id: 0, name: "A".into(), kind: NodeKind::Struct,
            language: Language::Rust, file_path: "a.rs".into(),
            line_start: 1, line_end: 5, content: None,
        }).unwrap();
        let id2 = db.insert_node(&Node {
            id: 0, name: "B".into(), kind: NodeKind::Trait,
            language: Language::Rust, file_path: "b.rs".into(),
            line_start: 1, line_end: 5, content: None,
        }).unwrap();

        db.insert_edge(&Edge { from_id: id1, to_id: id2, kind: EdgeKind::Implements, confidence: 1.0 }).unwrap();
        db.insert_edge(&Edge { from_id: id1, to_id: id2, kind: EdgeKind::References, confidence: 0.8 }).unwrap();

        let edges = db.edges_from(id1).unwrap();
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().any(|e| e.kind == EdgeKind::Implements));
        assert!(edges.iter().any(|e| e.kind == EdgeKind::References));
    }

    #[test]
    fn test_edge_confidence_stored() {
        let db = GraphDb::in_memory().unwrap();
        let id1 = db.insert_node(&Node {
            id: 0, name: "x".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "x.rs".into(),
            line_start: 1, line_end: 1, content: None,
        }).unwrap();
        let id2 = db.insert_node(&Node {
            id: 0, name: "y".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "y.rs".into(),
            line_start: 1, line_end: 1, content: None,
        }).unwrap();

        db.insert_edge(&Edge { from_id: id1, to_id: id2, kind: EdgeKind::Calls, confidence: 0.75 }).unwrap();
        let edges = db.edges_from(id1).unwrap();
        assert!((edges[0].confidence - 0.75).abs() < 0.01);
    }

    #[test]
    #[test]
    fn test_fuzzy_search_basic() {
        let db = GraphDb::in_memory().unwrap();
        for name in &["process_request", "handle_response", "parse_input", "validate_data"] {
            db.insert_node(&Node {
                id: 0, name: name.to_string(), kind: NodeKind::Function,
                language: Language::Rust, file_path: "lib.rs".into(),
                line_start: 1, line_end: 5, content: None,
            }).unwrap();
        }

        let results = db.fuzzy_search_nodes("proc").unwrap();
        assert!(!results.is_empty(), "fuzzy search should find matches");
        assert_eq!(results[0].name, "process_request", "best match should be first");
    }

    #[test]
    fn test_fuzzy_search_typo_tolerance() {
        let db = GraphDb::in_memory().unwrap();
        db.insert_node(&Node {
            id: 0, name: "calculate_total".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "math.rs".into(),
            line_start: 1, line_end: 5, content: None,
        }).unwrap();
        db.insert_node(&Node {
            id: 0, name: "validate_input".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "input.rs".into(),
            line_start: 1, line_end: 5, content: None,
        }).unwrap();

        // "calctot" should fuzzy-match "calculate_total"
        let results = db.fuzzy_search_nodes("calctot").unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "calculate_total");
    }

    #[test]
    fn test_fuzzy_search_no_match() {
        let db = GraphDb::in_memory().unwrap();
        db.insert_node(&Node {
            id: 0, name: "hello".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "t.rs".into(),
            line_start: 1, line_end: 1, content: None,
        }).unwrap();

        let results = db.fuzzy_search_nodes("zzzzz").unwrap();
        assert!(results.is_empty(), "no fuzzy match for gibberish");
    }

    #[test]
    fn test_fuzzy_search_empty_db() {
        let db = GraphDb::in_memory().unwrap();
        let results = db.fuzzy_search_nodes("anything").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_keyword_search() {
        let db = GraphDb::in_memory().unwrap();
        db.insert_node(&Node {
            id: 0, name: "process_data".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "data.rs".into(),
            line_start: 1, line_end: 10,
            content: Some("pub fn process_data(input: Vec<String>) -> Result<()>".into()),
        }).unwrap();
        db.insert_node(&Node {
            id: 0, name: "validate".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "val.rs".into(),
            line_start: 1, line_end: 5, content: Some("fn validate(s: &str) -> bool".into()),
        }).unwrap();

        let results = db.keyword_search("process").unwrap();
        assert!(!results.is_empty(), "FTS5 should find 'process'");
        assert_eq!(results[0].name, "process_data");
    }

    #[test]
    fn test_keyword_search_content() {
        let db = GraphDb::in_memory().unwrap();
        db.insert_node(&Node {
            id: 0, name: "handler".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "web.rs".into(),
            line_start: 1, line_end: 20,
            content: Some("async fn handler(req: Request) -> Response { authenticate(req) }".into()),
        }).unwrap();

        // Search by content, not name
        let results = db.keyword_search("authenticate").unwrap();
        assert!(!results.is_empty(), "FTS5 should search content too");
    }

    #[test]
    fn test_keyword_search_empty() {
        let db = GraphDb::in_memory().unwrap();
        db.insert_node(&Node {
            id: 0, name: "hello".into(), kind: NodeKind::Function,
            language: Language::Rust, file_path: "h.rs".into(),
            line_start: 1, line_end: 1, content: None,
        }).unwrap();

        let results = db.keyword_search("nonexistent_xyz").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_incremental_hash() {
        let db = GraphDb::in_memory().unwrap();
        let content = "fn main() {}";
        assert!(db.needs_reindex("test.rs", content).unwrap(), "new file needs indexing");
        db.store_file_hash("test.rs", content).unwrap();
        assert!(!db.needs_reindex("test.rs", content).unwrap(), "same content skipped");
        assert!(db.needs_reindex("test.rs", "fn main() { println!() }").unwrap(), "changed content needs re-index");
    }

    #[test]
    fn test_node_with_none_content() {
        let node = Node {
            id: 0, name: "no_content".into(), kind: NodeKind::Function,
            language: Language::Go, file_path: "main.go".into(),
            line_start: 1, line_end: 10, content: None,
        };
        let json = serde_json::to_string(&node).unwrap();
        let parsed: Node = serde_json::from_str(&json).unwrap();
        assert!(parsed.content.is_none());
        assert_eq!(parsed.language, Language::Go);
    }
}
