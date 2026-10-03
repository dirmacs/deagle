//! deagle-server — HTTP API for deagle code intelligence.
//!
//! Exposes the same capabilities as the CLI via REST endpoints:
//! - POST /api/map — index a directory
//! - GET  /api/search?q=name&kind=struct — search entities
//! - POST /api/sg — structural pattern search
//! - POST /api/rg — regex text search
//! - GET  /api/stats — graph statistics
//! - GET  /health — health check

use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
};
use deagle_core::GraphDb;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;

struct AppState {
    db: Mutex<GraphDb>,
    root_dir: PathBuf,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("deagle_server=info")
        .init();

    let db_path = std::env::var("DEAGLE_DB").unwrap_or_else(|_| ".deagle/graph.db".to_string());
    let root_dir = std::env::var("DEAGLE_ROOT").unwrap_or_else(|_| ".".to_string());
    let port: u16 = std::env::var("DEAGLE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3500);

    std::fs::create_dir_all(
        std::path::Path::new(&db_path)
            .parent()
            .unwrap_or(std::path::Path::new(".")),
    )
    .ok();

    let db = GraphDb::open(std::path::Path::new(&db_path)).expect("Failed to open graph database");

    let state = Arc::new(AppState {
        db: Mutex::new(db),
        root_dir: PathBuf::from(root_dir),
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/search", get(search))
        .route("/api/stats", get(stats))
        .route("/api/map", post(map))
        .route("/api/sg", post(sg))
        .route("/api/rg", post(rg))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("deagle-server listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    kind: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SearchResponse {
    results: Vec<NodeJson>,
    count: usize,
}

#[derive(Serialize, Deserialize)]
struct NodeJson {
    name: String,
    kind: String,
    language: String,
    file_path: String,
    line_start: u32,
}

async fn search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let results = db
        .search_nodes(&params.q)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let filtered: Vec<NodeJson> = results
        .into_iter()
        .filter(|n| {
            params
                .kind
                .as_ref()
                .is_none_or(|k| n.kind.to_string() == *k)
        })
        .map(|n| NodeJson {
            name: n.name,
            kind: n.kind.to_string(),
            language: n.language.to_string(),
            file_path: n.file_path,
            line_start: n.line_start,
        })
        .collect();

    let count = filtered.len();
    Ok(Json(SearchResponse {
        results: filtered,
        count,
    }))
}

#[derive(Serialize, Deserialize)]
struct StatsResponse {
    nodes: usize,
    edges: usize,
}

async fn stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<StatsResponse>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let nodes = db
        .node_count()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let edges = db
        .edge_count()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(StatsResponse { nodes, edges }))
}

#[derive(Deserialize)]
struct MapRequest {
    dir: Option<String>,
    /// Clear the whole graph before indexing. Defaults to false, matching
    /// `deagle map` (which clears only under `--force`) and `deagle_map` (which
    /// never clears). Without this, incremental indexing would take away the only
    /// way an HTTP caller had to reset.
    force: Option<bool>,
}

#[derive(Serialize)]
struct MapResponse {
    files: usize,
    entities: usize,
    /// Relationships indexed alongside the nodes. Reported because a caller
    /// cannot otherwise tell a graph from a bag of nodes: both count entities,
    /// and this endpoint used to silently index zero edges.
    edges: usize,
}

async fn map(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MapRequest>,
) -> Result<Json<MapResponse>, (StatusCode, String)> {
    let dir = req
        .dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.root_dir.clone());

    let db = state.db.lock().await;
    if req.force.unwrap_or(false) {
        db.clear()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    let mut files = 0usize;
    let mut entities = 0usize;
    let mut edges = 0usize;

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
        let lang = deagle_core::Language::from_extension(ext);
        if lang == deagle_core::Language::Unknown {
            continue;
        }

        let content = std::fs::read_to_string(path).unwrap_or_default();
        if content.is_empty() {
            continue;
        }

        let rel = path.strip_prefix(&dir).unwrap_or(path);
        let rel_str = rel.to_string_lossy().to_string();

        // Skip files whose content is unchanged since the last index. This is
        // what the stored hashes exist for; without them every call re-parses
        // and re-inserts the entire tree.
        match db.needs_reindex(&rel_str, &content) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(e) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
            }
        }

        if let Ok(result) = deagle_parse::parse_file_with_edges(rel, &content, lang) {
            // Replace this file's rows rather than adding alongside them, so a
            // re-index of changed content does not duplicate its nodes and
            // edges. Skipping above and replacing here must both be present:
            // skip without replace leaves stale rows, replace without skip
            // redoes the work.
            let _ = db.remove_file(&rel_str);

            // Nodes first: edges reference nodes by index into the parse
            // result, so they can only be resolved once the nodes have
            // database ids.
            let db_ids = db
                .insert_batch(&result.nodes, &[])
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            for (from_idx, to_idx, kind) in &result.edges {
                match (db_ids.get(*from_idx), db_ids.get(*to_idx)) {
                    (Some(&from_id), Some(&to_id)) if from_id > 0 && to_id > 0 => {
                        let _ = db.insert_edge(&deagle_core::Edge {
                            from_id,
                            to_id,
                            kind: *kind,
                            // Every edge a parser emits is structural
                            // containment, not an inference — same reasoning
                            // as the CLI's `deagle map`.
                            confidence: 1.0,
                        });
                        edges += 1;
                    }
                    // An edge naming a node the parser did not emit is
                    // skipped rather than guessed at, and not counted.
                    _ => {}
                }
            }

            entities += result.nodes.len();
            files += 1;

            let _ = db.store_file_hash(&rel_str, &content);
        }
    }

    Ok(Json(MapResponse {
        files,
        entities,
        edges,
    }))
}

