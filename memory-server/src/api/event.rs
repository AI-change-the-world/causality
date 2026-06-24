//! Event Processing API handlers
//!
//! Implements:
//! - POST /api/v1/systems/{profile_id}/events - Create and process an event
//! - GET /api/v1/systems/{profile_id}/events/{id} - Get event details
//!
//! New architecture:
//! - Events are immutable after creation
//! - Returns memories_created and memories_reinforced counts
//! - Supports scope_id and source fields
//! - Uses global LLM and embedding providers configured for the service

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::AppState;
use crate::domain::{EventSource, ProcessingStatus};
use crate::error::AppResult;
use crate::service::{CreateFromEventRequest, EventIngestionResult};

/// Request body for creating an event
///
/// This endpoint accepts raw event content and uses LLM to extract
/// structured memories. The LLM automatically understands the event type
/// and extracts relevant facts, preferences, patterns, and rules.
///
/// LLM and Embedding providers are configured globally in config.yaml,
/// no need to specify them per request.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateEventApiRequest {
    /// Owner ID - the unique identifier of the memory owner
    pub owner_id: String,
    /// Event content (any form: click description, conversation history, operation log, etc.)
    pub content: String,
    /// Optional context to help LLM better understand the event
    #[serde(default)]
    pub context: Option<String>,
    /// Scope identifier for created memories (optional, null = global context)
    #[serde(default)]
    pub scope_id: Option<String>,
    /// Event source. Recommended values: conversation, user_action, system_event, manual, api.
    #[serde(default)]
    pub source: Option<EventSource>,
}

impl CreateEventApiRequest {
    fn into_service_request(self, profile_id: Uuid) -> CreateFromEventRequest {
        CreateFromEventRequest {
            profile_id,
            owner_id: self.owner_id,
            content: self.content,
            context: self.context,
            scope_id: self.scope_id,
            source: self.source,
        }
    }
}

impl From<EventIngestionResult> for CreateEventApiResponse {
    fn from(result: EventIngestionResult) -> Self {
        CreateEventApiResponse {
            event_id: result.event.id,
            processing_status: result.event.processing_status,
            error_message: result.event.error_message,
            processed_at: result.event.processed_at,
            event_summary: result.event.summary,
            analysis_payload: result.event.analysis_payload,
            analysis_schema_version: result.event.analysis_schema_version,
            memories_created: result.memories_created,
            memories_reinforced: result.memories_reinforced,
            memories_superseded: result.memories_superseded,
            created_memory_ids: result.created_memory_ids,
            reinforced_memory_ids: result.reinforced_memory_ids,
            superseded_memory_ids: result.superseded_memory_ids,
            skipped: result.skipped,
            skip_reason: result.skip_reason,
            relevance_score: result.relevance_score,
        }
    }
}

/// Response for creating an event
///
/// Contains the event details and memory processing results.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateEventApiResponse {
    /// The created event ID
    pub event_id: Uuid,
    /// Current processing status
    pub processing_status: ProcessingStatus,
    /// Error message when processing failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// When processing reached a terminal status
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed_at: Option<DateTime<Utc>>,
    /// Event summary (LLM-generated understanding of the event)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_summary: Option<String>,
    /// Profile-aware event analysis payload
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_payload: Option<Value>,
    /// Schema version used to generate the analysis payload
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_schema_version: Option<i32>,
    /// Number of memories created from this event
    pub memories_created: usize,
    /// Number of existing memories reinforced by this event
    pub memories_reinforced: usize,
    /// Number of memories superseded (new versions created)
    pub memories_superseded: usize,
    /// IDs of created memories
    pub created_memory_ids: Vec<Uuid>,
    /// IDs of reinforced memories
    pub reinforced_memory_ids: Vec<Uuid>,
    /// IDs of superseded memories (old versions)
    pub superseded_memory_ids: Vec<Uuid>,
    /// Whether the event was skipped due to irrelevance
    pub skipped: bool,
    /// Reason for skipping (if skipped)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    /// Relevance score used by the skip decision
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevance_score: Option<f32>,
}

