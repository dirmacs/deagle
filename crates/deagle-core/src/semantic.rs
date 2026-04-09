//! Semantic code search via ares-vector.
//!
//! Enables searching code by meaning rather than exact name.
//! Requires the `semantic` feature flag and pre-computed embeddings.

use crate::{DeagleError, Node, Result};
use ares_vector::{VectorDb, Config, DistanceMetric, VectorMetadata};

/// Semantic search index backed by ares-vector.
pub struct SemanticIndex {
    store: VectorDb,
    collection: String,
}

impl SemanticIndex {
    /// Create an in-memory semantic index.
    pub async fn in_memory(collection: &str, dimensions: usize) -> Result<Self> {
        let config = Config::memory();
        let store: std::result::Result<VectorDb, ares_vector::Error> =
            VectorDb::open(config).await;
        let store = store.map_err(|e| DeagleError::Other(format!("vector store: {}", e)))?;

        let cr: std::result::Result<(), ares_vector::Error> =
            store.create_collection(collection, dimensions, DistanceMetric::Cosine).await;
        cr.map_err(|e| DeagleError::Other(format!("create collection: {}", e)))?;

        Ok(Self {
            store,
            collection: collection.to_string(),
        })
    }

    /// Create a persistent semantic index at the given path.
    pub async fn persistent(
        path: impl Into<std::path::PathBuf>,
        collection: &str,
        dimensions: usize,
    ) -> Result<Self> {
        let config = Config::persistent(path);
        let store: std::result::Result<VectorDb, ares_vector::Error> =
            VectorDb::open(config).await;
        let store = store.map_err(|e| DeagleError::Other(format!("vector store: {}", e)))?;

        if !store.collection_exists(collection) {
            let cr: std::result::Result<(), ares_vector::Error> =
                store.create_collection(collection, dimensions, DistanceMetric::Cosine).await;
            cr.map_err(|e| DeagleError::Other(format!("create collection: {}", e)))?;
        }

        Ok(Self {
            store,
            collection: collection.to_string(),
        })
    }

    /// Index a node's content as a vector embedding.
    /// The caller provides the embedding vector (from fastembed or an external API).
    pub async fn index_node(&self, node: &Node, embedding: &[f32]) -> Result<()> {
        let mut metadata = VectorMetadata::new();
        metadata.insert("name", node.name.clone());
        metadata.insert("kind", node.kind.to_string());
        metadata.insert("language", node.language.to_string());
        metadata.insert("file_path", node.file_path.clone());
        metadata.insert("line_start", node.line_start as i64);

        let res: std::result::Result<(), ares_vector::Error> = self
            .store
            .insert(
                &self.collection,
                &node.id.to_string(),
                embedding,
                Some(metadata),
            )
            .await;
        res.map_err(|e| DeagleError::Other(format!("index node: {}", e)))?;

        Ok(())
    }

    /// Search for code semantically similar to a query embedding.
    /// Returns (node_id, score) pairs sorted by similarity.
    pub async fn search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<(String, f32)>> {
        let results: std::result::Result<Vec<ares_vector::SearchResult>, ares_vector::Error> = self
            .store
            .search(&self.collection, query_embedding, top_k)
            .await;
        let results = results.map_err(|e| DeagleError::Other(format!("semantic search: {}", e)))?;

        Ok(results.into_iter().map(|r| (r.id, r.score)).collect())
    }

    /// Get the number of indexed vectors.
    pub fn count(&self) -> Result<usize> {
        self.store
            .count(&self.collection)
            .map_err(|e| DeagleError::Other(format!("count: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_semantic_index_create() {
        let idx = SemanticIndex::in_memory("test_code", 4).await.unwrap();
        assert_eq!(idx.collection, "test_code");
        assert_eq!(idx.count().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_semantic_index_and_search() {
        let idx = SemanticIndex::in_memory("test_srch", 4).await.unwrap();

        let node = Node {
            id: 1,
            name: "process_request".into(),
            kind: crate::NodeKind::Function,
            language: crate::Language::Rust,
            file_path: "src/handler.rs".into(),
            line_start: 10,
            line_end: 30,
            content: Some("pub fn process_request() {}".into()),
        };

        idx.index_node(&node, &[1.0, 0.0, 0.0, 0.0]).await.unwrap();
        assert_eq!(idx.count().unwrap(), 1);

        let results = idx.search(&[0.9, 0.1, 0.0, 0.0], 5).await.unwrap();
        assert!(!results.is_empty(), "Should find indexed node");
    }
}
