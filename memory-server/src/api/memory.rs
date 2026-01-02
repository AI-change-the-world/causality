//! Memory CRUD API handlers
//!
//! Implements:
//! - POST /api/v1/memories - Create a new memory
//! - GET /api/v1/memories/{id} - Get a memory by ID
//! - PUT /api/v1/memories/{id} - Update a memory
//! - DELETE /api/v1/memories/{id} - Delete (archive) a memory

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
use crate::domain::{
    CreateMemoryInput, EmbeddingStatus, Layer, Memory, MemoryCategory, ProcessingStatus, ScopeType,
    Status, UpdateMode,
};
use crate::error::AppResult;
use crate::service::UpdateMemoryRequest;

/// Request body for creating a new memory
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateMemoryApiRequest {
    /// Memory layer (session or task - long_term is rejected)
    pub layer: Layer,
    /// Scope type (user, org, project, task, session)
    pub scope_type: ScopeType,
    /// Scope identifier
    pub scope_id: String,
    /// Usage scene (e.g., "work.contract_review")
    pub scene: String,
    /// Memory content (Markdown format)
    pub content: String,
    /// Importance score (0.0 - 1.0), defaults to 0.5
    pub importance: Option<f32>,
    /// Confidence score (0.0 - 1.0), defaults to 1.0
    pub confidence: Option<f32>,
    /// Time-to-live in seconds
    pub ttl_seconds: Option<i64>,
    /// Event source that triggered memory creation
    pub event_source: Option<String>,
    /// Timestamp when the triggering event occurred
    pub event_time: Option<DateTime<Utc>>,
    /// Embedding provider to use (uses default if not specified)
    pub embedding_provider: Option<String>,
    /// Whether to process content with LLM (compression, classification, tag extraction)
    #[serde(default)]
    pub process_with_llm: bool,
    /// LLM provider to use for processing (uses default if not specified)
    pub llm_provider: Option<String>,
}

