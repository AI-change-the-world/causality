//! Retrieval API handlers
//!
//! Implements:
//! - POST /api/v1/memories/retrieve - Retrieve memories based on query and filters

use axum::{extract::State, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::memory::GetMemoryResponse;
use crate::api::AppState;
use crate::domain::{Layer, MemoryCategory, ScopeType};
use crate::error::AppResult;
use crate::service::RetrieveRequest;

/// Request body for memory retrieval
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RetrieveApiRequest {
    /// Query text for semantic search and full-text search
    pub query: String,
    /// Filter by scope type
    pub scope_type: Option<ScopeType>,
    /// Filter by scope ID
    pub scope_id: Option<String>,
    /// Filter by layers
    pub layers: Option<Vec<Layer>>,
    /// Filter by scenes
    pub scenes: Option<Vec<String>>,
    /// Filter by memory categories (LLM-classified)
    pub categories: Option<Vec<MemoryCategory>>,
    /// Filter by tags (extracted keywords)
    pub tags: Option<Vec<String>>,
    /// Filter by event source prefix
    pub event_source_prefix: Option<String>,
    /// Maximum number of results (default: 10)
    pub top_k: Option<usize>,
    /// Minimum score threshold
    pub min_score: Option<f32>,
    /// Whether to use full-text search (default: true)
    pub use_fulltext: Option<bool>,
    /// Whether to use vector search (default: true)
    pub use_vector: Option<bool>,
    /// Custom weight for full-text search score (overrides config)
    pub fulltext_weight: Option<f32>,
    /// Whether to return highlighted snippets (default: false)
    pub highlight: Option<bool>,
    /// Whether to enhance the query with semantic synonyms (default: false)
    /// When enabled, the query will be expanded with related terms to improve retrieval results.
    #[serde(default)]
    pub enhance_query: bool,
}

impl From<RetrieveApiRequest> for RetrieveRequest {
    fn from(req: RetrieveApiRequest) -> Self {
        RetrieveRequest {
            query: req.query,
            scope_type: req.scope_type,
            scope_id: req.scope_id,
            layers: req.layers,
            scenes: req.scenes,
            categories: req.categories,
            tags: req.tags,
            event_source_prefix: req.event_source_prefix,
            top_k: req.top_k,
            min_score: req.min_score,
            use_fulltext: req.use_fulltext,
            use_vector: req.use_vector,
            fulltext_weight: req.fulltext_weight,
            highlight: req.highlight,
        }
    }
}

/// A retrieved memory with scores
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RetrievedMemoryResponse {
    /// The memory record
    pub memory: GetMemoryResponse,
    /// Composite score
    pub score: f32,
    /// Vector similarity score
    pub similarity: f32,
    /// Full-text search match score (if full-text search was used)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_match_score: Option<f32>,
    /// Highlighted snippets from the content (if highlighting was requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlights: Option<Vec<String>>,
}

/// Response for memory retrieval
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RetrieveApiResponse {
    /// Retrieved memories sorted by score
    pub memories: Vec<RetrievedMemoryResponse>,
    /// Total candidates after structured filtering
    pub total_candidates: usize,
}

/// Create retrieval routes
pub fn retrieval_routes() -> Router<AppState> {
    Router::new().route("/retrieve", post(retrieve_memories))
}

/// POST /api/v1/memories/retrieve - Retrieve memories
///
/// Retrieves memories based on query and filters:
/// 1. Applies structured filters (scope, scene, layer, event_source_prefix)
/// 2. Optionally enhances query with semantic synonyms (if enhance_query=true)
/// 3. Performs vector similarity search
/// 4. Computes composite scores
/// 5. Returns top-K results sorted by score
/// 6. Updates hit counts for returned memories
#[utoipa::path(
    post,
    path = "/api/v1/memories/retrieve",
    tag = "retrieval",
    request_body = RetrieveApiRequest,
    responses(
        (status = 200, description = "Memories retrieved successfully", body = RetrieveApiResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn retrieve_memories(
    State(state): State<AppState>,
    Json(request): Json<RetrieveApiRequest>,
) -> AppResult<Json<RetrieveApiResponse>> {
    let enhance_query = request.enhance_query;
    let retrieve_request: RetrieveRequest = request.into();

    // Use retrieve_with_enhancement if enhance_query is true, otherwise use regular retrieve
    let result = if enhance_query {
        state
            .memory_guard
            .retrieve_with_enhancement(retrieve_request, true, None, None)
            .await?
    } else {
        state
            .retrieval_engine
            .retrieve(retrieve_request, None, None)
            .await?
    };

    let memories: Vec<RetrievedMemoryResponse> = result
        .memories
        .into_iter()
        .map(|rm| RetrievedMemoryResponse {
            memory: rm.memory.into(),
            score: rm.score,
            similarity: rm.similarity,
            text_match_score: rm.text_match_score,
            highlights: rm.highlights,
        })
        .collect();

    let response = RetrieveApiResponse {
        memories,
        total_candidates: result.total_candidates,
    };

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retrieve_request_conversion() {
        let api_request = RetrieveApiRequest {
            query: "test query".to_string(),
            scope_type: Some(ScopeType::User),
            scope_id: Some("user123".to_string()),
            layers: Some(vec![Layer::Session, Layer::Task]),
            scenes: Some(vec!["work.review".to_string()]),
            categories: Some(vec![
                MemoryCategory::UserPreference,
                MemoryCategory::BehaviorPattern,
            ]),
            tags: Some(vec!["important".to_string(), "work".to_string()]),
            event_source_prefix: Some("button_click".to_string()),
            top_k: Some(20),
            min_score: Some(0.5),
            use_fulltext: Some(true),
            use_vector: Some(true),
            fulltext_weight: Some(0.2),
            highlight: Some(true),
            enhance_query: false,
        };

        let retrieve_request: RetrieveRequest = api_request.clone().into();

        assert_eq!(retrieve_request.query, api_request.query);
        assert_eq!(retrieve_request.scope_type, api_request.scope_type);
        assert_eq!(retrieve_request.scope_id, api_request.scope_id);
        assert_eq!(retrieve_request.layers, api_request.layers);
        assert_eq!(retrieve_request.scenes, api_request.scenes);
        assert_eq!(retrieve_request.categories, api_request.categories);
        assert_eq!(retrieve_request.tags, api_request.tags);
        assert_eq!(
            retrieve_request.event_source_prefix,
            api_request.event_source_prefix
        );
        assert_eq!(retrieve_request.top_k, api_request.top_k);
        assert_eq!(retrieve_request.min_score, api_request.min_score);
        assert_eq!(retrieve_request.use_fulltext, api_request.use_fulltext);
        assert_eq!(retrieve_request.use_vector, api_request.use_vector);
        assert_eq!(
            retrieve_request.fulltext_weight,
            api_request.fulltext_weight
        );
        assert_eq!(retrieve_request.highlight, api_request.highlight);
    }

    #[test]
    fn test_retrieve_request_minimal() {
        let api_request = RetrieveApiRequest {
            query: "simple query".to_string(),
            scope_type: None,
            scope_id: None,
            layers: None,
            scenes: None,
            categories: None,
            tags: None,
            event_source_prefix: None,
            top_k: None,
            min_score: None,
            use_fulltext: None,
            use_vector: None,
            fulltext_weight: None,
            highlight: None,
            enhance_query: false,
        };

        let retrieve_request: RetrieveRequest = api_request.into();

        assert_eq!(retrieve_request.query, "simple query");
        assert!(retrieve_request.scope_type.is_none());
        assert!(retrieve_request.scope_id.is_none());
        assert!(retrieve_request.layers.is_none());
        assert!(retrieve_request.scenes.is_none());
        assert!(retrieve_request.categories.is_none());
        assert!(retrieve_request.tags.is_none());
        assert!(retrieve_request.event_source_prefix.is_none());
        assert!(retrieve_request.top_k.is_none());
        assert!(retrieve_request.min_score.is_none());
        assert!(retrieve_request.use_fulltext.is_none());
        assert!(retrieve_request.use_vector.is_none());
        assert!(retrieve_request.fulltext_weight.is_none());
        assert!(retrieve_request.highlight.is_none());
    }

    #[test]
    fn test_retrieve_request_with_enhance_query() {
        let json = r#"{
            "query": "test query",
            "enhance_query": true
        }"#;

        let api_request: RetrieveApiRequest = serde_json::from_str(json).unwrap();
        assert!(api_request.enhance_query);
    }

    #[test]
    fn test_retrieve_request_enhance_query_default() {
        let json = r#"{
            "query": "test query"
        }"#;

        let api_request: RetrieveApiRequest = serde_json::from_str(json).unwrap();
        assert!(!api_request.enhance_query); // Default is false
    }
}
