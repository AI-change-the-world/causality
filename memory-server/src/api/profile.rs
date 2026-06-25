//! Profile API handlers
//!
//! Implements:
//! - POST /api/v1/systems - Create a system profile namespace with LLM parsing
//! - GET /api/v1/systems - List system profiles
//! - GET /api/v1/systems/{profile_id} - Get a profile by ID
//! - PUT /api/v1/systems/{profile_id} - Update a profile by ID

use axum::{
    extract::State,
    http::StatusCode,
    response::Html,
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::AppState;
use crate::domain::{CreateProfileInput, SchemaStatus, UpdateProfileInput};
use crate::error::AppResult;

/// Request body for initializing the system profile
///
/// The description will be parsed by LLM to extract structured fields.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct InitializeProfileRequest {
    /// System name (max 100 characters)
    pub name: String,
    /// Natural language description of the system
    /// LLM will parse this to extract purpose, domain, target_audience, etc.
    pub description: String,
}

impl From<InitializeProfileRequest> for CreateProfileInput {
    fn from(req: InitializeProfileRequest) -> Self {
        CreateProfileInput {
            name: req.name,
            description: req.description,
        }
    }
}

/// Response for system profile operations
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProfileResponse {
    /// Unique identifier
    pub id: Uuid,
    /// System name
    pub name: String,
    /// Original user description
    pub description: String,
    /// System purpose (LLM parsed)
    pub purpose: String,
    /// Business domain (LLM parsed)
    pub domain: String,
    /// Target audience (LLM parsed)
    pub target_audience: String,
    /// Valid event categories (LLM parsed)
    pub event_categories: Vec<String>,
    /// Memory types to prioritize (LLM parsed)
    pub memory_focus: Vec<String>,
    /// Things the system should not handle (LLM parsed)
    pub boundaries: Vec<String>,
    /// Extraction prompt template (LLM generated)
    pub extraction_prompt: String,
    /// Generated metadata schema contract
    pub metadata_schema: Value,
    /// Schema lifecycle state
    pub schema_status: SchemaStatus,
    /// Schema version
    pub schema_version: i32,
    /// When schema was confirmed
    pub schema_confirmed_at: Option<DateTime<Utc>>,
    /// Prompt used to generate the schema proposal
    pub schema_generation_prompt: Option<String>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl From<crate::domain::SystemProfile> for ProfileResponse {
    fn from(profile: crate::domain::SystemProfile) -> Self {
        ProfileResponse {
            id: profile.id,
            name: profile.name,
            description: profile.description,
            purpose: profile.purpose,
            domain: profile.domain,
            target_audience: profile.target_audience,
            event_categories: profile.event_categories,
            memory_focus: profile.memory_focus,
            boundaries: profile.boundaries,
            extraction_prompt: profile.extraction_prompt,
            metadata_schema: profile.metadata_schema,
            schema_status: profile.schema_status,
            schema_version: profile.schema_version,
            schema_confirmed_at: profile.schema_confirmed_at,
            schema_generation_prompt: profile.schema_generation_prompt,
            created_at: profile.created_at,
            updated_at: profile.updated_at,
        }
    }
}

/// Response for schema confirmation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConfirmProfileSchemaResponse {
    pub profile: ProfileResponse,
}

/// Request body for updating the system profile
///
/// Supports two modes:
/// 1. Partial update: Only updates provided fields
/// 2. Re-parse: If `reparse=true` and description is provided, uses LLM to re-parse
#[derive(Debug, Clone, Deserialize, ToSchema, Default)]
pub struct UpdateProfileRequest {
    /// New system name (optional)
    #[serde(default)]
    pub name: Option<String>,
    /// New description (optional, triggers re-parse if reparse=true)
    #[serde(default)]
    pub description: Option<String>,
    /// New purpose (optional)
    #[serde(default)]
    pub purpose: Option<String>,
    /// New domain (optional)
    #[serde(default)]
    pub domain: Option<String>,
    /// New target audience (optional)
    #[serde(default)]
    pub target_audience: Option<String>,
    /// New event categories (optional)
    #[serde(default)]
    pub event_categories: Option<Vec<String>>,
    /// New memory focus (optional)
    #[serde(default)]
    pub memory_focus: Option<Vec<String>>,
    /// New boundaries (optional)
    #[serde(default)]
    pub boundaries: Option<Vec<String>>,
    /// New extraction prompt (optional)
    #[serde(default)]
    pub extraction_prompt: Option<String>,
    /// Whether to re-parse description with LLM
    #[serde(default)]
    pub reparse: bool,
}