impl From<CreateMemoryApiRequest> for CreateMemoryInput {
    fn from(req: CreateMemoryApiRequest) -> Self {
        CreateMemoryInput {
            layer: req.layer,
            scope_type: req.scope_type,
            scope_id: req.scope_id,
            scene: req.scene,
            content: req.content,
            importance: req.importance,
            confidence: req.confidence,
            ttl_seconds: req.ttl_seconds,
            event_source: req.event_source,
            event_time: req.event_time,
            embedding_provider: req.embedding_provider,
            process_with_llm: req.process_with_llm,
            llm_provider: req.llm_provider,
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
    /// Memory category (if LLM processing was enabled)
    pub category: Option<MemoryCategory>,
}

/// Response for getting a memory
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GetMemoryResponse {
    pub id: Uuid,
    pub layer: Layer,
    pub scope_type: ScopeType,
    pub scope_id: String,
    pub scene: String,
    pub status: Status,
    pub content: String,
    pub raw_content: Option<String>,
    pub category: Option<MemoryCategory>,
    pub tags: Option<Vec<String>>,
    pub importance: f32,
    pub confidence: f32,
    pub hit_count: i64,
    pub last_hit_at: Option<DateTime<Utc>>,
    pub ttl_seconds: Option<i64>,
    pub expires_at: Option<DateTime<Utc>>,
    pub event_source: Option<String>,
    pub event_time: Option<DateTime<Utc>>,
    pub embedding_status: EmbeddingStatus,
    pub embedding_provider: Option<String>,
    pub processing_status: ProcessingStatus,
    pub llm_provider: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Memory> for GetMemoryResponse {
    fn from(m: Memory) -> Self {
        GetMemoryResponse {
            id: m.id,
            layer: m.layer,
            scope_type: m.scope_type,
            scope_id: m.scope_id,
            scene: m.scene,
            status: m.status,
            content: m.content,
            raw_content: m.raw_content,
            category: m.category,
            tags: m.tags,
            importance: m.importance,
            confidence: m.confidence,
            hit_count: m.hit_count,
            last_hit_at: m.last_hit_at,
            ttl_seconds: m.ttl_seconds,
            expires_at: m.expires_at,
            event_source: m.event_source,
            event_time: m.event_time,
            embedding_status: m.embedding_status,
            embedding_provider: m.embedding_provider,
            processing_status: m.processing_status,
            llm_provider: m.llm_provider,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// Request body for updating a memory
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateMemoryApiRequest {
    /// Update mode (append, merge, supersede)
    pub mode: UpdateMode,
    /// New content (optional)
    pub content: Option<String>,
    /// New importance value (optional)
    pub importance: Option<f32>,
    /// New confidence value (optional)
    pub confidence: Option<f32>,
    /// New TTL in seconds (optional)
    pub ttl_seconds: Option<i64>,
    /// Whether to process updated content with LLM
    #[serde(default)]
    pub process_with_llm: bool,
}

impl From<UpdateMemoryApiRequest> for UpdateMemoryRequest {
    fn from(req: UpdateMemoryApiRequest) -> Self {
        UpdateMemoryRequest {
            mode: req.mode,
            content: req.content,
            importance: req.importance,
            confidence: req.confidence,
            ttl_seconds: req.ttl_seconds,
            process_with_llm: req.process_with_llm,
        }
    }
}

/// Create memory routes
pub fn memory_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(create_memory))
        .route("/{id}", get(get_memory))
        .route("/{id}", put(update_memory))
        .route("/{id}", delete(delete_memory))
}

/// POST /api/v1/memories - Create a new memory
///
/// Creates a new memory with the specified parameters.
/// Long-term layer is rejected - requires manual confirmation.
#[utoipa::path(
    post,
    path = "/api/v1/memories",
    tag = "memories",
    request_body = CreateMemoryApiRequest,
    responses(
        (status = 201, description = "Memory created successfully", body = CreateMemoryResponse),
        (status = 400, description = "Invalid request (e.g., long-term layer)", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn create_memory(
    State(state): State<AppState>,
    Json(request): Json<CreateMemoryApiRequest>,
) -> AppResult<(StatusCode, Json<CreateMemoryResponse>)> {
    let input: CreateMemoryInput = request.into();

    let memory = state.memory_guard.create_memory(input, None).await?;

    let response = CreateMemoryResponse {
        id: memory.id,
        created_at: memory.created_at,
        embedding_status: memory.embedding_status,
        processing_status: memory.processing_status,
        category: memory.category,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// GET /api/v1/memories/{id} - Get a memory by ID
///
/// Returns the complete memory record including all metadata.
#[utoipa::path(
    get,
    path = "/api/v1/memories/{id}",
    tag = "memories",
    params(
        ("id" = Uuid, Path, description = "Memory ID")
    ),
    responses(
        (status = 200, description = "Memory found", body = GetMemoryResponse),
        (status = 404, description = "Memory not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_memory(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<GetMemoryResponse>> {
    let memory = state.memory_guard.get_memory(id).await?;
    Ok(Json(memory.into()))
}

/// PUT /api/v1/memories/{id} - Update a memory
///
/// Updates a memory with the specified mode:
/// - append: Appends new content to existing content
/// - merge: Merges new content with existing content (with separator)
/// - supersede: Replaces existing content with new content
#[utoipa::path(
    put,
    path = "/api/v1/memories/{id}",
    tag = "memories",
    params(
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
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateMemoryApiRequest>,
) -> AppResult<Json<GetMemoryResponse>> {
    let update_request: UpdateMemoryRequest = request.into();

    let memory = state
        .memory_guard
        .update_memory(id, update_request, None)
        .await?;
    Ok(Json(memory.into()))
}

/// DELETE /api/v1/memories/{id} - Delete (archive) a memory
///
/// Performs soft delete by setting status to 'archived'.
/// Content is preserved for audit purposes.
#[utoipa::path(
    delete,
    path = "/api/v1/memories/{id}",
    tag = "memories",
    params(
        ("id" = Uuid, Path, description = "Memory ID")
    ),
    responses(
        (status = 200, description = "Memory archived", body = GetMemoryResponse),
        (status = 404, description = "Memory not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn delete_memory(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<GetMemoryResponse>> {
    let memory = state.memory_guard.delete_memory(id, None).await?;
    Ok(Json(memory.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_memory_request_conversion() {
        let api_request = CreateMemoryApiRequest {
            layer: Layer::Session,
            scope_type: ScopeType::User,
            scope_id: "user123".to_string(),
            scene: "test.scene".to_string(),
            content: "Test content".to_string(),
            importance: Some(0.8),
            confidence: Some(0.9),
            ttl_seconds: Some(3600),
            event_source: Some("button_click:like".to_string()),
            event_time: Some(Utc::now()),
            embedding_provider: Some("openai".to_string()),
            process_with_llm: false,
            llm_provider: None,
        };

        let input: CreateMemoryInput = api_request.clone().into();

        assert_eq!(input.layer, api_request.layer);
        assert_eq!(input.scope_type, api_request.scope_type);
        assert_eq!(input.scope_id, api_request.scope_id);
        assert_eq!(input.scene, api_request.scene);
        assert_eq!(input.content, api_request.content);
        assert_eq!(input.importance, api_request.importance);
        assert_eq!(input.confidence, api_request.confidence);
        assert_eq!(input.ttl_seconds, api_request.ttl_seconds);
        assert_eq!(input.event_source, api_request.event_source);
        assert_eq!(input.embedding_provider, api_request.embedding_provider);
        assert_eq!(input.process_with_llm, api_request.process_with_llm);
        assert_eq!(input.llm_provider, api_request.llm_provider);
    }

    #[test]
    fn test_update_memory_request_conversion() {
        let api_request = UpdateMemoryApiRequest {
            mode: UpdateMode::Append,
            content: Some("New content".to_string()),
            importance: Some(0.7),
            confidence: Some(0.85),
            ttl_seconds: Some(7200),
            process_with_llm: false,
        };

        let update_request: UpdateMemoryRequest = api_request.clone().into();

        assert_eq!(update_request.mode, api_request.mode);
        assert_eq!(update_request.content, api_request.content);
        assert_eq!(update_request.importance, api_request.importance);
        assert_eq!(update_request.confidence, api_request.confidence);
        assert_eq!(update_request.ttl_seconds, api_request.ttl_seconds);
        assert_eq!(
            update_request.process_with_llm,
            api_request.process_with_llm
        );
    }

    #[test]
    fn test_memory_to_response_conversion() {
        let memory = Memory {
            id: Uuid::new_v4(),
            layer: Layer::Task,
            scope_type: ScopeType::Project,
            scope_id: "project456".to_string(),
            scene: "work.review".to_string(),
            status: Status::Active,
            content: "Test memory content".to_string(),
            raw_content: None,
            category: Some(MemoryCategory::UserPreference),
            tags: Some(vec!["tag1".to_string(), "tag2".to_string()]),
            importance: 0.75,
            confidence: 0.95,
            hit_count: 5,
            last_hit_at: Some(Utc::now()),
            ttl_seconds: Some(604800),
            expires_at: Some(Utc::now()),
            event_source: Some("conversation:preference".to_string()),
            event_time: Some(Utc::now()),
            embedding_status: EmbeddingStatus::Completed,
            embedding_provider: Some("openai".to_string()),
            processing_status: ProcessingStatus::Completed,
            llm_provider: Some("openai".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let response: GetMemoryResponse = memory.clone().into();

        assert_eq!(response.id, memory.id);
        assert_eq!(response.layer, memory.layer);
        assert_eq!(response.scope_type, memory.scope_type);
        assert_eq!(response.scope_id, memory.scope_id);
        assert_eq!(response.scene, memory.scene);
        assert_eq!(response.status, memory.status);
        assert_eq!(response.content, memory.content);
        assert_eq!(response.raw_content, memory.raw_content);
        assert_eq!(response.category, memory.category);
        assert_eq!(response.tags, memory.tags);
        assert_eq!(response.importance, memory.importance);
        assert_eq!(response.confidence, memory.confidence);
        assert_eq!(response.hit_count, memory.hit_count);
        assert_eq!(response.event_source, memory.event_source);
        assert_eq!(response.embedding_status, memory.embedding_status);
        assert_eq!(response.processing_status, memory.processing_status);
        assert_eq!(response.llm_provider, memory.llm_provider);
        assert_eq!(response.embedding_provider, memory.embedding_provider);
    }
}
