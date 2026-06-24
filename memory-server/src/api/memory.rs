//! Memory CRUD API handlers
//!
//! Implements:
//! - POST /api/v1/systems/{profile_id}/memories - Create a new memory
//! - GET /api/v1/systems/{profile_id}/memories/{id} - Get a memory by ID
//! - PUT /api/v1/systems/{profile_id}/memories/{id} - Update a memory
//! - DELETE /api/v1/systems/{profile_id}/memories/{id} - Archive a memory
//!
//! New architecture:
//! - Removed layer, scope_type, scene, ttl_seconds, expires_at, event_source, event_time
//! - Added category (hierarchical string), is_global
//! - Added version chain fields
//! - Memory content is immutable after creation

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::AppState;
use crate::domain::{CreateMemoryInput, EmbeddingStatus, Memory, ProcessingStatus, Status};
use crate::embedding::EmbeddingRequest;
use crate::error::AppResult;
use crate::service::UpdateMemoryRequest;

/// Request body for creating a new memory
///
/// This endpoint is for direct memory creation where the user provides all fields.
/// For LLM-assisted memory extraction from events, use POST
/// /api/v1/systems/{profile_id}/events instead.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateMemoryApiRequest {
    /// Owner ID - the unique identifier of the memory owner
    pub owner_id: String,
    /// Scope identifier (null = global memory)
    pub scope_id: Option<String>,
    /// Memory content (Markdown format)
    pub content: String,
    /// Hierarchical category (e.g., "work.code.eslint")
    pub category: Option<String>,
    /// Tags/keywords
    pub tags: Option<Vec<String>>,
    /// Importance score (0.0 - 1.0), defaults to 0.5
    pub importance: Option<f32>,
    /// Confidence score (0.0 - 1.0), defaults to 1.0
    pub confidence: Option<f32>,
    /// Whether this is a global memory
    #[serde(default)]
    pub is_global: bool,
    /// Embedding provider to use (uses default if not specified)
    pub embedding_provider: Option<String>,
}

impl CreateMemoryApiRequest {
    fn into_domain_input(self, profile_id: Uuid) -> CreateMemoryInput {
        CreateMemoryInput {
            profile_id,
            owner_id: self.owner_id,
            scope_id: self.scope_id,
            content: self.content,
            category: self.category,
            tags: self.tags,
            importance: self.importance,
            confidence: self.confidence,
            is_global: self.is_global,
            embedding_provider: self.embedding_provider,
            // Direct creation never uses LLM processing
            process_with_llm: false,
            llm_provider: None,
        }
    }
}

/// Response for memory creation
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateMemoryResponse {
    /// Unique memory ID
    pub id: Uuid,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Embedding generation status
    pub embedding_status: EmbeddingStatus,
    /// LLM processing status
    pub processing_status: ProcessingStatus,
    /// Hierarchical category
    pub category: Option<String>,
}