impl From<UpdateProfileRequest> for UpdateProfileInput {
    fn from(req: UpdateProfileRequest) -> Self {
        UpdateProfileInput {
            name: req.name,
            description: req.description,
            purpose: req.purpose,
            domain: req.domain,
            target_audience: req.target_audience,
            event_categories: req.event_categories,
            memory_focus: req.memory_focus,
            boundaries: req.boundaries,
            extraction_prompt: req.extraction_prompt,
            metadata_schema: None,
            schema_status: None,
            schema_version: None,
            schema_confirmed_at: None,
            schema_generation_prompt: None,
            reparse: req.reparse,
        }
    }
}

/// Create profile routes
pub fn profile_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(create_profile))
        .route("/", get(list_profiles))
        .route("/ui", get(profile_editor_page))
        .route("/{profile_id}", get(get_profile_by_id))
        .route("/{profile_id}", put(update_profile_by_id))
        .route("/{profile_id}/schema/confirm", post(confirm_profile_schema))
}

fn profile_editor_html() -> &'static str {
    include_str!("profile_editor.html")
}

pub async fn profile_editor_page() -> Html<&'static str> {
    Html(profile_editor_html())
}

/// POST /api/v1/systems - Create a system profile namespace
///
/// Creates a system profile using LLM to parse the natural language description.
#[utoipa::path(
    post,
    path = "/api/v1/systems",
    tag = "system",
    request_body = InitializeProfileRequest,
    responses(
        (status = 201, description = "System profile created successfully", body = ProfileResponse),
        (status = 400, description = "Invalid request (validation error)", body = crate::error::ErrorResponse),
        (status = 409, description = "System profile already exists", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error (LLM parsing failed)", body = crate::error::ErrorResponse)
    )
)]
pub async fn create_profile(
    State(state): State<AppState>,
    Json(request): Json<InitializeProfileRequest>,
) -> AppResult<(StatusCode, Json<ProfileResponse>)> {
    tracing::info!(name = %request.name, "Creating system profile");

    let input: CreateProfileInput = request.into();
    let profile = state.profile_service.initialize(input).await?;

    tracing::info!(id = %profile.id, "System profile created successfully");

    Ok((StatusCode::CREATED, Json(profile.into())))
}

/// GET /api/v1/systems - List all system profiles
#[utoipa::path(
    get,
    path = "/api/v1/systems",
    tag = "system",
    responses(
        (status = 200, description = "System profiles listed successfully", body = Vec<ProfileResponse>),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn list_profiles(State(state): State<AppState>) -> AppResult<Json<Vec<ProfileResponse>>> {
    let profiles = state
        .profile_service
        .list()
        .await?
        .into_iter()
        .map(Into::into)
        .collect();

    Ok(Json(profiles))
}

/// GET /api/v1/systems/{profile_id} - Get a specific system profile
#[utoipa::path(
    get,
    path = "/api/v1/systems/{profile_id}",
    tag = "system",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID")
    ),
    responses(
        (status = 200, description = "System profile found", body = ProfileResponse),
        (status = 404, description = "System profile not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_profile_by_id(
    State(state): State<AppState>,
    axum::extract::Path(profile_id): axum::extract::Path<Uuid>,
) -> AppResult<Json<ProfileResponse>> {
    let profile = state.profile_service.get_by_id(profile_id).await?;

    Ok(Json(profile.into()))
}

/// PUT /api/v1/systems/{profile_id} - Update a specific system profile
#[utoipa::path(
    put,
    path = "/api/v1/systems/{profile_id}",
    tag = "system",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID")
    ),
    request_body = UpdateProfileRequest,
    responses(
        (status = 200, description = "System profile updated successfully", body = ProfileResponse),
        (status = 400, description = "Invalid request (validation error)", body = crate::error::ErrorResponse),
        (status = 404, description = "System profile not found", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error (LLM parsing failed)", body = crate::error::ErrorResponse)
    )
)]
pub async fn update_profile_by_id(
    State(state): State<AppState>,
    axum::extract::Path(profile_id): axum::extract::Path<Uuid>,
    Json(request): Json<UpdateProfileRequest>,
) -> AppResult<Json<ProfileResponse>> {
    tracing::info!(profile_id = %profile_id, reparse = request.reparse, "Updating system profile");

    let input: UpdateProfileInput = request.into();
    let updated = state.profile_service.update(profile_id, input).await?;

    tracing::info!(id = %updated.id, "System profile updated successfully");

    Ok(Json(updated.into()))
}

