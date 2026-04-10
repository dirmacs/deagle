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
    extract::{Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
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

    let db_path = std::env::var("DEAGLE_DB")
        .unwrap_or_else(|_| ".deagle/graph.db".to_string());
    let root_dir = std::env::var("DEAGLE_ROOT")
        .unwrap_or_else(|_| ".".to_string());
    let port: u16 = std::env::var("DEAGLE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3500);

    std::fs::create_dir_all(std::path::Path::new(&db_path).parent().unwrap_or(std::path::Path::new("."))).ok();

    let db = GraphDb::open(std::path::Path::new(&db_path))
        .expect("Failed to open graph database");

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
    let results = db.search_nodes(&params.q)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let filtered: Vec<NodeJson> = results
        .into_iter()
        .filter(|n| params.kind.as_ref().map_or(true, |k| n.kind.to_string() == *k))
        .map(|n| NodeJson {
            name: n.name,
            kind: n.kind.to_string(),
            language: n.language.to_string(),
            file_path: n.file_path,
            line_start: n.line_start,
        })
        .collect();

    let count = filtered.len();
    Ok(Json(SearchResponse { results: filtered, count }))
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
    let nodes = db.node_count().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let edges = db.edge_count().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(StatsResponse { nodes, edges }))
}

#[derive(Deserialize)]
struct MapRequest {
    dir: Option<String>,
}

#[derive(Serialize)]
struct MapResponse {
    files: usize,
    entities: usize,
}

async fn map(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MapRequest>,
) -> Result<Json<MapResponse>, (StatusCode, String)> {
    let dir = req.dir.map(PathBuf::from).unwrap_or_else(|| state.root_dir.clone());

    let db = state.db.lock().await;
    db.clear().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut files = 0usize;
    let mut entities = 0usize;

    let walker = ignore::WalkBuilder::new(&dir)
        .hidden(true).git_ignore(true).build();

    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = deagle_core::Language::from_extension(ext);
        if lang == deagle_core::Language::Unknown { continue; }

        let content = std::fs::read_to_string(path).unwrap_or_default();
        if content.is_empty() { continue; }

        let rel = path.strip_prefix(&dir).unwrap_or(path);
        if let Ok(nodes) = deagle_parse::parse_file(rel, &content, lang) {
            for n in &nodes { let _ = db.insert_node(n); }
            entities += nodes.len();
            files += 1;
        }
    }

    Ok(Json(MapResponse { files, entities }))
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
    let dir = req.dir.map(PathBuf::from).unwrap_or_else(|| state.root_dir.clone());
    let mut matches = Vec::new();

    let walker = ignore::WalkBuilder::new(&dir)
        .hidden(true).git_ignore(true).build();

    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = deagle_core::Language::from_extension(ext);
        if lang == deagle_core::Language::Unknown { continue; }

        let content = std::fs::read_to_string(path).unwrap_or_default();
        if content.is_empty() { continue; }

        let rel = path.strip_prefix(&dir).unwrap_or(path);
        if let Ok(ms) = deagle_parse::pattern::search_pattern(rel, &content, &req.pattern, lang) {
            for m in ms {
                matches.push(PatternMatch {
                    file: m.file_path,
                    line: m.line_start,
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
    let dir = req.dir.map(PathBuf::from).unwrap_or_else(|| state.root_dir.clone());

    let results = deagle_parse::text_search::search_directory(&dir, &req.pattern, None)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let matches: Vec<PatternMatch> = results
        .into_iter()
        .map(|m| PatternMatch {
            file: m.file_path,
            line: m.line_number as u32,
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
        let resp = app.oneshot(Request::get("/health").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_stats_empty() {
        let app = test_app();
        let resp = app.oneshot(Request::get("/api/stats").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let stats: StatsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(stats.nodes, 0);
        assert_eq!(stats.edges, 0);
    }

    #[tokio::test]
    async fn test_search_empty_db() {
        let app = test_app();
        let resp = app.oneshot(
            Request::get("/api/search?q=test").body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let sr: SearchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(sr.count, 0);
    }

    #[tokio::test]
    async fn test_search_with_data() {
        let db = GraphDb::in_memory().unwrap();
        db.insert_node(&deagle_core::Node {
            id: 0, name: "hello".into(), kind: deagle_core::NodeKind::Function,
            language: deagle_core::Language::Rust, file_path: "lib.rs".into(),
            line_start: 1, line_end: 5, content: None,
        }).unwrap();

        let state = Arc::new(AppState {
            db: Mutex::new(db),
            root_dir: PathBuf::from("."),
        });
        let app = Router::new()
            .route("/api/search", get(search))
            .with_state(state);

        let resp = app.oneshot(
            Request::get("/api/search?q=hello").body(Body::empty()).unwrap()
        ).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let sr: SearchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(sr.count, 1);
        assert_eq!(sr.results[0].name, "hello");
    }
}
