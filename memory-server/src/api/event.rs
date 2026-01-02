//! Event Processing API handlers
//!
//! Implements:
//! - POST /api/v1/memories/from-event - Create memories from event extraction

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::AppState;
use crate::domain::{InferenceType, MemoryCategory, ProcessingMode, ScopeType};
use crate::error::AppResult;
use crate::service::CreateFromEventRequest;

/// Request body for creating memories from an event
///
/// This endpoint accepts raw event content and uses LLM to extract
/// structured memories. The LLM automatically understands the event type
/// and extracts relevant facts, preferences, patterns, and rules.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateFromEventApiRequest {
    /// Owner ID - the unique identifier of the memory owner
    pub owner_id: String,
    /// Event content (any form: click description, conversation history, operation log, etc.)
    pub content: String,
    /// Optional context to help LLM better understand the event
    #[serde(default)]
    pub context: Option<String>,
    /// Processing mode (auto, assisted, manual)
    /// - auto: Extract and create memories without confirmation
    /// - assisted: Return proposed memories for user approval (default)
    /// - manual: Only summarize events without extraction
    #[serde(default)]
    pub mode: Option<ProcessingMode>,
    /// Scope type for created memories (user, org, project, task, session)
    pub scope_type: ScopeType,
    /// Scope identifier for created memories
    pub scope_id: String,
    /// Usage scene for created memories (e.g., "work.contract_review")
    pub scene: String,
}

impl From<CreateFromEventApiRequest> for CreateFromEventRequest {
    fn from(req: CreateFromEventApiRequest) -> Self {
        CreateFromEventRequest {
            owner_id: req.owner_id,
            content: req.content,
            context: req.context,
            scope_type: req.scope_type,
            scope_id: req.scope_id,
            scene: req.scene,
            mode: req.mode,
        }
    }
}

/// Response for a single extracted memory
///
/// Contains all the information about a memory extracted from an event,
/// including the inference type, confidence, and reasoning.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ExtractedMemoryResponse {
    /// The extracted memory content
    pub content: String,
    /// Type of inference made (fact, preference, pattern, rule)
    pub inference_type: InferenceType,
    /// Confidence score for this extraction (0.0 - 1.0)
    pub confidence: f32,
    /// Auto-classified category
    pub category: MemoryCategory,
    /// Auto-extracted tags/keywords
    pub tags: Vec<String>,
    /// Importance score (0.0 - 1.0)
    pub importance: f32,
    /// Reasoning explaining why this memory was extracted
    pub reasoning: String,
}

/// Response for creating memories from an event
///
/// Contains the event summary, list of extracted memories, and optionally
/// the IDs of created memories (only in "auto" mode).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateFromEventApiResponse {
    /// Event summary (LLM-generated understanding of the event)
    pub event_summary: String,
    /// List of extracted memories with their details
    pub extracted_memories: Vec<ExtractedMemoryResponse>,
    /// IDs of created memories (only populated in "auto" mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_memory_ids: Option<Vec<Uuid>>,
}

/// Create event processing routes
pub fn event_routes() -> Router<AppState> {
    Router::new().route("/from-event", post(create_from_event))
}

