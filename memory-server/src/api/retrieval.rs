//! Retrieval API handlers
//!
//! Implements:
//! - POST /api/v1/memories/retrieve - Retrieve memories based on query and filters
//! - POST /api/v1/memories/retrieve/auto - Auto retrieve with query parsing and formatted output

use axum::{extract::State, routing::post, Json, Router};
use chrono::{DateTime, Utc};
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
    /// Owner ID - the unique identifier of the memory owner
    pub owner_id: Option<String>,
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
            owner_id: req.owner_id,
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

/// Request body for auto retrieval
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AutoRetrieveApiRequest {
    /// User query text (will be parsed by LLM to extract search intent)
    pub query: String,
    /// Owner ID - the unique identifier of the memory owner (required)
    pub owner_id: String,
    /// Optional context to help LLM better understand the query
    #[serde(default)]
    pub context: Option<String>,
    /// Filter by scope type
    pub scope_type: Option<ScopeType>,
    /// Filter by scope ID
    pub scope_id: Option<String>,
    /// Maximum number of results (default: 10)
    pub top_k: Option<usize>,
    /// Minimum score threshold
    pub min_score: Option<f32>,
}

/// A single record in the auto retrieval response
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AutoRetrieveRecord {
    /// Record number (1-indexed)
    pub record: usize,
    /// Event time (when the memory was created or event occurred)
    pub time: DateTime<Utc>,
    /// Event description (from event_source or scene)
    pub event: String,
    /// Inferred memory content
    pub infer: String,
    /// Relevance score
    pub score: f32,
    /// Memory category if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<MemoryCategory>,
    /// Tags if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Response for auto retrieval (formatted output)
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AutoRetrieveApiResponse {
    /// Parsed query intent (what the LLM understood from the query)
    pub parsed_intent: String,
    /// Retrieved records in formatted structure
    pub records: Vec<AutoRetrieveRecord>,
    /// Total candidates found
    pub total_candidates: usize,
    /// Formatted markdown output for direct use
    pub markdown: String,
}

/// Create retrieval routes
pub fn retrieval_routes() -> Router<AppState> {
    Router::new()
        .route("/retrieve", post(retrieve_memories))
        .route("/retrieve/auto", post(auto_retrieve_memories))
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

/// POST /api/v1/memories/retrieve/auto - Auto retrieve with query parsing
///
/// Automatically retrieves memories by:
/// 1. Parsing user query through LLM to understand intent
/// 2. Performing hybrid search (vector + fulltext)
/// 3. Returning formatted markdown output
#[utoipa::path(
    post,
    path = "/api/v1/memories/retrieve/auto",
    tag = "retrieval",
    request_body = AutoRetrieveApiRequest,
    responses(
        (status = 200, description = "Memories retrieved successfully", body = AutoRetrieveApiResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn auto_retrieve_memories(
    State(state): State<AppState>,
    Json(request): Json<AutoRetrieveApiRequest>,
) -> AppResult<Json<AutoRetrieveApiResponse>> {
    // Build retrieve request with query enhancement enabled
    let retrieve_request = RetrieveRequest {
        query: request.query.clone(),
        owner_id: Some(request.owner_id.clone()),
        scope_type: request.scope_type,
        scope_id: request.scope_id,
        layers: None,
        scenes: None,
        categories: None,
        tags: None,
        event_source_prefix: None,
        top_k: request.top_k,
        min_score: request.min_score,
        use_fulltext: Some(true),
        use_vector: Some(true),
        fulltext_weight: None,
        highlight: Some(true),
    };

    // Use retrieve_with_enhancement for better query understanding
    let result = state
        .memory_guard
        .retrieve_with_enhancement(retrieve_request, true, None, None)
        .await?;

    // Convert to formatted records
    let records: Vec<AutoRetrieveRecord> = result
        .memories
        .iter()
        .enumerate()
        .map(|(idx, rm)| {
            let event = rm
                .memory
                .event_source
                .clone()
                .unwrap_or_else(|| rm.memory.scene.clone());
            let time = rm.memory.event_time.unwrap_or(rm.memory.created_at);

            AutoRetrieveRecord {
                record: idx + 1,
                time,
                event,
                infer: rm.memory.content.clone(),
                score: rm.score,
                category: rm.memory.category.clone(),
                tags: rm.memory.tags.clone(),
            }
        })
        .collect();

    // Generate markdown output
    let markdown = format_records_as_markdown(&records);

    // Use the original query as parsed intent (could be enhanced with LLM parsing later)
    let parsed_intent = if let Some(ctx) = &request.context {
        format!("{} (context: {})", request.query, ctx)
    } else {
        request.query.clone()
    };

    let response = AutoRetrieveApiResponse {
        parsed_intent,
        records,
        total_candidates: result.total_candidates,
        markdown,
    };

    Ok(Json(response))
}

/// Format records as markdown string
fn format_records_as_markdown(records: &[AutoRetrieveRecord]) -> String {
    if records.is_empty() {
        return "No matching records found.".to_string();
    }

    let mut output = String::new();
    for record in records {
        output.push_str(&format!("record: {}\n", record.record));
        output.push_str(&format!(
            "time: {}\n",
            record.time.format("%Y-%m-%d %H:%M:%S")
        ));
        output.push_str(&format!("event: {}\n", record.event));
        output.push_str(&format!("infer: {}\n", record.infer));
        output.push_str(&format!("score: {:.2}\n", record.score));
        if let Some(category) = &record.category {
            output.push_str(&format!("category: {:?}\n", category));
        }
        if let Some(tags) = &record.tags {
            if !tags.is_empty() {
                output.push_str(&format!("tags: {}\n", tags.join(", ")));
            }
        }
        output.push('\n');
    }
    output.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retrieve_request_conversion() {
        let api_request = RetrieveApiRequest {
            query: "test query".to_string(),
            owner_id: Some("owner123".to_string()),
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
        assert_eq!(retrieve_request.owner_id, api_request.owner_id);
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
            owner_id: None,
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

    #[test]
    fn test_auto_retrieve_request_parsing() {
        let json = r#"{
            "query": "用户喜欢什么颜色",
            "owner_id": "owner123",
            "scope_type": "user",
            "scope_id": "user123",
            "top_k": 5
        }"#;

        let api_request: AutoRetrieveApiRequest = serde_json::from_str(json).unwrap();
        assert_eq!(api_request.query, "用户喜欢什么颜色");
        assert_eq!(api_request.owner_id, "owner123");
        assert_eq!(api_request.scope_type, Some(ScopeType::User));
        assert_eq!(api_request.scope_id, Some("user123".to_string()));
        assert_eq!(api_request.top_k, Some(5));
        assert!(api_request.context.is_none());
    }

    #[test]
    fn test_auto_retrieve_request_minimal() {
        let json = r#"{
            "query": "test query",
            "owner_id": "owner456"
        }"#;

        let api_request: AutoRetrieveApiRequest = serde_json::from_str(json).unwrap();
        assert_eq!(api_request.query, "test query");
        assert_eq!(api_request.owner_id, "owner456");
        assert!(api_request.scope_type.is_none());
        assert!(api_request.scope_id.is_none());
        assert!(api_request.top_k.is_none());
        assert!(api_request.min_score.is_none());
    }

    #[test]
    fn test_format_records_as_markdown_empty() {
        let records: Vec<AutoRetrieveRecord> = vec![];
        let markdown = format_records_as_markdown(&records);
        assert_eq!(markdown, "No matching records found.");
    }

    #[test]
    fn test_format_records_as_markdown() {
        let records = vec![
            AutoRetrieveRecord {
                record: 1,
                time: Utc::now(),
                event: "button_click:like".to_string(),
                infer: "用户喜欢深色模式".to_string(),
                score: 0.95,
                category: Some(MemoryCategory::UserPreference),
                tags: Some(vec!["dark_mode".to_string(), "ui".to_string()]),
            },
            AutoRetrieveRecord {
                record: 2,
                time: Utc::now(),
                event: "settings.theme".to_string(),
                infer: "用户偏好简洁界面".to_string(),
                score: 0.82,
                category: None,
                tags: None,
            },
        ];

        let markdown = format_records_as_markdown(&records);
        assert!(markdown.contains("record: 1"));
        assert!(markdown.contains("record: 2"));
        assert!(markdown.contains("event: button_click:like"));
        assert!(markdown.contains("infer: 用户喜欢深色模式"));
        assert!(markdown.contains("score: 0.95"));
        assert!(markdown.contains("tags: dark_mode, ui"));
    }
}