/// Response for getting a memory
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GetMemoryResponse {
    pub id: Uuid,
    pub owner_id: String,
    pub scope_id: Option<String>,
    pub content: String,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub importance: f32,
    pub confidence: f32,
    // Version chain
    pub root_memory_id: Option<Uuid>,
    pub version_number: i32,
    pub is_current_version: bool,
    pub supersedes: Option<Uuid>,
    pub superseded_by: Option<Uuid>,
    // Lifecycle
    pub is_global: bool,
    pub hit_count: i64,
    pub last_hit_at: Option<DateTime<Utc>>,
    pub reinforcement_count: i64,
    pub last_reinforced_at: Option<DateTime<Utc>>,
    pub decay_score: f32,
    // Source
    pub source_event_id: Option<Uuid>,
    // Status
    pub status: Status,
    // Processing
    pub embedding_status: EmbeddingStatus,
    pub embedding_provider: Option<String>,
    pub processing_status: ProcessingStatus,
    pub llm_provider: Option<String>,
    // Inference / reconciliation metadata
    pub conflict_reason: Option<String>,
    pub consistency_confidence: Option<f32>,
    // Timestamps
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Memory> for GetMemoryResponse {
    fn from(m: Memory) -> Self {
        GetMemoryResponse {
            id: m.id,
            owner_id: m.owner_id,
            scope_id: m.scope_id,
            content: m.content,
            category: m.category,
            tags: m.tags,
            importance: m.importance,
            confidence: m.confidence,
            root_memory_id: m.root_memory_id,
            version_number: m.version_number,
            is_current_version: m.is_current_version,
            supersedes: m.supersedes,
            superseded_by: m.superseded_by,
            is_global: m.is_global,
            hit_count: m.hit_count,
            last_hit_at: m.last_hit_at,
            reinforcement_count: m.reinforcement_count,
            last_reinforced_at: m.last_reinforced_at,
            decay_score: m.decay_score,
            source_event_id: m.source_event_id,
            status: m.status,
            embedding_status: m.embedding_status,
            embedding_provider: m.embedding_provider,
            processing_status: m.processing_status,
            llm_provider: m.llm_provider,
            conflict_reason: m.conflict_reason,
            consistency_confidence: m.consistency_confidence,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// Request body for updating a memory (metadata only)
///
/// In the new architecture, memory content is immutable.
/// Only metadata fields can be updated.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateMemoryApiRequest {
    /// New confidence value (optional)
    pub confidence: Option<f32>,
    /// New decay score (optional)
    pub decay_score: Option<f32>,
}

impl From<UpdateMemoryApiRequest> for UpdateMemoryRequest {
    fn from(req: UpdateMemoryApiRequest) -> Self {
        UpdateMemoryRequest {
            confidence: req.confidence,
            decay_score: req.decay_score,
        }
    }
}

/// Response for memory version history
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemoryHistoryResponse {
    /// Root memory ID (the first version in the chain)
    pub root_memory_id: Uuid,
    /// Requested memory ID
    pub requested_memory_id: Uuid,
    /// Current version ID
    pub current_version_id: Option<Uuid>,
    /// Zero-based position of the requested memory in the version chain
    pub requested_index: Option<usize>,
    /// Previous version in the chain, if any
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_version_id: Option<Uuid>,
    /// Next version that superseded the requested memory, if any
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_version_id: Option<Uuid>,
    /// Number of supersede hops from the requested memory to current
    pub hops_to_current: usize,
    /// Total number of versions
    pub total_versions: usize,
    /// Path from the requested memory to the current version (inclusive)
    pub lineage_to_current: Vec<Uuid>,
    /// All versions ordered by version_number
    pub versions: Vec<MemoryVersionResponse>,
}

/// Response for resolving any memory version to the latest current version.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ResolveCurrentMemoryResponse {
    /// Requested memory ID
    pub requested_memory_id: Uuid,
    /// Latest current memory
    pub current_memory: GetMemoryResponse,
    /// Number of supersede hops from requested to current
    pub hops_to_current: usize,
    /// Inclusive path of memory IDs from requested to current
    pub lineage_to_current: Vec<Uuid>,
}

/// A single version in the memory history
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemoryVersionResponse {
    /// Memory ID
    pub id: Uuid,
    /// Version number (1-indexed)
    pub version_number: i32,
    /// Whether this is the current version
    pub is_current_version: bool,
    /// Memory content
    pub content: String,
    /// Category
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Tags
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Importance score
    pub importance: f32,
    /// Confidence score
    pub confidence: f32,
    /// Status
    pub status: Status,
    /// ID of the memory this version supersedes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<Uuid>,
    /// ID of the memory that superseded this version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<Uuid>,
    /// Source event ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_event_id: Option<Uuid>,
    /// Source event content (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_event_content: Option<String>,
    /// Source event time (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_event_time: Option<DateTime<Utc>>,
    /// Created at
    pub created_at: DateTime<Utc>,
}

/// Request body for promoting a memory to global status
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PromoteMemoryRequest {
    /// Reason for promotion (required for audit trail)
    pub reason: String,
}

/// Response for memory promotion
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PromoteMemoryResponse {
    /// The promoted memory
    pub memory: GetMemoryResponse,
    /// Whether the memory was already global
    pub was_already_global: bool,
    /// Promotion timestamp
    pub promoted_at: DateTime<Utc>,
}

/// Create memory routes
pub fn memory_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(create_memory))
        .route("/{id}", get(get_memory))
        .route("/{id}", put(update_memory))
        .route("/{id}", delete(delete_memory))
        .route("/{id}/history", get(get_memory_history))
        .route("/{id}/resolve-current", get(resolve_current_memory))
        .route("/{id}/promote", post(promote_memory))
}