/// POST /api/v1/systems/{profile_id}/schema/confirm - Confirm schema proposal.
#[utoipa::path(
    post,
    path = "/api/v1/systems/{profile_id}/schema/confirm",
    tag = "system",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID")
    ),
    responses(
        (status = 200, description = "Profile schema confirmed", body = ConfirmProfileSchemaResponse),
        (status = 404, description = "System profile not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn confirm_profile_schema(
    State(state): State<AppState>,
    axum::extract::Path(profile_id): axum::extract::Path<Uuid>,
) -> AppResult<Json<ConfirmProfileSchemaResponse>> {
    let profile = state.profile_service.confirm_schema(profile_id).await?;

    Ok(Json(ConfirmProfileSchemaResponse {
        profile: profile.into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_request_conversion() {
        let request = InitializeProfileRequest {
            name: "房产推荐系统".to_string(),
            description: "这是一个房产推荐系统".to_string(),
        };

        let input: CreateProfileInput = request.clone().into();

        assert_eq!(input.name, request.name);
        assert_eq!(input.description, request.description);
    }

    #[test]
    fn test_update_request_conversion() {
        let request = UpdateProfileRequest {
            name: Some("新名称".to_string()),
            event_categories: Some(vec!["咨询".to_string(), "看房".to_string()]),
            reparse: false,
            ..Default::default()
        };

        let input: UpdateProfileInput = request.clone().into();

        assert_eq!(input.name, request.name);
        assert_eq!(input.event_categories, request.event_categories);
        assert_eq!(input.reparse, request.reparse);
        assert!(input.description.is_none());
        assert!(input.purpose.is_none());
    }

    #[test]
    fn test_update_request_defaults() {
        let json = r#"{"name": "新名称"}"#;
        let request: UpdateProfileRequest = serde_json::from_str(json).unwrap();

        assert_eq!(request.name, Some("新名称".to_string()));
        assert!(request.description.is_none());
        assert!(!request.reparse);
    }

    #[test]
    fn test_update_request_with_reparse() {
        let json = r#"{"description": "新描述", "reparse": true}"#;
        let request: UpdateProfileRequest = serde_json::from_str(json).unwrap();

        assert_eq!(request.description, Some("新描述".to_string()));
        assert!(request.reparse);
    }

    #[test]
    fn test_profile_response_serialization() {
        let response = ProfileResponse {
            id: Uuid::new_v4(),
            name: "测试系统".to_string(),
            description: "测试描述".to_string(),
            purpose: "测试用途".to_string(),
            domain: "测试领域".to_string(),
            target_audience: "测试用户".to_string(),
            event_categories: vec!["事件1".to_string()],
            memory_focus: vec!["记忆1".to_string()],
            boundaries: vec!["边界1".to_string()],
            extraction_prompt: "提取prompt".to_string(),
            metadata_schema: serde_json::json!({
                "version": 1,
                "entity_types": {
                    "user_profile": {
                        "fields": {
                            "memory_type": { "type": "string", "required": true, "filterable": true }
                        }
                    }
                },
                "filterable_fields": ["memory_type"],
                "retrieval_defaults": {
                    "use_vector": true,
                    "use_fulltext": true,
                    "collapse_lineage": true
                }
            }),
            schema_status: SchemaStatus::Draft,
            schema_version: 1,
            schema_confirmed_at: None,
            schema_generation_prompt: Some("请生成符合 schema 的 metadata".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("测试系统"));
        assert!(json.contains("测试领域"));
        assert!(json.contains("event_categories"));
        assert!(json.contains("metadata_schema"));
        assert!(json.contains("schema_status"));
    }

    #[test]
    fn test_confirm_profile_schema_response_serialization() {
        let response = ConfirmProfileSchemaResponse {
            profile: ProfileResponse {
                id: Uuid::new_v4(),
                name: "测试系统".to_string(),
                description: "测试描述".to_string(),
                purpose: "测试用途".to_string(),
                domain: "测试领域".to_string(),
                target_audience: "测试用户".to_string(),
                event_categories: vec![],
                memory_focus: vec![],
                boundaries: vec![],
                extraction_prompt: "提取prompt".to_string(),
                metadata_schema: serde_json::json!({
                    "version": 1,
                    "entity_types": {},
                    "filterable_fields": [],
                    "retrieval_defaults": {}
                }),
                schema_status: SchemaStatus::Confirmed,
                schema_version: 1,
                schema_confirmed_at: Some(Utc::now()),
                schema_generation_prompt: Some("schema prompt".to_string()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"profile\""));
        assert!(json.contains("\"confirmed\""));
    }

    #[test]
    fn test_profile_editor_page_contains_prompt_editor() {
        let html = profile_editor_html();
        assert!(html.contains("extraction_prompt"));
        assert!(html.contains("/api/v1/systems"));
    }
}