/// POST /api/v1/memories/from-event - Create memories from event extraction
///
/// Extracts memories from raw event content using LLM processing.
/// The behavior depends on the processing mode:
/// - auto: Extract and create memories without confirmation
/// - assisted: Return proposed memories for user approval (default)
/// - manual: Only summarize events without extraction
#[utoipa::path(
    post,
    path = "/api/v1/memories/from-event",
    tag = "memories",
    request_body = CreateFromEventApiRequest,
    responses(
        (status = 200, description = "Memories extracted successfully", body = CreateFromEventApiResponse),
        (status = 201, description = "Memories created successfully (auto mode)", body = CreateFromEventApiResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn create_from_event(
    State(state): State<AppState>,
    Json(request): Json<CreateFromEventApiRequest>,
) -> AppResult<(StatusCode, Json<CreateFromEventApiResponse>)> {
    let mode = request.mode;
    let service_request: CreateFromEventRequest = request.into();

    let result = state
        .memory_guard
        .create_from_event(service_request, None)
        .await?;

    // Convert extracted memories to response format
    let extracted_memories: Vec<ExtractedMemoryResponse> = result
        .extracted_memories
        .into_iter()
        .map(|m| ExtractedMemoryResponse {
            content: m.content,
            inference_type: m.inference_type,
            confidence: m.confidence,
            category: m.category,
            tags: m.tags,
            importance: m.importance,
            reasoning: m.reasoning,
        })
        .collect();

    let response = CreateFromEventApiResponse {
        event_summary: result.event_summary,
        extracted_memories,
        created_memory_ids: result.created_memory_ids,
    };

    // Return 201 Created if memories were created (auto mode), otherwise 200 OK
    let status = if mode == Some(crate::domain::ProcessingMode::Auto) {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    Ok((status, Json(response)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_from_event_request_conversion() {
        let api_request = CreateFromEventApiRequest {
            owner_id: "owner123".to_string(),
            content: "User clicked the dark mode button".to_string(),
            context: Some("Settings page".to_string()),
            mode: Some(ProcessingMode::Auto),
            scope_type: ScopeType::User,
            scope_id: "user123".to_string(),
            scene: "settings.preferences".to_string(),
        };

        let service_request: CreateFromEventRequest = api_request.clone().into();

        assert_eq!(service_request.owner_id, api_request.owner_id);
        assert_eq!(service_request.content, api_request.content);
        assert_eq!(service_request.context, api_request.context);
        assert_eq!(service_request.mode, api_request.mode);
        assert_eq!(service_request.scope_type, api_request.scope_type);
        assert_eq!(service_request.scope_id, api_request.scope_id);
        assert_eq!(service_request.scene, api_request.scene);
    }

    #[test]
    fn test_create_from_event_request_defaults() {
        let json = r#"{
            "owner_id": "owner123",
            "content": "Test event content",
            "scope_type": "user",
            "scope_id": "user123",
            "scene": "test.scene"
        }"#;

        let api_request: CreateFromEventApiRequest = serde_json::from_str(json).unwrap();

        assert_eq!(api_request.owner_id, "owner123");
        assert_eq!(api_request.content, "Test event content");
        assert!(api_request.context.is_none());
        assert!(api_request.mode.is_none()); // Will default to Assisted in service layer
        assert_eq!(api_request.scope_type, ScopeType::User);
        assert_eq!(api_request.scope_id, "user123");
        assert_eq!(api_request.scene, "test.scene");
    }

    #[test]
    fn test_extracted_memory_response_serialization() {
        let response = ExtractedMemoryResponse {
            content: "User prefers dark mode".to_string(),
            inference_type: InferenceType::Preference,
            confidence: 0.85,
            category: MemoryCategory::UserPreference,
            tags: vec!["dark_mode".to_string(), "ui".to_string()],
            importance: 0.7,
            reasoning: "User explicitly clicked dark mode button".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"inference_type\":\"preference\""));
        assert!(json.contains("\"category\":\"user_preference\""));
        assert!(json.contains("\"confidence\":0.85"));
    }

    #[test]
    fn test_create_from_event_response_serialization() {
        let response = CreateFromEventApiResponse {
            event_summary: "User changed theme settings".to_string(),
            extracted_memories: vec![ExtractedMemoryResponse {
                content: "User prefers dark mode".to_string(),
                inference_type: InferenceType::Preference,
                confidence: 0.85,
                category: MemoryCategory::UserPreference,
                tags: vec!["dark_mode".to_string()],
                importance: 0.7,
                reasoning: "Clicked dark mode button".to_string(),
            }],
            created_memory_ids: None,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"event_summary\":\"User changed theme settings\""));
        assert!(!json.contains("created_memory_ids")); // Should be skipped when None
    }

    #[test]
    fn test_create_from_event_response_with_created_ids() {
        let id = Uuid::new_v4();
        let response = CreateFromEventApiResponse {
            event_summary: "User changed theme settings".to_string(),
            extracted_memories: vec![],
            created_memory_ids: Some(vec![id]),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("created_memory_ids"));
        assert!(json.contains(&id.to_string()));
    }
}