#[derive(Deserialize)]
struct PatternRequest {
    pattern: String,
    dir: Option<String>,
}

#[derive(Serialize)]
struct PatternMatch {
    file: String,
    line: u32,
    /// 1-indexed character column of the match within its line. `None` only
    /// when the producer does not compute one. Both `rg` and `sg` fill this in,
    /// with the same unit — see `deagle_parse`'s `char_column`.
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<u64>,
    /// Byte offset of the match from the start of the file. Filled by `rg`;
    /// `sg` reports `None` because `pattern::PatternMatch` carries no byte
    /// offset — `None` there means absent, not stale.
    #[serde(skip_serializing_if = "Option::is_none")]
    byte_offset: Option<u64>,
    text: String,
}

#[derive(Serialize)]
struct PatternResponse {
    matches: Vec<PatternMatch>,
    count: usize,
}

async fn sg(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PatternRequest>,
) -> Result<Json<PatternResponse>, (StatusCode, String)> {
    let dir = req
        .dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.root_dir.clone());
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
        let lang = deagle_core::Language::from_extension(ext);
        if lang == deagle_core::Language::Unknown {
            continue;
        }

        let content = std::fs::read_to_string(path).unwrap_or_default();
        if content.is_empty() {
            continue;
        }

        let rel = path.strip_prefix(&dir).unwrap_or(path);
        if let Ok(ms) = deagle_parse::pattern::search_pattern(rel, &content, &req.pattern, lang) {
            for m in ms {
                matches.push(PatternMatch {
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
    Ok(Json(PatternResponse { matches, count }))
}

async fn rg(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PatternRequest>,
) -> Result<Json<PatternResponse>, (StatusCode, String)> {
    let dir = req
        .dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.root_dir.clone());

    let results = deagle_parse::text_search::search_directory(&dir, &req.pattern, None)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let matches: Vec<PatternMatch> = results
        .into_iter()
        .map(|m| PatternMatch {
            file: m.file_path,
            line: m.line_number as u32,
            column: Some(m.column),
            byte_offset: Some(m.byte_offset),
            text: m.line,
        })
        .collect();

    let count = matches.len();
    Ok(Json(PatternResponse { matches, count }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_app() -> Router {
        let db = GraphDb::in_memory().unwrap();
        let state = Arc::new(AppState {
            db: Mutex::new(db),
            root_dir: PathBuf::from("."),
        });
        Router::new()
            .route("/health", get(health))
            .route("/api/search", get(search))
            .route("/api/stats", get(stats))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_health() {
        let app = test_app();
        let resp = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_stats_empty() {
        let app = test_app();
        let resp = app
            .oneshot(Request::get("/api/stats").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let stats: StatsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(stats.nodes, 0);
        assert_eq!(stats.edges, 0);
    }

    #[tokio::test]
    async fn test_search_empty_db() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::get("/api/search?q=test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let sr: SearchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(sr.count, 0);
    }

    /// `POST /api/map` must index incrementally. It used to `clear()` the whole
    /// graph and re-insert every file on every call, and it never stored file
    /// hashes, so `needs_reindex` had nothing to compare against.
    ///
    /// Asserts against `/api/stats` after each call, not against the response
    /// alone: the dangerous variant is a skip added *without* the per-file
    /// replace, where a changed file is inserted alongside its old rows and the
    /// response still looks right.
    #[tokio::test]
    async fn test_map_indexes_incrementally() {
        let dir = std::env::temp_dir().join(format!("deagle-map-incr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("lib.rs");
        std::fs::write(
            &file,
            "struct Config { name: String }\n\nfn helper() -> i32 { 42 }\n",
        )
        .unwrap();

        let db = GraphDb::in_memory().unwrap();
        let state = Arc::new(AppState {
            db: Mutex::new(db),
            root_dir: dir.clone(),
        });
        let app = Router::new()
            .route("/api/map", post(map))
            .route("/api/stats", get(stats))
            .with_state(state);

        // First call indexes the fixture.
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/map")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let first: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(first["files"], 1, "first call indexes the one file");
        let first_entities = first["entities"].as_u64().unwrap();
        assert!(first_entities > 0);

        // Second call with nothing touched must skip it. This is the assertion
        // that fails today: the handler re-indexes everything and reports 1.
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/map")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let second: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            second["files"], 0,
            "unchanged file must be skipped, not re-indexed"
        );

        // And the graph must not have grown.
        let resp = app
            .clone()
            .oneshot(Request::get("/api/stats").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let stats: StatsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            stats.nodes as u64, first_entities,
            "a skipped file must leave the graph untouched"
        );

        // Change the file. The old rows must be replaced, not joined.
        std::fs::write(
            &file,
            "struct Config { name: String }\n\nfn helper() -> i32 { 42 }\n\nfn extra() -> u8 { 7 }\n",
        )
        .unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/map")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let third: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(third["files"], 1, "the changed file is re-indexed");
        let third_entities = third["entities"].as_u64().unwrap();
        assert!(
            third_entities > first_entities,
            "the added function must produce more entities"
        );

        // The control that catches skip-without-replace: without `remove_file`,
        // the graph holds the old rows *and* the new ones, and this is where that
        // shows up. Exactly the reported count, nothing carried over.
        let resp = app
            .clone()
            .oneshot(Request::get("/api/stats").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let stats: StatsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            stats.nodes as u64, third_entities,
            "re-indexing a changed file must replace its rows, not duplicate them"
        );

        // `force` is a capability I added, so it gets its own assertion rather
        // than shipping untested: it must rebuild from scratch, which means the
        // totals come back identical instead of doubling.
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/map")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"force":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let forced: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(forced["files"], 1, "force re-indexes even unchanged files");

        let resp = app
            .clone()
            .oneshot(Request::get("/api/stats").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let stats: StatsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            stats.nodes as u64, third_entities,
            "a forced rebuild clears first, so totals must not accumulate"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `POST /api/map` must index a graph, not a bag of nodes. It used to call
    /// `parse_file` and insert nodes only, so edges were silently zero while the
    /// CLI indexed them. Asserts against the database, not just the response.
    #[tokio::test]
    async fn test_map_indexes_edges() {
        let dir = std::env::temp_dir().join(format!("deagle-map-edges-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("lib.rs"),
            "struct Config { name: String }\n\nfn helper() -> i32 { 42 }\n",
        )
        .unwrap();

        let db = GraphDb::in_memory().unwrap();
        let state = Arc::new(AppState {
            db: Mutex::new(db),
            root_dir: dir.clone(),
        });
        let app = Router::new()
            .route("/api/map", post(map))
            .route("/api/stats", get(stats))
            .with_state(state);

        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/map")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(json["files"], 1);
        assert!(
            json["entities"].as_u64().unwrap() > 0,
            "entities were already indexed"
        );

        // The response now reports edges, and that count must be real: the file
        // node contains each definition, so at least two CONTAINS edges exist.
        let reported = json["edges"]
            .as_u64()
            .expect("map must report an edge count");
        assert!(
            reported >= 2,
            "expected at least 2 containment edges, response said {reported}"
        );

        // Control: the same database, read back through /api/stats. A response
        // field alone could be a hardcoded number; this cannot.
        let resp = app
            .oneshot(Request::get("/api/stats").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let stats: StatsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            stats.edges as u64, reported,
            "reported edge count must match what the database actually holds"
        );
    }

    /// `POST /api/rg` must report where each match landed: a 1-indexed
    /// character column and a byte offset that indexes the file.
    #[tokio::test]
    async fn test_rg_reports_column_and_byte_offset() {
        let dir = std::env::temp_dir().join(format!("deagle-rg-api-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "// ééé // TODO\n").unwrap();

        let state = Arc::new(AppState {
            db: Mutex::new(GraphDb::in_memory().unwrap()),
            root_dir: dir.clone(),
        });
        let app = Router::new().route("/api/rg", post(rg)).with_state(state);

        let resp = app
            .oneshot(
                Request::post("/api/rg")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"pattern":"TODO"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let on_disk = std::fs::read(dir.join("a.rs")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 1);
        assert_eq!(json["matches"][0]["line"], 1);
        // "é" is two bytes, so the character column (11) and the byte
        // offset (13) disagree -- that is the distinction under test.
        assert_eq!(json["matches"][0]["column"], 11);
        let byte_offset = json["matches"][0]["byte_offset"].as_u64().unwrap();
        assert!(
            on_disk[byte_offset as usize..].starts_with(b"TODO"),
            "byte_offset must index the file at the match"
        );
        assert_eq!(byte_offset, 13);
    }

    #[tokio::test]
    async fn test_search_with_data() {
        let db = GraphDb::in_memory().unwrap();
        db.insert_node(&deagle_core::Node {
            id: 0,
            name: "hello".into(),
            kind: deagle_core::NodeKind::Function,
            language: deagle_core::Language::Rust,
            file_path: "lib.rs".into(),
            line_start: 1,
            line_end: 5,
            content: None,
        })
        .unwrap();

        let state = Arc::new(AppState {
            db: Mutex::new(db),
            root_dir: PathBuf::from("."),
        });
        let app = Router::new()
            .route("/api/search", get(search))
            .with_state(state);

        let resp = app
            .oneshot(
                Request::get("/api/search?q=hello")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let sr: SearchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(sr.count, 1);
        assert_eq!(sr.results[0].name, "hello");
    }
}