/// Response for getting an event
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GetEventApiResponse {
    /// Event ID
    pub id: Uuid,
    /// Owner ID
    pub owner_id: String,
    /// Scope ID (null = global context)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    /// Event content
    pub content: String,
    /// Event context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Event summary
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Profile-aware event analysis payload
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_payload: Option<Value>,
    /// Schema version used to generate the analysis payload
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_schema_version: Option<i32>,
    /// Event source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<EventSource>,
    /// Current processing status
    pub processing_status: ProcessingStatus,
    /// Error message when processing failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// When processing reached a terminal status
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed_at: Option<DateTime<Utc>>,
    /// Whether the event was skipped due to irrelevance
    pub skipped: bool,
    /// Reason for skipping (if skipped)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    /// Relevance score used by the skip decision
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevance_score: Option<f32>,
    /// When the event occurred
    pub event_time: DateTime<Utc>,
    /// When the event was created
    pub created_at: DateTime<Utc>,
    /// Related memories (created from or reinforced by this event)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_memories: Option<Vec<RelatedMemoryResponse>>,
}

/// A memory related to an event
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RelatedMemoryResponse {
    /// Memory ID
    pub memory_id: Uuid,
    /// Relation type (created_from or reinforced_by)
    pub relation_type: String,
    /// Memory content preview
    pub content_preview: String,
}

/// Create event processing routes
pub fn event_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(create_event))
        .route("/{id}", get(get_event))
        .route("/{id}/retry", post(retry_event))
}

/// Create async event processing routes
pub fn event_async_routes() -> Router<AppState> {
    Router::new().route("/", post(create_event_async))
}

/// Response for async event creation
///
/// Returns immediately with event_id. Use GET /api/v1/systems/{profile_id}/events/{id} to check status.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateEventAsyncResponse {
    /// The created event ID
    pub event_id: Uuid,
    /// Event processing status
    pub status: ProcessingStatus,
    /// Message indicating how to check status
    pub message: String,
}

