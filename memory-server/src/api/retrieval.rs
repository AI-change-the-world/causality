//! Retrieval API handlers
//!
//! Implements:
//! - POST /api/v1/memories/retrieve - Retrieve memories based on query and filters
//! - POST /api/v1/memories/retrieve/auto - Auto retrieve with query parsing and formatted output
//!
//! New architecture:
//! - Removed scope_type, layer, scene, event_source_prefix filters
//! - Added category_prefix filter
//! - Added include_evidence and include_history options
//! - owner_id is now required

use axum::{extract::State, routing::post, Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::memory::GetMemoryResponse;
use crate::api::AppState;
use crate::domain::{Event, Memory};
use crate::error::AppResult;
use crate::service::RetrieveRequest;

/// Request body for memory retrieval
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RetrieveApiRequest {
    /// Query text for semantic search and full-text search
    pub query: String,
    /// Owner ID - the unique identifier of the memory owner (required)
    pub owner_id: String,
    /// Filter by scope ID (when provided, also includes global memories)
    pub scope_id: Option<String>,
    /// Maximum number of results (default: 10)
    pub top_k: Option<usize>,
    /// Minimum score threshold
    pub min_score: Option<f32>,
    /// Minimum confidence threshold
    pub min_confidence: Option<f32>,
    /// Whether to use full-text search (default: true)
    pub use_fulltext: Option<bool>,
    /// Whether to use vector search (default: true)
    pub use_vector: Option<bool>,
    /// Custom weight for full-text search score (overrides config)
    pub fulltext_weight: Option<f32>,
    /// Whether to return highlighted snippets (default: false)
    pub highlight: Option<bool>,
    /// Whether to include source events (evidence) for each memory
    #[serde(default)]
    pub include_evidence: Option<bool>,
    /// Whether to include version history for each memory
    #[serde(default)]
    pub include_history: Option<bool>,
}

impl From<RetrieveApiRequest> for RetrieveRequest {
    fn from(req: RetrieveApiRequest) -> Self {
        RetrieveRequest {
            query: req.query,
            owner_id: req.owner_id,
            scope_id: req.scope_id,
            category_prefix: None,
            tags: None,
            top_k: req.top_k,
            min_score: req.min_score,
            min_confidence: req.min_confidence,
            use_fulltext: req.use_fulltext,
            use_vector: req.use_vector,
            fulltext_weight: req.fulltext_weight,
            highlight: req.highlight,
            include_evidence: req.include_evidence,
            include_history: req.include_history,
        }
    }
}

/// Evidence (source events) for a memory in API response
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EvidenceResponse {
    /// Events that created or reinforced this memory
    pub events: Vec<EventResponse>,
    /// Total count of supporting events
    pub total_count: usize,
}

/// Event in API response
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EventResponse {
    /// Event ID
    pub id: String,
    /// Event content
    pub content: String,
    /// Event context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Event summary
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Event source
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Event time
    pub event_time: DateTime<Utc>,
}

impl From<Event> for EventResponse {
    fn from(event: Event) -> Self {
        EventResponse {
            id: event.id.to_string(),
            content: event.content,
            context: event.context,
            summary: event.summary,
            source: event.source,
            event_time: event.event_time,
        }
    }
}

/// Version history for a memory in API response
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HistoryResponse {
    /// All versions of this memory (ordered by version_number)
    pub versions: Vec<VersionResponse>,
    /// The current (latest) version ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_version_id: Option<String>,
}

/// A version in the history
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VersionResponse {
    /// Memory ID
    pub id: String,
    /// Version number
    pub version_number: i32,
    /// Whether this is the current version
    pub is_current_version: bool,
    /// Content of this version
    pub content: String,
    /// Status of this version
    pub status: String,
    /// Created at
    pub created_at: DateTime<Utc>,
}

impl From<Memory> for VersionResponse {
    fn from(memory: Memory) -> Self {
        VersionResponse {
            id: memory.id.to_string(),
            version_number: memory.version_number,
            is_current_version: memory.is_current_version,
            content: memory.content,
            status: memory.status.to_string(),
            created_at: memory.created_at,
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
    /// Source events (evidence) for this memory (if include_evidence was true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceResponse>,
    /// Version history for this memory (if include_history was true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<HistoryResponse>,
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
    /// Category of the memory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Inferred memory content
    pub infer: String,
    /// Relevance score
    pub score: f32,
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
/// 1. For memories with embedding_provider: vector search + fulltext search
/// 2. For memories without embedding_provider: fulltext search only
/// 3. Merges results and computes composite scores
/// 4. Returns top-K results sorted by score
/// 5. Updates hit counts for returned memories
/// 6. Optionally loads evidence and history
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
    use crate::embedding::EmbeddingRequest;
    use tracing::{debug, warn};

    debug!(
        query = %request.query,
        owner_id = %request.owner_id,
        scope_id = ?request.scope_id,
        "retrieve_memories: starting request"
    );

    let include_global = request.scope_id.is_some();
    let top_k = request.top_k.unwrap_or(10);
    let use_vector = request.use_vector.unwrap_or(true);
    let mut all_similarities: Vec<(uuid::Uuid, f32)> = Vec::new();

    // Step 1: Try vector search if enabled
    if use_vector {
        // Query memories grouped by embedding_provider (only those with embeddings)
        let memories_by_provider = state
            .memory_guard
            .memory_repo()
            .find_by_owner_scope_grouped_by_provider(
                &request.owner_id,
                request.scope_id.as_deref(),
                include_global,
            )
            .await?;

        debug!(
            provider_count = memories_by_provider.len(),
            "retrieve_memories: found memories with embeddings grouped by provider"
        );

        // Use global embedding provider for query embedding
        let embedding_provider_name = &state.embedding_provider_name;

        // Generate query embedding using global provider
        let embedding_request = EmbeddingRequest::new(&request.query);
        match state.embedding_provider.embed(embedding_request).await {
            Ok(query_embedding) => {
                // Search in the global provider's Qdrant collection
                let search_results = match state
                    .qdrant_repo
                    .search(
                        embedding_provider_name,
                        query_embedding.embedding,
                        top_k * 2, // Get more candidates for filtering
                        None,
                    )
                    .await
                {
                    Ok(results) => results,
                    Err(e) => {
                        warn!(provider = %embedding_provider_name, error = %e, "Vector search failed");
                        vec![]
                    }
                };

                debug!(
                    provider = %embedding_provider_name,
                    results = search_results.len(),
                    "retrieve_memories: vector search completed"
                );

                // Add results to combined list
                for result in search_results {
                    all_similarities.push((result.memory_id, result.score));
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to generate query embedding");
            }
        }
    }

    debug!(
        total_similarities = all_similarities.len(),
        "retrieve_memories: combined all vector search results"
    );

    let mut retrieve_request: RetrieveRequest = request.into();

    // If no vector results, disable vector search in the request
    let has_vector_results = !all_similarities.is_empty();
    if !has_vector_results {
        retrieve_request.use_vector = Some(false);
    }

    // Step 2: Use retrieval engine with the combined similarity results
    // This will also do fulltext search for ALL memories (including those without embeddings)
    let result = state
        .retrieval_engine
        .retrieve(
            retrieve_request,
            if has_vector_results {
                Some(all_similarities)
            } else {
                None
            },
            None,
        )
        .await?;

    let memories: Vec<RetrievedMemoryResponse> = result
        .memories
        .into_iter()
        .map(|rm| {
            let evidence = rm.evidence.map(|e| EvidenceResponse {
                events: e.events.into_iter().map(Into::into).collect(),
                total_count: e.total_count,
            });

            let history = rm.history.map(|h| HistoryResponse {
                versions: h.versions.into_iter().map(Into::into).collect(),
                current_version_id: h.current_version.map(|m| m.id.to_string()),
            });

            RetrievedMemoryResponse {
                memory: rm.memory.into(),
                score: rm.score,
                similarity: rm.similarity,
                text_match_score: rm.text_match_score,
                highlights: rm.highlights,
                evidence,
                history,
            }
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
/// 1. For memories with embedding_provider: vector search + fulltext search
/// 2. For memories without embedding_provider: fulltext search only
/// 3. Merges results and returns formatted markdown output
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
    use crate::embedding::EmbeddingRequest;
    use tracing::{debug, warn};

    debug!(
        query = %request.query,
        owner_id = %request.owner_id,
        scope_id = ?request.scope_id,
        "auto_retrieve_memories: starting request"
    );

    let include_global = request.scope_id.is_some();
    let top_k = request.top_k.unwrap_or(10);
    let mut all_similarities: Vec<(uuid::Uuid, f32)> = Vec::new();

    // Step 1: Try vector search for memories with embeddings
    let memories_by_provider = state
        .memory_guard
        .memory_repo()
        .find_by_owner_scope_grouped_by_provider(
            &request.owner_id,
            request.scope_id.as_deref(),
            include_global,
        )
        .await?;

    debug!(
        provider_count = memories_by_provider.len(),
        "auto_retrieve_memories: found memories with embeddings grouped by provider"
    );

    // Use global embedding provider for query embedding
    let embedding_provider_name = &state.embedding_provider_name;

    // Generate query embedding using global provider
    let embedding_request = EmbeddingRequest::new(&request.query);
    match state.embedding_provider.embed(embedding_request).await {
        Ok(query_embedding) => {
            // Search in the global provider's Qdrant collection
            let search_results = match state
                .qdrant_repo
                .search(
                    embedding_provider_name,
                    query_embedding.embedding,
                    top_k * 2,
                    None,
                )
                .await
            {
                Ok(results) => results,
                Err(e) => {
                    warn!(provider = %embedding_provider_name, error = %e, "Vector search failed");
                    vec![]
                }
            };

            debug!(
                provider = %embedding_provider_name,
                results = search_results.len(),
                "auto_retrieve_memories: vector search completed"
            );

            for result in search_results {
                all_similarities.push((result.memory_id, result.score));
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to generate query embedding");
        }
    }

    // Build retrieve request - use_vector depends on whether we have vector results
    let has_vector_results = !all_similarities.is_empty();
    let retrieve_request = RetrieveRequest {
        query: request.query.clone(),
        owner_id: request.owner_id.clone(),
        scope_id: request.scope_id,
        category_prefix: None,
        tags: None,
        top_k: request.top_k,
        min_score: request.min_score,
        min_confidence: None,
        use_fulltext: Some(true),
        use_vector: Some(has_vector_results),
        fulltext_weight: None,
        highlight: Some(true),
        include_evidence: None,
        include_history: None,
    };

    // Use retrieval engine with the combined similarity results
    let result = state
        .retrieval_engine
        .retrieve(
            retrieve_request,
            if has_vector_results {
                Some(all_similarities)
            } else {
                None
            },
            None,
        )
        .await?;

    // Convert to formatted records
    let records: Vec<AutoRetrieveRecord> = result
        .memories
        .iter()
        .enumerate()
        .map(|(idx, rm)| {
            let time = rm.memory.created_at;

            AutoRetrieveRecord {
                record: idx + 1,
                time,
                category: rm.memory.category.clone(),
                infer: rm.memory.content.clone(),
                score: rm.score,
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
        if let Some(category) = &record.category {
            output.push_str(&format!("category: {}\n", category));
        }
        output.push_str(&format!("infer: {}\n", record.infer));
        output.push_str(&format!("score: {:.2}\n", record.score));
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
            owner_id: "owner123".to_string(),
            scope_id: Some("scope456".to_string()),
            top_k: Some(20),
            min_score: Some(0.5),
            min_confidence: Some(0.7),
            use_fulltext: Some(true),
            use_vector: Some(true),
            fulltext_weight: Some(0.2),
            highlight: Some(true),
            include_evidence: Some(true),
            include_history: Some(false),
        };

        let retrieve_request: RetrieveRequest = api_request.clone().into();

        assert_eq!(retrieve_request.query, api_request.query);
        assert_eq!(retrieve_request.owner_id, api_request.owner_id);
        assert_eq!(retrieve_request.scope_id, api_request.scope_id);
        // category_prefix and tags are set to None in conversion (internal use only)
        assert!(retrieve_request.category_prefix.is_none());
        assert!(retrieve_request.tags.is_none());
        assert_eq!(retrieve_request.top_k, api_request.top_k);
        assert_eq!(retrieve_request.min_score, api_request.min_score);
        assert_eq!(retrieve_request.min_confidence, api_request.min_confidence);
        assert_eq!(retrieve_request.use_fulltext, api_request.use_fulltext);
        assert_eq!(retrieve_request.use_vector, api_request.use_vector);
        assert_eq!(
            retrieve_request.fulltext_weight,
            api_request.fulltext_weight
        );
        assert_eq!(retrieve_request.highlight, api_request.highlight);
        assert_eq!(
            retrieve_request.include_evidence,
            api_request.include_evidence
        );
        assert_eq!(
            retrieve_request.include_history,
            api_request.include_history
        );
    }

    #[test]
    fn test_retrieve_request_minimal() {
        let api_request = RetrieveApiRequest {
            query: "simple query".to_string(),
            owner_id: "owner123".to_string(),
            scope_id: None,
            top_k: None,
            min_score: None,
            min_confidence: None,
            use_fulltext: None,
            use_vector: None,
            fulltext_weight: None,
            highlight: None,
            include_evidence: None,
            include_history: None,
        };

        let retrieve_request: RetrieveRequest = api_request.into();

        assert_eq!(retrieve_request.query, "simple query");
        assert_eq!(retrieve_request.owner_id, "owner123");
        assert!(retrieve_request.scope_id.is_none());
        assert!(retrieve_request.category_prefix.is_none());
        assert!(retrieve_request.tags.is_none());
        assert!(retrieve_request.top_k.is_none());
        assert!(retrieve_request.min_score.is_none());
        assert!(retrieve_request.min_confidence.is_none());
        assert!(retrieve_request.use_fulltext.is_none());
        assert!(retrieve_request.use_vector.is_none());
        assert!(retrieve_request.fulltext_weight.is_none());
        assert!(retrieve_request.highlight.is_none());
        assert!(retrieve_request.include_evidence.is_none());
        assert!(retrieve_request.include_history.is_none());
    }

    #[test]
    fn test_retrieve_request_parsing() {
        let json = r#"{
            "query": "test query",
            "owner_id": "owner123"
        }"#;

        let api_request: RetrieveApiRequest = serde_json::from_str(json).unwrap();
        assert_eq!(api_request.query, "test query");
        assert_eq!(api_request.owner_id, "owner123");
    }

    #[test]
    fn test_auto_retrieve_request_parsing() {
        let json = r#"{
            "query": "用户喜欢什么颜色",
            "owner_id": "owner123",
            "scope_id": "user123",
            "top_k": 5
        }"#;

        let api_request: AutoRetrieveApiRequest = serde_json::from_str(json).unwrap();
        assert_eq!(api_request.query, "用户喜欢什么颜色");
        assert_eq!(api_request.owner_id, "owner123");
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
                category: Some("preference.ui".to_string()),
                infer: "用户喜欢深色模式".to_string(),
                score: 0.95,
                tags: Some(vec!["dark_mode".to_string(), "ui".to_string()]),
            },
            AutoRetrieveRecord {
                record: 2,
                time: Utc::now(),
                category: None,
                infer: "用户偏好简洁界面".to_string(),
                score: 0.82,
                tags: None,
            },
        ];

        let markdown = format_records_as_markdown(&records);
        assert!(markdown.contains("record: 1"));
        assert!(markdown.contains("record: 2"));
        assert!(markdown.contains("category: preference.ui"));
        assert!(markdown.contains("infer: 用户喜欢深色模式"));
        assert!(markdown.contains("score: 0.95"));
        assert!(markdown.contains("tags: dark_mode, ui"));
    }
}
