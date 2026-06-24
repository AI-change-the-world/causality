//! Retrieval API handlers
//!
//! Implements:
//! - POST /api/v1/systems/{profile_id}/memories/retrieve - Retrieve memories
//! - POST /api/v1/systems/{profile_id}/memories/retrieve/auto - Auto retrieve
//!
//! New architecture:
//! - Removed scope_type, layer, scene, event_source_prefix filters
//! - Retrieval is driven by query + scope + score controls
//! - Added include_evidence and include_history options
//! - owner_id is now required

use axum::{
    extract::{Path, State},
    routing::post,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::memory::GetMemoryResponse;
use crate::api::AppState;
use crate::domain::{Event, EventSource, Memory};
use crate::error::AppResult;
use crate::service::{CurrentMemoryResolution, MemoryHistory, RetrieveRequest, RetrievedMemory};

/// Request body for memory retrieval
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RetrieveApiRequest {
    /// Query text for semantic search and full-text search
    pub query: String,
    /// Owner ID - the unique identifier of the memory owner (required)
    pub owner_id: String,
    /// Filter by scope ID. Null means retrieve global memories only.
    pub scope_id: Option<String>,
    /// Maximum number of results (default: 10)
    pub top_k: Option<usize>,
    /// Advanced retrieval controls. Omit for the standard recall path.
    #[serde(default)]
    pub options: RetrieveOptions,
}

/// Advanced retrieval options for callers that need scoring/debug controls.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct RetrieveOptions {
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

impl RetrieveApiRequest {
    fn into_service_request(self, profile_id: Uuid) -> RetrieveRequest {
        let options = self.options;
        RetrieveRequest {
            profile_id,
            query: self.query,
            owner_id: self.owner_id,
            scope_id: self.scope_id,
            top_k: self.top_k,
            min_score: options.min_score,
            min_confidence: options.min_confidence,
            use_fulltext: options.use_fulltext,
            use_vector: options.use_vector,
            fulltext_weight: options.fulltext_weight,
            highlight: options.highlight,
            include_evidence: options.include_evidence,
            include_history: options.include_history,
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
    /// Event source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<EventSource>,
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

/// A lineage path from one matched node to the current memory returned to the caller.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LineageSourceResponse {
    /// Originally matched memory ID before lineage collapse
    pub source_memory_id: String,
    /// Inclusive path from the source memory to the returned current memory
    pub lineage_to_current: Vec<String>,
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
    /// Source matches merged into this current memory via supersede lineage collapse
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_lineage_sources: Option<Vec<LineageSourceResponse>>,
}

/// Retrieval trace for debugging the recall chain.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RetrievalTraceResponse {
    /// Original user query
    pub query: String,
    /// Whether vector search participated in this retrieval
    pub used_vector: bool,
    /// Whether full-text search participated in this retrieval
    pub used_fulltext: bool,
    /// Scope filter used by the request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    /// Number of vector candidates returned before merge
    pub vector_candidate_count: usize,
    /// Number of candidates after structured filtering
    pub structured_candidate_count: usize,
    /// Number of final results after lineage collapse
    pub final_result_count: usize,
}

/// Response for memory retrieval
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RetrieveApiResponse {
    /// Retrieved memories sorted by score
    pub memories: Vec<RetrievedMemoryResponse>,
    /// Total candidates after structured filtering
    pub total_candidates: usize,
    /// Retrieval trace for debugging and demos
    pub trace: RetrievalTraceResponse,
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
    /// Filter by scope ID. Null means retrieve global memories only.
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
    /// Structured metadata payload
    pub metadata: serde_json::Value,
    /// Inferred memory content
    pub infer: String,
    /// Relevance score
    pub score: f32,
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
    /// Retrieval trace for debugging and demos
    pub trace: RetrievalTraceResponse,
    /// Formatted markdown output for direct use
    pub markdown: String,
}

/// Create retrieval routes
pub fn retrieval_routes() -> Router<AppState> {
    Router::new()
        .route("/retrieve", post(retrieve_memories))
        .route("/retrieve/auto", post(auto_retrieve_memories))
}

/// POST /api/v1/systems/{profile_id}/memories/retrieve - Retrieve memories
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
    path = "/api/v1/systems/{profile_id}/memories/retrieve",
    tag = "retrieval",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID")
    ),
    request_body = RetrieveApiRequest,
    responses(
        (status = 200, description = "Memories retrieved successfully", body = RetrieveApiResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn retrieve_memories(
    State(state): State<AppState>,
    Path(profile_id): Path<Uuid>,
    Json(request): Json<RetrieveApiRequest>,
) -> AppResult<Json<RetrieveApiResponse>> {
    use crate::embedding::EmbeddingRequest;
    use crate::repository::VectorFilter;
    use tracing::{debug, warn};

    let request_body = request;

    debug!(
        profile_id = %profile_id,
        query = %request_body.query,
        owner_id = %request_body.owner_id,
        scope_id = ?request_body.scope_id,
        "retrieve_memories: starting request"
    );

    let include_global = true;
    let top_k = request_body.top_k.unwrap_or(10);
    let use_vector = request_body.options.use_vector.unwrap_or(true);
    let use_fulltext = request_body.options.use_fulltext.unwrap_or(true);
    let mut all_similarities: Vec<(uuid::Uuid, f32)> = Vec::new();

    // Step 1: Try vector search if enabled
    if use_vector {
        // Use global embedding provider for query embedding
        let embedding_provider_name = &state.embedding_provider_name;

        // Generate query embedding using global provider
        let embedding_request = EmbeddingRequest::new(&request_body.query);
        match state.embedding_provider.embed(embedding_request).await {
            Ok(query_embedding) => {
                let filter = VectorFilter {
                    profile_id: Some(profile_id),
                    owner_id: Some(request_body.owner_id.clone()),
                    scope_id: request_body.scope_id.clone(),
                    include_global: Some(include_global),
                    statuses: Some(vec!["active".to_string()]),
                    ..Default::default()
                };

                // Search in the global provider's Qdrant collection
                let search_results = match state
                    .qdrant_repo
                    .search(
                        embedding_provider_name,
                        query_embedding.embedding,
                        top_k * 2, // Get more candidates for filtering
                        Some(filter),
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

    let mut retrieve_request = request_body.clone().into_service_request(profile_id);

    // If no vector results, disable vector search in the request
    let has_vector_results = !all_similarities.is_empty();
    let vector_candidate_count = all_similarities.len();
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

    let collapsed =
        collapse_retrieved_memories_to_current(&state, profile_id, result.memories).await?;
    let final_result_count = collapsed.len();

    let memories = collapsed
        .into_iter()
        .map(collapsed_retrieved_memory_to_response)
        .collect();

    let response = RetrieveApiResponse {
        memories,
        total_candidates: result.total_candidates,
        trace: RetrievalTraceResponse {
            query: request_body.query,
            used_vector: has_vector_results,
            used_fulltext: use_fulltext,
            scope_id: request_body.scope_id,
            vector_candidate_count,
            structured_candidate_count: result.total_candidates,
            final_result_count,
        },
    };

    Ok(Json(response))
}

#[derive(Debug, Clone)]
struct CollapsedRetrievedMemory {
    retrieved: RetrievedMemory,
    merged_lineage_sources: Vec<LineageSourceResponse>,
}

fn collapsed_retrieved_memory_to_response(
    collapsed: CollapsedRetrievedMemory,
) -> RetrievedMemoryResponse {
    let rm = collapsed.retrieved;
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
        merged_lineage_sources: Some(collapsed.merged_lineage_sources),
    }
}

async fn collapse_retrieved_memories_to_current(
    state: &AppState,
    profile_id: Uuid,
    memories: Vec<RetrievedMemory>,
) -> AppResult<Vec<CollapsedRetrievedMemory>> {
    let mut collapsed_by_current: HashMap<Uuid, CollapsedRetrievedMemory> = HashMap::new();
    let mut order: Vec<Uuid> = Vec::new();

    for retrieved in memories {
        let resolution = state
            .retrieval_engine
            .resolve_to_current_memory(retrieved.memory.id)
            .await?;

        if resolution.current_memory.profile_id != profile_id {
            continue;
        }

        let current_id = resolution.current_memory.id;
        let source = lineage_source_response(&resolution);

        match collapsed_by_current.get_mut(&current_id) {
            Some(existing) => {
                existing.retrieved.score = existing.retrieved.score.max(retrieved.score);
                existing.retrieved.similarity =
                    existing.retrieved.similarity.max(retrieved.similarity);
                existing.retrieved.text_match_score = max_option_f32(
                    existing.retrieved.text_match_score,
                    retrieved.text_match_score,
                );
                if existing.retrieved.highlights.is_none() {
                    existing.retrieved.highlights = retrieved.highlights.clone();
                }
                if existing.retrieved.evidence.is_none() {
                    existing.retrieved.evidence = retrieved.evidence.clone();
                }
                if existing.retrieved.history.is_none() {
                    existing.retrieved.history = retrieved.history.clone();
                }
                if !existing
                    .merged_lineage_sources
                    .iter()
                    .any(|item| item.source_memory_id == source.source_memory_id)
                {
                    existing.merged_lineage_sources.push(source);
                }
            }
            None => {
                order.push(current_id);

                let mut current_retrieved = retrieved.clone();
                current_retrieved.memory = resolution.current_memory.clone();
                current_retrieved.history =
                    merge_history_with_lineage(current_retrieved.history, &resolution);

                collapsed_by_current.insert(
                    current_id,
                    CollapsedRetrievedMemory {
                        retrieved: current_retrieved,
                        merged_lineage_sources: vec![source],
                    },
                );
            }
        }
    }

    let mut collapsed: Vec<CollapsedRetrievedMemory> = order
        .into_iter()
        .filter_map(|id| collapsed_by_current.remove(&id))
        .collect();

    collapsed.sort_by(|a, b| {
        b.retrieved
            .score
            .partial_cmp(&a.retrieved.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(collapsed)
}

fn lineage_source_response(resolution: &CurrentMemoryResolution) -> LineageSourceResponse {
    LineageSourceResponse {
        source_memory_id: resolution.requested_memory.id.to_string(),
        lineage_to_current: resolution
            .lineage_to_current
            .iter()
            .map(|memory| memory.id.to_string())
            .collect(),
    }
}

fn merge_history_with_lineage(
    history: Option<MemoryHistory>,
    resolution: &CurrentMemoryResolution,
) -> Option<MemoryHistory> {
    history.or_else(|| {
        Some(MemoryHistory {
            versions: resolution.lineage_to_current.iter().cloned().collect(),
            current_version: Some(resolution.current_memory.clone()),
        })
    })
}

fn max_option_f32(left: Option<f32>, right: Option<f32>) -> Option<f32> {
    match (left, right) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// POST /api/v1/systems/{profile_id}/memories/retrieve/auto - Auto retrieve with query parsing
///
/// Automatically retrieves memories by:
/// 1. For memories with embedding_provider: vector search + fulltext search
/// 2. For memories without embedding_provider: fulltext search only
/// 3. Merges results and returns formatted markdown output
#[utoipa::path(
    post,
    path = "/api/v1/systems/{profile_id}/memories/retrieve/auto",
    tag = "retrieval",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID")
    ),
    request_body = AutoRetrieveApiRequest,
    responses(
        (status = 200, description = "Memories retrieved successfully", body = AutoRetrieveApiResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn auto_retrieve_memories(
    State(state): State<AppState>,
    Path(profile_id): Path<Uuid>,
    Json(request): Json<AutoRetrieveApiRequest>,
) -> AppResult<Json<AutoRetrieveApiResponse>> {
    use crate::embedding::EmbeddingRequest;
    use crate::repository::VectorFilter;
    use tracing::{debug, warn};

    let request_body = request;

    debug!(
        profile_id = %profile_id,
        query = %request_body.query,
        owner_id = %request_body.owner_id,
        scope_id = ?request_body.scope_id,
        "auto_retrieve_memories: starting request"
    );

    let include_global = true;
    let top_k = request_body.top_k.unwrap_or(10);
    let mut all_similarities: Vec<(uuid::Uuid, f32)> = Vec::new();

    // Use global embedding provider for query embedding
    let embedding_provider_name = &state.embedding_provider_name;

    // Generate query embedding using global provider
    let embedding_request = EmbeddingRequest::new(&request_body.query);
    match state.embedding_provider.embed(embedding_request).await {
        Ok(query_embedding) => {
            let filter = VectorFilter {
                profile_id: Some(profile_id),
                owner_id: Some(request_body.owner_id.clone()),
                scope_id: request_body.scope_id.clone(),
                include_global: Some(include_global),
                statuses: Some(vec!["active".to_string()]),
                ..Default::default()
            };

            // Search in the global provider's Qdrant collection
            let search_results = match state
                .qdrant_repo
                .search(
                    embedding_provider_name,
                    query_embedding.embedding,
                    top_k * 2,
                    Some(filter),
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
    let vector_candidate_count = all_similarities.len();
    let retrieve_request = RetrieveRequest {
        profile_id,
        query: request_body.query.clone(),
        owner_id: request_body.owner_id.clone(),
        scope_id: request_body.scope_id.clone(),
        top_k: request_body.top_k,
        min_score: request_body.min_score,
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
                metadata: rm.memory.metadata.clone(),
                infer: rm.memory.content.clone(),
                score: rm.score,
            }
        })
        .collect();

    // Generate markdown output
    let markdown = format_records_as_markdown(&records);

    // Use the original query as parsed intent (could be enhanced with LLM parsing later)
    let parsed_intent = if let Some(ctx) = &request_body.context {
        format!("{} (context: {})", request_body.query, ctx)
    } else {
        request_body.query.clone()
    };

    let response = AutoRetrieveApiResponse {
        parsed_intent,
        records,
        total_candidates: result.total_candidates,
        trace: RetrievalTraceResponse {
            query: request_body.query,
            used_vector: has_vector_results,
            used_fulltext: true,
            scope_id: request_body.scope_id,
            vector_candidate_count,
            structured_candidate_count: result.total_candidates,
            final_result_count: result.memories.len(),
        },
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
        output.push_str(&format!("metadata: {}\n", record.metadata));
        output.push_str(&format!("infer: {}\n", record.infer));
        output.push_str(&format!("score: {:.2}\n", record.score));
        output.push('\n');
    }
    output.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retrieve_request_conversion() {
        let profile_id = Uuid::new_v4();
        let api_request = RetrieveApiRequest {
            query: "test query".to_string(),
            owner_id: "owner123".to_string(),
            scope_id: Some("scope456".to_string()),
            top_k: Some(20),
            options: RetrieveOptions {
                min_score: Some(0.5),
                min_confidence: Some(0.7),
                use_fulltext: Some(true),
                use_vector: Some(true),
                fulltext_weight: Some(0.2),
                highlight: Some(true),
                include_evidence: Some(true),
                include_history: Some(false),
            },
        };

        let retrieve_request = api_request.clone().into_service_request(profile_id);

        assert_eq!(retrieve_request.profile_id, profile_id);
        assert_eq!(retrieve_request.query, api_request.query);
        assert_eq!(retrieve_request.owner_id, api_request.owner_id);
        assert_eq!(retrieve_request.scope_id, api_request.scope_id);
        assert_eq!(retrieve_request.top_k, api_request.top_k);
        assert_eq!(retrieve_request.min_score, api_request.options.min_score);
        assert_eq!(
            retrieve_request.min_confidence,
            api_request.options.min_confidence
        );
        assert_eq!(
            retrieve_request.use_fulltext,
            api_request.options.use_fulltext
        );
        assert_eq!(retrieve_request.use_vector, api_request.options.use_vector);
        assert_eq!(
            retrieve_request.fulltext_weight,
            api_request.options.fulltext_weight
        );
        assert_eq!(retrieve_request.highlight, api_request.options.highlight);
        assert_eq!(
            retrieve_request.include_evidence,
            api_request.options.include_evidence
        );
        assert_eq!(
            retrieve_request.include_history,
            api_request.options.include_history
        );
    }

    #[test]
    fn test_retrieve_request_minimal() {
        let profile_id = Uuid::new_v4();
        let api_request = RetrieveApiRequest {
            query: "simple query".to_string(),
            owner_id: "owner123".to_string(),
            scope_id: None,
            top_k: None,
            options: RetrieveOptions::default(),
        };

        let retrieve_request = api_request.into_service_request(profile_id);

        assert_eq!(retrieve_request.profile_id, profile_id);
        assert_eq!(retrieve_request.query, "simple query");
        assert_eq!(retrieve_request.owner_id, "owner123");
        assert!(retrieve_request.scope_id.is_none());
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
                metadata: serde_json::json!({
                    "memory_type": "ui_preference",
                    "theme": "dark",
                    "channel": "explicit"
                }),
                infer: "用户喜欢深色模式".to_string(),
                score: 0.95,
            },
            AutoRetrieveRecord {
                record: 2,
                time: Utc::now(),
                metadata: serde_json::json!({}),
                infer: "用户偏好简洁界面".to_string(),
                score: 0.82,
            },
        ];

        let markdown = format_records_as_markdown(&records);
        assert!(markdown.contains("record: 1"));
        assert!(markdown.contains("record: 2"));
        assert!(markdown.contains(
            "metadata: {\"channel\":\"explicit\",\"memory_type\":\"ui_preference\",\"theme\":\"dark\"}"
        ));
        assert!(markdown.contains("infer: 用户喜欢深色模式"));
        assert!(markdown.contains("score: 0.95"));
    }

    #[test]
    fn test_retrieved_memory_response_serializes_lineage_sources() {
        let response = RetrievedMemoryResponse {
            memory: GetMemoryResponse {
                id: Uuid::new_v4(),
                owner_id: "owner123".to_string(),
                scope_id: Some("scope456".to_string()),
                content: "用户更喜欢 Rust".to_string(),
                metadata: serde_json::json!({
                    "memory_type": "language_preference",
                    "language": "Rust"
                }),
                schema_version: 1,
                importance: 0.9,
                confidence: 0.9,
                root_memory_id: Some(Uuid::new_v4()),
                version_number: 2,
                is_current_version: true,
                supersedes: Some(Uuid::new_v4()),
                superseded_by: None,
                is_global: false,
                hit_count: 0,
                last_hit_at: None,
                reinforcement_count: 0,
                last_reinforced_at: None,
                decay_score: 1.0,
                source_event_id: None,
                status: crate::domain::Status::Active,
                embedding_status: crate::domain::EmbeddingStatus::Completed,
                embedding_provider: Some("openai".to_string()),
                processing_status: crate::domain::ProcessingStatus::Completed,
                llm_provider: Some("openai".to_string()),
                conflict_reason: None,
                consistency_confidence: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            score: 0.91,
            similarity: 0.88,
            text_match_score: Some(0.77),
            highlights: None,
            evidence: None,
            history: None,
            merged_lineage_sources: Some(vec![LineageSourceResponse {
                source_memory_id: "old-memory-id".to_string(),
                lineage_to_current: vec![
                    "old-memory-id".to_string(),
                    "current-memory-id".to_string(),
                ],
            }]),
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(
            json["merged_lineage_sources"][0]["source_memory_id"],
            "old-memory-id"
        );
        assert_eq!(
            json["merged_lineage_sources"][0]["lineage_to_current"][1],
            "current-memory-id"
        );
    }
}