/// POST /api/v1/systems/{profile_id}/events-async - Create an event and process asynchronously
///
/// Creates an event and starts background processing. Returns immediately with event_id.
/// Use GET /api/v1/systems/{profile_id}/events/{id} to check processing status and results.
///
/// This is useful for long-running LLM processing to avoid HTTP timeouts.
#[utoipa::path(
    post,
    path = "/api/v1/systems/{profile_id}/events-async",
    tag = "events",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID")
    ),
    request_body = CreateEventApiRequest,
    responses(
        (status = 202, description = "Event created, processing started", body = CreateEventAsyncResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn create_event_async(
    Path(profile_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<CreateEventApiRequest>,
) -> AppResult<(StatusCode, Json<CreateEventAsyncResponse>)> {
    tracing::info!(
        owner_id = %request.owner_id,
        "create_event_async: starting async request"
    );

    let event = state
        .event_ingestion_service
        .create_pending_event(request.into_service_request(profile_id))
        .await?;
    let event_id = event.id;

    // Clone what we need for the background task
    let state_clone = state.clone();

    // Spawn background task for processing
    tokio::spawn(async move {
        tracing::info!(
            event_id = %event_id,
            "Background task: starting event processing"
        );

        if let Err(e) = process_event_background(state_clone, profile_id, event_id).await {
            tracing::error!(
                event_id = %event_id,
                error = %e,
                "Background task: event processing failed"
            );
        } else {
            tracing::info!(
                event_id = %event_id,
                "Background task: event processing completed"
            );
        }
    });

    let response = CreateEventAsyncResponse {
        event_id,
        status: event.processing_status,
        message: format!(
            "Event created. Check status at GET /api/v1/systems/{}/events/{}",
            profile_id, event_id
        ),
    };

    Ok((StatusCode::ACCEPTED, Json(response)))
}

/// Background task to process an event
/// Uses global LLM and Embedding providers from AppState (configured in config.yaml)
async fn process_event_background(
    state: AppState,
    profile_id: Uuid,
    event_id: Uuid,
) -> Result<(), crate::error::AppError> {
    state
        .event_ingestion_service
        .process_existing_event(profile_id, event_id)
        .await?;

    Ok(())
}

/// POST /api/v1/systems/{profile_id}/events/{id}/retry - Retry a failed event
#[utoipa::path(
    post,
    path = "/api/v1/systems/{profile_id}/events/{id}/retry",
    tag = "events",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID"),
        ("id" = Uuid, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Event retried", body = CreateEventApiResponse),
        (status = 400, description = "Event is not retryable", body = crate::error::ErrorResponse),
        (status = 404, description = "Event not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn retry_event(
    State(state): State<AppState>,
    Path((profile_id, id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<CreateEventApiResponse>> {
    let result = state
        .event_ingestion_service
        .retry_event(profile_id, id)
        .await?;

    Ok(Json(CreateEventApiResponse::from(result)))
}

/// POST /api/v1/systems/{profile_id}/events - Create and process an event
///
/// Creates an event and extracts memories from raw event content using LLM processing.
/// Returns the event details along with memory processing results.
///
/// The handler uses global LLM and Embedding providers configured in config.yaml.
#[utoipa::path(
    post,
    path = "/api/v1/systems/{profile_id}/events",
    tag = "events",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID")
    ),
    request_body = CreateEventApiRequest,
    responses(
        (status = 201, description = "Event created and processed successfully", body = CreateEventApiResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn create_event(
    Path(profile_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<CreateEventApiRequest>,
) -> AppResult<(StatusCode, Json<CreateEventApiResponse>)> {
    tracing::info!(
        owner_id = %request.owner_id,
        "create_event: starting request"
    );

    let result = state
        .event_ingestion_service
        .ingest_event_sync(request.into_service_request(profile_id))
        .await?;

    let status = if result.skipped {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    let response = CreateEventApiResponse::from(result);

    Ok((status, Json(response)))
}

/// GET /api/v1/systems/{profile_id}/events/{id} - Get event details
///
/// Returns the event details including related memories.
#[utoipa::path(
    get,
    path = "/api/v1/systems/{profile_id}/events/{id}",
    tag = "events",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID"),
        ("id" = Uuid, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Event found", body = GetEventApiResponse),
        (status = 404, description = "Event not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_event(
    State(state): State<AppState>,
    Path((profile_id, id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<GetEventApiResponse>> {
    // Get event from repository
    let event = state.retrieval_engine.get_event(id).await?;
    if event.profile_id != profile_id {
        return Err(crate::error::AppError::Validation(
            "event does not belong to the requested system profile".to_string(),
        ));
    }

    // Get related memories
    let relations = state
        .retrieval_engine
        .get_event_memory_relations(id)
        .await?;
    let related_memories: Vec<RelatedMemoryResponse> = relations
        .into_iter()
        .map(|(memory, relation_type)| RelatedMemoryResponse {
            memory_id: memory.id,
            relation_type: relation_type.to_string(),
            content_preview: memory.content.chars().take(100).collect::<String>()
                + if memory.content.len() > 100 {
                    "..."
                } else {
                    ""
                },
        })
        .collect();

    let response = GetEventApiResponse {
        id: event.id,
        owner_id: event.owner_id,
        scope_id: event.scope_id,
        content: event.content,
        context: event.context,
        summary: event.summary,
        analysis_payload: event.analysis_payload,
        analysis_schema_version: event.analysis_schema_version,
        source: event.source,
        processing_status: event.processing_status,
        error_message: event.error_message,
        processed_at: event.processed_at,
        skipped: event.skipped,
        skip_reason: event.skip_reason,
        relevance_score: event.relevance_score,
        event_time: event.event_time,
        created_at: event.created_at,
        related_memories: if related_memories.is_empty() {
            None
        } else {
            Some(related_memories)
        },
    };

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_event_request_conversion() {
        let profile_id = Uuid::new_v4();
        let api_request = CreateEventApiRequest {
            owner_id: "owner123".to_string(),
            content: "User clicked the dark mode button".to_string(),
            context: Some("Settings page".to_string()),
            scope_id: Some("user123".to_string()),
            source: Some(EventSource::Api),
        };

        let service_request = api_request.clone().into_service_request(profile_id);

        assert_eq!(service_request.profile_id, profile_id);
        assert_eq!(service_request.owner_id, api_request.owner_id);
        assert_eq!(service_request.content, api_request.content);
        assert_eq!(service_request.context, api_request.context);
        assert_eq!(service_request.scope_id, api_request.scope_id);
        assert_eq!(service_request.source, api_request.source);
    }

    #[test]
    fn test_create_event_request_defaults() {
        let json = r#"{
            "owner_id": "owner123",
            "content": "Test event content"
        }"#;

        let api_request: CreateEventApiRequest = serde_json::from_str(json).unwrap();

        assert_eq!(api_request.owner_id, "owner123");
        assert_eq!(api_request.content, "Test event content");
        assert!(api_request.context.is_none());
        assert!(api_request.scope_id.is_none());
        assert!(api_request.source.is_none());
    }

    #[test]
    fn test_create_event_request_with_all_fields() {
        let json = r#"{
            "owner_id": "owner123",
            "content": "Test event content",
            "context": "Test context",
            "scope_id": "scope456",
            "source": "api"
        }"#;

        let api_request: CreateEventApiRequest = serde_json::from_str(json).unwrap();

        assert_eq!(api_request.owner_id, "owner123");
        assert_eq!(api_request.content, "Test event content");
        assert_eq!(api_request.context, Some("Test context".to_string()));
        assert_eq!(api_request.scope_id, Some("scope456".to_string()));
        assert_eq!(api_request.source, Some(EventSource::Api));
    }

    #[test]
    fn test_create_event_request_rejects_unknown_source() {
        let json = r#"{
            "owner_id": "owner123",
            "content": "Test event content",
            "source": "user_created"
        }"#;

        let result = serde_json::from_str::<CreateEventApiRequest>(json);

        assert!(result.is_err());
    }

    #[test]
    fn test_create_event_response_serialization() {
        let id = Uuid::new_v4();
        let response = CreateEventApiResponse {
            skip_reason: None,
            relevance_score: None,
            skipped: false,
            event_id: id,
            processing_status: ProcessingStatus::Completed,
            error_message: None,
            processed_at: None,
            event_summary: Some("User changed theme settings".to_string()),
            analysis_payload: Some(serde_json::json!({
                "event_type": "preference_update"
            })),
            analysis_schema_version: Some(1),
            memories_created: 2,
            memories_reinforced: 1,
            memories_superseded: 0,
            created_memory_ids: vec![Uuid::new_v4()],
            reinforced_memory_ids: vec![Uuid::new_v4()],
            superseded_memory_ids: vec![],
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains(&id.to_string()));
        assert!(json.contains("\"processing_status\":\"completed\""));
        assert!(json.contains("\"memories_created\":2"));
        assert!(json.contains("\"memories_reinforced\":1"));
    }

    #[test]
    fn test_create_event_response_without_summary() {
        let response = CreateEventApiResponse {
            skip_reason: None,
            relevance_score: None,
            skipped: false,
            event_id: Uuid::new_v4(),
            processing_status: ProcessingStatus::Pending,
            error_message: None,
            processed_at: None,
            event_summary: None,
            analysis_payload: None,
            analysis_schema_version: None,
            memories_created: 0,
            memories_reinforced: 0,
            memories_superseded: 0,
            created_memory_ids: vec![],
            reinforced_memory_ids: vec![],
            superseded_memory_ids: vec![],
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("event_summary")); // Should be skipped when None
    }

    #[test]
    fn test_get_event_response_serialization() {
        let response = GetEventApiResponse {
            id: Uuid::new_v4(),
            owner_id: "owner123".to_string(),
            scope_id: Some("scope456".to_string()),
            content: "Test event content".to_string(),
            context: Some("Test context".to_string()),
            summary: Some("Test summary".to_string()),
            analysis_payload: Some(serde_json::json!({
                "event_type": "test"
            })),
            analysis_schema_version: Some(1),
            source: Some(EventSource::Api),
            processing_status: ProcessingStatus::Completed,
            error_message: None,
            processed_at: None,
            skipped: false,
            skip_reason: None,
            relevance_score: None,
            event_time: Utc::now(),
            created_at: Utc::now(),
            related_memories: None,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"owner_id\":\"owner123\""));
        assert!(json.contains("\"processing_status\":\"completed\""));
        assert!(!json.contains("related_memories")); // Should be skipped when None
    }

    #[test]
    fn test_related_memory_response() {
        let response = RelatedMemoryResponse {
            memory_id: Uuid::new_v4(),
            relation_type: "created_from".to_string(),
            content_preview: "User prefers dark mode...".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"relation_type\":\"created_from\""));
        assert!(json.contains("\"content_preview\":\"User prefers dark mode...\""));
    }
}
