//! Event Processing API handlers
//!
//! Implements:
//! - POST /api/v1/events - Create and process an event, extracting memories
//! - GET /api/v1/events/{id} - Get event details with related memories
//!
//! New architecture:
//! - Events are immutable after creation
//! - Returns memories_created and memories_reinforced counts
//! - Supports scope_id and source fields
//! - Dynamically creates embedding providers based on existing memories

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::AppState;
use crate::embedding::EmbeddingProvider;
use crate::error::AppResult;
use crate::service::{CreateFromEventRequest, EventProcessingContext};

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
    /// Event source (e.g., "user_created", "api", "conversation")
    #[serde(default)]
    pub source: Option<String>,
}

impl From<CreateEventApiRequest> for CreateFromEventRequest {
    fn from(req: CreateEventApiRequest) -> Self {
        CreateFromEventRequest {
            owner_id: req.owner_id,
            content: req.content,
            context: req.context,
            scope_id: req.scope_id,
            source: req.source,
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
    /// Whether the event has been processed
    pub processed: bool,
    /// Event summary (LLM-generated understanding of the event)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_summary: Option<String>,
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
    /// Event source
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Whether the event has been processed
    pub processed: bool,
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
}

/// Create async event processing routes
pub fn event_async_routes() -> Router<AppState> {
    Router::new().route("/", post(create_event_async))
}

/// Response for async event creation
///
/// Returns immediately with event_id. Use GET /api/v1/events/{id} to check status.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateEventAsyncResponse {
    /// The created event ID
    pub event_id: Uuid,
    /// Event status (always "pending" for async creation)
    pub status: String,
    /// Message indicating how to check status
    pub message: String,
}

/// POST /api/v1/events-async - Create an event and process asynchronously
///
/// Creates an event and starts background processing. Returns immediately with event_id.
/// Use GET /api/v1/events/{id} to check processing status and results.
///
/// This is useful for long-running LLM processing to avoid HTTP timeouts.
#[utoipa::path(
    post,
    path = "/api/v1/events-async",
    tag = "events",
    request_body = CreateEventApiRequest,
    responses(
        (status = 202, description = "Event created, processing started", body = CreateEventAsyncResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn create_event_async(
    State(state): State<AppState>,
    Json(request): Json<CreateEventApiRequest>,
) -> AppResult<(StatusCode, Json<CreateEventAsyncResponse>)> {
    tracing::info!(
        owner_id = %request.owner_id,
        "create_event_async: starting async request"
    );

    // Create the event first (unprocessed)
    let event_input = crate::domain::CreateEventInput {
        owner_id: request.owner_id.clone(),
        scope_id: request.scope_id.clone(),
        content: request.content.clone(),
        context: request.context.clone(),
        source: request.source.clone(),
        event_time: None,
    };

    let event = crate::domain::Event::new(event_input);
    let event_id = event.id;

    // Store the event (unprocessed)
    state
        .memory_guard
        .event_repo()
        .create(&event)
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("Failed to create event: {}", e)))?;

    // Clone what we need for the background task
    let state_clone = state.clone();

    // Spawn background task for processing
    tokio::spawn(async move {
        tracing::info!(
            event_id = %event_id,
            "Background task: starting event processing"
        );

        if let Err(e) = process_event_background(state_clone, event_id).await {
            tracing::error!(
                event_id = %event_id,
                error = %e,
                "Background task: event processing failed"
            );
            // TODO: Could update event with error status if we add that field
        } else {
            tracing::info!(
                event_id = %event_id,
                "Background task: event processing completed"
            );
        }
    });

    let response = CreateEventAsyncResponse {
        event_id,
        status: "pending".to_string(),
        message: format!(
            "Event created. Check status at GET /api/v1/events/{}",
            event_id
        ),
    };

    Ok((StatusCode::ACCEPTED, Json(response)))
}

/// Background task to process an event
/// Uses global LLM and Embedding providers from AppState (configured in config.yaml)
async fn process_event_background(
    state: AppState,
    event_id: Uuid,
) -> Result<(), crate::error::AppError> {
    // Get the event we created
    let event = state.retrieval_engine.get_event(event_id).await?;

    // Use global LLM provider from AppState
    let llm_provider = state.llm_provider.clone();

    let memory_processor = Arc::new(crate::service::MemoryProcessor::new(llm_provider.clone()));

    // Create EventHandler with ProfileService
    let event_handler = Some(Arc::new(
        crate::service::EventHandler::with_profile_service(
            llm_provider.clone(),
            state.profile_service.clone(),
        ),
    ));

    // Use global Embedding provider from AppState
    let embedding_provider = state.embedding_provider.clone();
    let embedding_provider_name = state.embedding_provider_name.clone();

    // Get Qdrant repository
    let qdrant_repo = state.qdrant_repo.clone();

    // Empty map - we only use the global embedding provider now
    let embedding_providers = HashMap::new();

    // Build context and process
    let context = EventProcessingContext {
        memory_processor,
        event_handler,
        llm_provider,
        embedding_provider,
        embedding_provider_name,
        qdrant_repo,
        embedding_providers,
    };

    state
        .memory_guard
        .process_event_with_context(&event, context, None)
        .await?;

    Ok(())
}

/// POST /api/v1/events - Create and process an event
///
/// Creates an event and extracts memories from raw event content using LLM processing.
/// Returns the event details along with memory processing results.
///
/// The handler uses global LLM and Embedding providers configured in config.yaml.
#[utoipa::path(
    post,
    path = "/api/v1/events",
    tag = "events",
    request_body = CreateEventApiRequest,
    responses(
        (status = 201, description = "Event created and processed successfully", body = CreateEventApiResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn create_event(
    State(state): State<AppState>,
    Json(request): Json<CreateEventApiRequest>,
) -> AppResult<(StatusCode, Json<CreateEventApiResponse>)> {
    tracing::info!(
        owner_id = %request.owner_id,
        "create_event: starting request"
    );

    // Use global LLM provider from AppState
    debug!("create_event: using global LLM provider from AppState");
    let llm_provider = state.llm_provider.clone();

    // Create MemoryProcessor with the global LLM provider
    let memory_processor = Arc::new(crate::service::MemoryProcessor::new(llm_provider.clone()));
    debug!("create_event: MemoryProcessor created");

    // Create EventHandler with ProfileService
    debug!("create_event: creating EventHandler with ProfileService");
    let event_handler = Some(Arc::new(
        crate::service::EventHandler::with_profile_service(
            llm_provider.clone(),
            state.profile_service.clone(),
        ),
    ));

    // Use global Embedding provider from AppState
    debug!("create_event: using global Embedding provider from AppState");
    let embedding_provider = state.embedding_provider.clone();
    let embedding_provider_name = state.embedding_provider_name.clone();

    // Query existing memories (for potential future use, but we only use global provider now)
    debug!("create_event: querying existing memories by provider");
    let memories_by_provider = state
        .memory_guard
        .memory_repo()
        .find_by_owner_scope_grouped_by_provider(
            &request.owner_id,
            request.scope_id.as_deref(),
            true, // include_global
        )
        .await?;

    debug!(
        provider_count = memories_by_provider.len(),
        "create_event: found existing memories grouped by provider"
    );

    // Empty map - we only use the global embedding provider now
    let embedding_providers: HashMap<String, Arc<dyn EmbeddingProvider>> = HashMap::new();

    // Get Qdrant repository
    debug!("create_event: getting Qdrant repository");
    let qdrant_repo = state.qdrant_repo.clone();
    debug!("create_event: got Qdrant repository");

    // Build EventProcessingContext with global providers
    debug!("create_event: building EventProcessingContext");

    let context = EventProcessingContext {
        memory_processor,
        event_handler,
        llm_provider,
        embedding_provider,
        embedding_provider_name,
        qdrant_repo,
        embedding_providers,
    };

    let service_request: CreateFromEventRequest = request.into();
    debug!("create_event: calling create_from_event_with_context");

    let result = state
        .memory_guard
        .create_from_event_with_context(service_request, context, None)
        .await?;

    let response = CreateEventApiResponse {
        event_id: result.event.id,
        processed: result.event.processed,
        event_summary: result.event.summary,
        memories_created: result.memories_created,
        memories_reinforced: result.memories_reinforced,
        memories_superseded: result.memories_superseded,
        created_memory_ids: result.created_memory_ids.clone(),
        reinforced_memory_ids: vec![], // TODO: Add to CreateFromEventResult if needed
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// GET /api/v1/events/{id} - Get event details
///
/// Returns the event details including related memories.
#[utoipa::path(
    get,
    path = "/api/v1/events/{id}",
    tag = "events",
    params(
        ("id" = Uuid, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Event found", body = GetEventApiResponse),
        (status = 404, description = "Event not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_event(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<GetEventApiResponse>> {
    // Get event from repository
    let event = state.retrieval_engine.get_event(id).await?;

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
        source: event.source,
        processed: event.processed,
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
        let api_request = CreateEventApiRequest {
            owner_id: "owner123".to_string(),
            content: "User clicked the dark mode button".to_string(),
            context: Some("Settings page".to_string()),
            scope_id: Some("user123".to_string()),
            source: Some("api".to_string()),
        };

        let service_request: CreateFromEventRequest = api_request.clone().into();

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
        assert_eq!(api_request.source, Some("api".to_string()));
    }

    #[test]
    fn test_create_event_response_serialization() {
        let id = Uuid::new_v4();
        let response = CreateEventApiResponse {
            event_id: id,
            processed: true,
            event_summary: Some("User changed theme settings".to_string()),
            memories_created: 2,
            memories_reinforced: 1,
            memories_superseded: 0,
            created_memory_ids: vec![Uuid::new_v4()],
            reinforced_memory_ids: vec![Uuid::new_v4()],
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains(&id.to_string()));
        assert!(json.contains("\"processed\":true"));
        assert!(json.contains("\"memories_created\":2"));
        assert!(json.contains("\"memories_reinforced\":1"));
    }

    #[test]
    fn test_create_event_response_without_summary() {
        let response = CreateEventApiResponse {
            event_id: Uuid::new_v4(),
            processed: false,
            event_summary: None,
            memories_created: 0,
            memories_reinforced: 0,
            memories_superseded: 0,
            created_memory_ids: vec![],
            reinforced_memory_ids: vec![],
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
            source: Some("api".to_string()),
            processed: true,
            event_time: Utc::now(),
            created_at: Utc::now(),
            related_memories: None,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"owner_id\":\"owner123\""));
        assert!(json.contains("\"processed\":true"));
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