/// POST /api/v1/systems/{profile_id}/memories - Create a new memory
///
/// Creates a new memory with the specified parameters.
/// If an embedding provider is configured, the memory content will be embedded
/// and stored in the vector database for semantic search.
#[utoipa::path(
    post,
    path = "/api/v1/systems/{profile_id}/memories",
    tag = "memories",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID")
    ),
    request_body = CreateMemoryApiRequest,
    responses(
        (status = 201, description = "Memory created successfully", body = CreateMemoryResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn create_memory(
    State(state): State<AppState>,
    Path(profile_id): Path<Uuid>,
    Json(request): Json<CreateMemoryApiRequest>,
) -> AppResult<(StatusCode, Json<CreateMemoryResponse>)> {
    let input = request.into_domain_input(profile_id);

    let mut memory = state.memory_guard.create_memory(input, None).await?;

    let payload = vector_payload(&memory);

    let embedding_provider_name = state.embedding_provider_name.clone();

    // Generate embedding using global provider
    let embedding_request = EmbeddingRequest::new(&memory.content);
    match state.embedding_provider.embed(embedding_request).await {
        Ok(embedding_response) => {
            // Store in Qdrant
            match state
                .qdrant_repo
                .upsert_vector(
                    &embedding_provider_name,
                    memory.id,
                    embedding_response.embedding,
                    payload,
                )
                .await
            {
                Ok(_) => {
                    // Update embedding status in database
                    memory = state
                        .memory_guard
                        .memory_repo()
                        .update_embedding_status_and_provider(
                            memory.id,
                            EmbeddingStatus::Completed,
                            Some(&embedding_provider_name),
                        )
                        .await?;
                }
                Err(e) => {
                    tracing::warn!(
                        memory_id = %memory.id,
                        error = %e,
                        "Failed to store embedding in Qdrant"
                    );
                    memory = state
                        .memory_guard
                        .memory_repo()
                        .update_embedding_status_and_provider(
                            memory.id,
                            EmbeddingStatus::Failed,
                            None,
                        )
                        .await?;
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                memory_id = %memory.id,
                error = %e,
                "Failed to generate embedding, memory created without vector"
            );
            // Update embedding status to failed in database
            memory = state
                .memory_guard
                .memory_repo()
                .update_embedding_status_and_provider(memory.id, EmbeddingStatus::Failed, None)
                .await?;
        }
    }

    let response = CreateMemoryResponse {
        id: memory.id,
        created_at: memory.created_at,
        embedding_status: memory.embedding_status,
        processing_status: memory.processing_status,
        category: memory.category,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// GET /api/v1/systems/{profile_id}/memories/{id} - Get a memory by ID
///
/// Returns the complete memory record including all metadata.
#[utoipa::path(
    get,
    path = "/api/v1/systems/{profile_id}/memories/{id}",
    tag = "memories",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID"),
        ("id" = Uuid, Path, description = "Memory ID")
    ),
    responses(
        (status = 200, description = "Memory found", body = GetMemoryResponse),
        (status = 404, description = "Memory not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_memory(
    State(state): State<AppState>,
    Path((profile_id, id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<GetMemoryResponse>> {
    let memory = state.memory_guard.get_memory(id).await?;
    ensure_memory_profile(&memory, profile_id)?;
    Ok(Json(GetMemoryResponse::from(memory)))
}

/// PUT /api/v1/systems/{profile_id}/memories/{id} - Update a memory (metadata only)
///
/// Updates memory metadata. Content is immutable in the new architecture.
/// To change content, create a new version via event processing.
#[utoipa::path(
    put,
    path = "/api/v1/systems/{profile_id}/memories/{id}",
    tag = "memories",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID"),
        ("id" = Uuid, Path, description = "Memory ID")
    ),
    request_body = UpdateMemoryApiRequest,
    responses(
        (status = 200, description = "Memory updated", body = GetMemoryResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 404, description = "Memory not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn update_memory(
    State(state): State<AppState>,
    Path((profile_id, id)): Path<(Uuid, Uuid)>,
    Json(request): Json<UpdateMemoryApiRequest>,
) -> AppResult<Json<GetMemoryResponse>> {
    let update_request: UpdateMemoryRequest = request.into();

    let existing = state.memory_guard.get_memory(id).await?;
    ensure_memory_profile(&existing, profile_id)?;

    let memory = state
        .memory_guard
        .update_memory(id, update_request, None)
        .await?;
    Ok(Json(GetMemoryResponse::from(memory)))
}

/// DELETE /api/v1/systems/{profile_id}/memories/{id} - Delete (archive) a memory
///
/// Performs soft delete by setting status to 'archived'.
/// Content is preserved for audit purposes.
#[utoipa::path(
    delete,
    path = "/api/v1/systems/{profile_id}/memories/{id}",
    tag = "memories",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID"),
        ("id" = Uuid, Path, description = "Memory ID")
    ),
    responses(
        (status = 200, description = "Memory archived", body = GetMemoryResponse),
        (status = 404, description = "Memory not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn delete_memory(
    State(state): State<AppState>,
    Path((profile_id, id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<GetMemoryResponse>> {
    let existing = state.memory_guard.get_memory(id).await?;
    ensure_memory_profile(&existing, profile_id)?;

    let memory = state.memory_guard.delete_memory(id, None).await?;
    sync_archived_vector(&state, &memory).await;
    Ok(Json(GetMemoryResponse::from(memory)))
}

/// GET /api/v1/systems/{profile_id}/memories/{id}/history - Get memory version history
///
/// Returns the complete version history of a memory, including all versions,
/// source events, and the supersede lineage around the requested node.
#[utoipa::path(
    get,
    path = "/api/v1/systems/{profile_id}/memories/{id}/history",
    tag = "memories",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID"),
        ("id" = Uuid, Path, description = "Memory ID (any version in the chain)")
    ),
    responses(
        (status = 200, description = "Version history retrieved", body = MemoryHistoryResponse),
        (status = 404, description = "Memory not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_memory_history(
    State(state): State<AppState>,
    Path((profile_id, id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<MemoryHistoryResponse>> {
    let memory = state.memory_guard.get_memory(id).await?;
    ensure_memory_profile(&memory, profile_id)?;
    let root_id = memory.root_memory_id.unwrap_or(memory.id);
    let history = state.retrieval_engine.get_memory_history(root_id).await?;
    let current_version_id = history
        .versions
        .iter()
        .find(|v| v.is_current_version)
        .map(|v| v.id);
    let requested_index = history.versions.iter().position(|v| v.id == id);
    let previous_version_id =
        requested_index.and_then(|idx| idx.checked_sub(1).map(|prev| history.versions[prev].id));
    let next_version_id =
        requested_index.and_then(|idx| history.versions.get(idx + 1).map(|next| next.id));
    let lineage_to_current: Vec<Uuid> = requested_index
        .map(|idx| history.versions[idx..].iter().map(|version| version.id).collect())
        .unwrap_or_default();
    let hops_to_current = lineage_to_current.len().saturating_sub(1);

    let mut versions = Vec::with_capacity(history.versions.len());
    for version in history.versions {
        let (source_event_content, source_event_time) =
            if let Some(event_id) = version.source_event_id {
                match state.retrieval_engine.get_event(event_id).await {
                    Ok(event) => (Some(event.content), Some(event.event_time)),
                    Err(_) => (None, None),
                }
            } else {
                (None, None)
            };

        versions.push(MemoryVersionResponse {
            id: version.id,
            version_number: version.version_number,
            is_current_version: version.is_current_version,
            content: version.content,
            category: version.category,
            tags: version.tags,
            importance: version.importance,
            confidence: version.confidence,
            status: version.status,
            supersedes: version.supersedes,
            superseded_by: version.superseded_by,
            source_event_id: version.source_event_id,
            source_event_content,
            source_event_time,
            created_at: version.created_at,
        });
    }

    let response = MemoryHistoryResponse {
        root_memory_id: root_id,
        requested_memory_id: id,
        current_version_id,
        requested_index,
        previous_version_id,
        next_version_id,
        hops_to_current,
        total_versions: versions.len(),
        lineage_to_current,
        versions,
    };

    Ok(Json(response))
}

/// GET /api/v1/systems/{profile_id}/memories/{id}/resolve-current - Resolve to latest current version
///
/// Follows the supersede chain from the requested memory until the latest
/// current version is reached, returning the resolved memory directly.
#[utoipa::path(
    get,
    path = "/api/v1/systems/{profile_id}/memories/{id}/resolve-current",
    tag = "memories",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID"),
        ("id" = Uuid, Path, description = "Memory ID (any version in the chain)")
    ),
    responses(
        (status = 200, description = "Current memory resolved", body = ResolveCurrentMemoryResponse),
        (status = 404, description = "Memory not found", body = crate::error::ErrorResponse),
        (status = 500, description = "Broken supersede chain", body = crate::error::ErrorResponse)
    )
)]
pub async fn resolve_current_memory(
    State(state): State<AppState>,
    Path((profile_id, id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<ResolveCurrentMemoryResponse>> {
    let memory = state.memory_guard.get_memory(id).await?;
    ensure_memory_profile(&memory, profile_id)?;

    let resolution = state.retrieval_engine.resolve_to_current_memory(id).await?;
    ensure_memory_profile(&resolution.current_memory, profile_id)?;

    let response = ResolveCurrentMemoryResponse {
        requested_memory_id: id,
        current_memory: GetMemoryResponse::from(resolution.current_memory),
        hops_to_current: resolution.lineage_to_current.len().saturating_sub(1),
        lineage_to_current: resolution
            .lineage_to_current
            .into_iter()
            .map(|memory| memory.id)
            .collect(),
    };

    Ok(Json(response))
}

/// POST /api/v1/systems/{profile_id}/memories/{id}/promote - Promote memory to global status
///
/// Manually promotes a memory to global status, making it accessible
/// across all scopes for the same owner.
#[utoipa::path(
    post,
    path = "/api/v1/systems/{profile_id}/memories/{id}/promote",
    tag = "memories",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID"),
        ("id" = Uuid, Path, description = "Memory ID")
    ),
    request_body = PromoteMemoryRequest,
    responses(
        (status = 200, description = "Memory promoted to global", body = PromoteMemoryResponse),
        (status = 400, description = "Invalid request or memory already global", body = crate::error::ErrorResponse),
        (status = 404, description = "Memory not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn promote_memory(
    State(state): State<AppState>,
    Path((profile_id, id)): Path<(Uuid, Uuid)>,
    Json(request): Json<PromoteMemoryRequest>,
) -> AppResult<Json<PromoteMemoryResponse>> {
    // Get the memory first to check if it's already global
    let memory = state.memory_guard.get_memory(id).await?;
    ensure_memory_profile(&memory, profile_id)?;
    let was_already_global = memory.is_global;

    if was_already_global {
        // Return the memory as-is if already global
        let response = PromoteMemoryResponse {
            memory: GetMemoryResponse::from(memory.clone()),
            was_already_global: true,
            promoted_at: memory.promoted_at.unwrap_or_else(Utc::now),
        };
        return Ok(Json(response));
    }

    // Promote the memory using the global promoter
    let promoted = state
        .lifecycle_manager
        .promote_memory(id, &request.reason)
        .await?;
    sync_memory_payload(&state, &promoted).await;

    let response = PromoteMemoryResponse {
        memory: GetMemoryResponse::from(promoted.clone()),
        was_already_global: false,
        promoted_at: promoted.promoted_at.unwrap_or_else(Utc::now),
    };

    Ok(Json(response))
}

fn ensure_memory_profile(memory: &Memory, profile_id: Uuid) -> AppResult<()> {
    if memory.profile_id != profile_id {
        return Err(crate::error::AppError::Validation(
            "memory does not belong to the requested system profile".to_string(),
        ));
    }

    Ok(())
}

async fn sync_archived_vector(state: &AppState, memory: &Memory) {
    if let Some(provider) = &memory.embedding_provider {
        if let Err(e) = state.qdrant_repo.delete_vector(provider, memory.id).await {
            tracing::warn!(
                memory_id = %memory.id,
                provider = %provider,
                error = %e,
                "Failed to delete archived memory vector"
            );
        }
    }
}

async fn sync_memory_payload(state: &AppState, memory: &Memory) {
    if let Some(provider) = &memory.embedding_provider {
        let payload = vector_payload(memory);

        if let Err(e) = state
            .qdrant_repo
            .update_payload(provider, memory.id, payload)
            .await
        {
            tracing::warn!(
                memory_id = %memory.id,
                provider = %provider,
                error = %e,
                "Failed to update memory vector payload"
            );
        }
    }
}

pub(crate) fn vector_payload(memory: &Memory) -> crate::repository::VectorPayload {
    crate::repository::VectorPayload {
        profile_id: memory.profile_id,
        owner_id: memory.owner_id.clone(),
        memory_id: memory.id,
        scope_id: memory.scope_id.clone(),
        category: memory.category.clone(),
        is_global: memory.is_global,
        status: memory.status.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_memory_request_conversion() {
        let profile_id = Uuid::new_v4();
        let api_request = CreateMemoryApiRequest {
            owner_id: "owner123".to_string(),
            scope_id: Some("scope456".to_string()),
            content: "Test content".to_string(),
            category: Some("work.code".to_string()),
            tags: Some(vec!["tag1".to_string(), "tag2".to_string()]),
            importance: Some(0.8),
            confidence: Some(0.9),
            is_global: false,
            embedding_provider: Some("openai".to_string()),
        };

        let input = api_request.clone().into_domain_input(profile_id);

        assert_eq!(input.profile_id, profile_id);
        assert_eq!(input.owner_id, api_request.owner_id);
        assert_eq!(input.scope_id, api_request.scope_id);
        assert_eq!(input.content, api_request.content);
        assert_eq!(input.category, api_request.category);
        assert_eq!(input.tags, api_request.tags);
        assert_eq!(input.importance, api_request.importance);
        assert_eq!(input.confidence, api_request.confidence);
        assert_eq!(input.is_global, api_request.is_global);
        assert_eq!(input.embedding_provider, api_request.embedding_provider);
        // Direct creation never uses LLM processing
        assert!(!input.process_with_llm);
        assert!(input.llm_provider.is_none());
    }

    #[test]
    fn test_update_memory_request_conversion() {
        let api_request = UpdateMemoryApiRequest {
            confidence: Some(0.85),
            decay_score: Some(0.7),
        };

        let update_request: UpdateMemoryRequest = api_request.clone().into();

        assert_eq!(update_request.confidence, api_request.confidence);
        assert_eq!(update_request.decay_score, api_request.decay_score);
    }

    #[test]
    fn test_memory_to_response_conversion() {
        let memory = Memory {
            profile_id: Uuid::new_v4(),
            id: Uuid::new_v4(),
            owner_id: "owner123".to_string(),
            scope_id: Some("scope456".to_string()),
            content: "Test memory content".to_string(),
            category: Some("work.code".to_string()),
            tags: Some(vec!["tag1".to_string(), "tag2".to_string()]),
            importance: 0.75,
            confidence: 0.95,
            root_memory_id: None,
            version_number: 1,
            is_current_version: true,
            supersedes: None,
            superseded_by: None,
            is_global: false,
            hit_count: 5,
            last_hit_at: Some(Utc::now()),
            reinforcement_count: 2,
            last_reinforced_at: Some(Utc::now()),
            decay_score: 0.8,
            source_event_id: None,
            status: Status::Active,
            embedding_status: EmbeddingStatus::Completed,
            embedding_provider: Some("openai".to_string()),
            processing_status: ProcessingStatus::Completed,
            llm_provider: Some("openai".to_string()),
            inference_type: None,
            inference_confidence: None,
            inference_reasoning: None,
            conflict_reason: Some("conflict reason".to_string()),
            consistency_confidence: Some(0.87),
            promoted_at: None,
            promotion_reason: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let response: GetMemoryResponse = memory.clone().into();

        assert_eq!(response.id, memory.id);
        assert_eq!(response.owner_id, memory.owner_id);
        assert_eq!(response.scope_id, memory.scope_id);
        assert_eq!(response.content, memory.content);
        assert_eq!(response.category, memory.category);
        assert_eq!(response.tags, memory.tags);
        assert_eq!(response.importance, memory.importance);
        assert_eq!(response.confidence, memory.confidence);
        assert_eq!(response.root_memory_id, memory.root_memory_id);
        assert_eq!(response.version_number, memory.version_number);
        assert_eq!(response.is_current_version, memory.is_current_version);
        assert_eq!(response.is_global, memory.is_global);
        assert_eq!(response.hit_count, memory.hit_count);
        assert_eq!(response.reinforcement_count, memory.reinforcement_count);
        assert_eq!(response.last_reinforced_at, memory.last_reinforced_at);
        assert_eq!(response.decay_score, memory.decay_score);
        assert_eq!(response.conflict_reason, memory.conflict_reason);
        assert_eq!(
            response.consistency_confidence,
            memory.consistency_confidence
        );
        assert_eq!(response.status, memory.status);
        assert_eq!(response.embedding_status, memory.embedding_status);
        assert_eq!(response.processing_status, memory.processing_status);
    }

    #[test]
    fn test_history_response_serialization_includes_lineage_fields() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let current = Uuid::new_v4();

        let response = MemoryHistoryResponse {
            root_memory_id: first,
            requested_memory_id: second,
            current_version_id: Some(current),
            requested_index: Some(1),
            previous_version_id: Some(first),
            next_version_id: Some(current),
            hops_to_current: 1,
            total_versions: 3,
            lineage_to_current: vec![second, current],
            versions: vec![],
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["requested_memory_id"], second.to_string());
        assert_eq!(json["requested_index"], 1);
        assert_eq!(json["previous_version_id"], first.to_string());
        assert_eq!(json["next_version_id"], current.to_string());
        assert_eq!(json["hops_to_current"], 1);
        assert_eq!(json["lineage_to_current"][0], second.to_string());
        assert_eq!(json["lineage_to_current"][1], current.to_string());
    }

    #[test]
    fn test_resolve_current_response_serialization() {
        let requested = Uuid::new_v4();
        let current = Uuid::new_v4();

        let response = ResolveCurrentMemoryResponse {
            requested_memory_id: requested,
            current_memory: GetMemoryResponse {
                id: current,
                owner_id: "owner123".to_string(),
                scope_id: Some("scope456".to_string()),
                content: "current".to_string(),
                category: Some("work.code".to_string()),
                tags: None,
                importance: 0.8,
                confidence: 0.9,
                root_memory_id: Some(requested),
                version_number: 2,
                is_current_version: true,
                supersedes: Some(requested),
                superseded_by: None,
                is_global: false,
                hit_count: 0,
                last_hit_at: None,
                reinforcement_count: 0,
                last_reinforced_at: None,
                decay_score: 1.0,
                source_event_id: None,
                status: Status::Active,
                embedding_status: EmbeddingStatus::Completed,
                embedding_provider: Some("openai".to_string()),
                processing_status: ProcessingStatus::Completed,
                llm_provider: Some("openai".to_string()),
                conflict_reason: None,
                consistency_confidence: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            hops_to_current: 1,
            lineage_to_current: vec![requested, current],
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["requested_memory_id"], requested.to_string());
        assert_eq!(json["current_memory"]["id"], current.to_string());
        assert_eq!(json["hops_to_current"], 1);
        assert_eq!(json["lineage_to_current"][0], requested.to_string());
        assert_eq!(json["lineage_to_current"][1], current.to_string());
    }

    #[test]
    fn test_create_memory_request_minimal() {
        let json = r#"{
            "owner_id": "owner123",
            "content": "Test content"
        }"#;

        let api_request: CreateMemoryApiRequest = serde_json::from_str(json).unwrap();

        assert_eq!(api_request.owner_id, "owner123");
        assert_eq!(api_request.content, "Test content");
        assert!(api_request.scope_id.is_none());
        assert!(api_request.category.is_none());
        assert!(api_request.tags.is_none());
        assert!(api_request.importance.is_none());
        assert!(api_request.confidence.is_none());
        assert!(!api_request.is_global);
    }

    #[test]
    fn test_create_memory_request_global() {
        let json = r#"{
            "owner_id": "owner123",
            "content": "Global memory content",
            "is_global": true
        }"#;

        let api_request: CreateMemoryApiRequest = serde_json::from_str(json).unwrap();

        assert!(api_request.is_global);
        assert!(api_request.scope_id.is_none());
    }
}
