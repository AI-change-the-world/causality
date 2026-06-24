//! Admin API handlers
//!
//! Implements:
//! - POST /api/v1/systems/{profile_id}/admin/eviction - Trigger eviction
//! - POST /api/v1/systems/{profile_id}/admin/decay-update - Update decay scores
//! - POST /api/v1/systems/{profile_id}/admin/embeddings/rebuild - Rebuild memory embeddings

use axum::{
    extract::{Path, State},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::{memory::vector_payload, AppState};
use crate::domain::EmbeddingStatus;
use crate::embedding::EmbeddingRequest;
use crate::error::AppResult;
use crate::service::{DecayConfig, EvictionResult};

/// Request body for triggering eviction
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct EvictionRequest {
    /// Owner ID to run eviction for (optional, runs for all if not specified)
    pub owner_id: Option<String>,
    /// Whether to perform a dry run (preview only, no changes)
    #[serde(default)]
    pub dry_run: bool,
}

/// Response for eviction operation
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EvictionResponse {
    /// Whether this was a dry run
    pub dry_run: bool,
    /// Number of memories transitioned to Cooldown
    pub cooldown_count: usize,
    /// Number of memories transitioned to Candidate
    pub candidate_count: usize,
    /// Number of memories archived
    pub archived_count: usize,
    /// Whether capacity limit was reached
    pub capacity_limit_reached: bool,
    /// Total memories affected
    pub total_affected: usize,
}

impl From<EvictionResult> for EvictionResponse {
    fn from(result: EvictionResult) -> Self {
        EvictionResponse {
            dry_run: false,
            cooldown_count: result.cooldown_count,
            candidate_count: result.candidate_count,
            archived_count: result.archived_count,
            capacity_limit_reached: result.capacity_limit_reached,
            total_affected: result.total_affected(),
        }
    }
}

/// Request body for updating decay scores
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DecayUpdateRequest {
    /// Owner ID to update decay scores for (optional, updates for all if not specified)
    pub owner_id: Option<String>,
    /// Custom decay configuration (optional, uses defaults if not specified)
    pub config: Option<DecayConfigRequest>,
}

/// Custom decay configuration
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DecayConfigRequest {
    /// Half-life in days for decay
    pub decay_half_life_days: Option<f32>,
    /// Boost factor for hit count
    pub hit_boost_factor: Option<f32>,
    /// Boost multiplier for global memories
    pub global_boost: Option<f32>,
}

/// Response for decay update operation
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DecayUpdateResponse {
    /// Number of memories updated
    pub updated_count: usize,
    /// Owner ID that was updated (if specified)
    pub owner_id: Option<String>,
    /// Configuration used for the update
    pub config_used: DecayConfigResponse,
}

/// Decay configuration in response
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DecayConfigResponse {
    pub decay_half_life_days: f32,
    pub hit_boost_factor: f32,
    pub global_boost: f32,
}

impl From<DecayConfig> for DecayConfigResponse {
    fn from(config: DecayConfig) -> Self {
        DecayConfigResponse {
            decay_half_life_days: config.decay_half_life_days,
            hit_boost_factor: config.hit_boost_factor,
            global_boost: config.global_boost,
        }
    }
}

/// Request body for rebuilding memory embeddings.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RebuildEmbeddingsRequest {
    /// Owner ID to rebuild for. Omit to rebuild across all owners in this profile.
    pub owner_id: Option<String>,
    /// Maximum number of memories to process in this request.
    pub limit: Option<usize>,
    /// Include memories whose embedding is still pending.
    #[serde(default = "default_rebuild_pending")]
    pub include_pending: bool,
    /// Include memories whose previous embedding attempt failed.
    #[serde(default = "default_rebuild_failed")]
    pub include_failed: bool,
}

fn default_rebuild_pending() -> bool {
    true
}

fn default_rebuild_failed() -> bool {
    true
}

/// A single memory that failed during embedding rebuild.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RebuildEmbeddingFailure {
    /// Memory ID that failed.
    pub memory_id: Uuid,
    /// Failure message.
    pub error: String,
}

/// Response for embedding rebuild operation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RebuildEmbeddingsResponse {
    /// Number of memories selected for rebuild.
    pub scanned_count: usize,
    /// Number of vectors successfully rebuilt.
    pub rebuilt_count: usize,
    /// Number of memories that failed during rebuild.
    pub failed_count: usize,
    /// Successfully rebuilt memory IDs.
    pub rebuilt_memory_ids: Vec<Uuid>,
    /// Per-memory rebuild failures.
    pub failures: Vec<RebuildEmbeddingFailure>,
}

/// Create admin routes
pub fn admin_routes() -> Router<AppState> {
    Router::new()
        .route("/eviction", post(run_eviction))
        .route("/decay-update", post(update_decay_scores))
        .route("/embeddings/rebuild", post(rebuild_embeddings))
}

/// POST /api/v1/systems/{profile_id}/admin/eviction - Trigger eviction process
///
/// Runs the eviction process to transition stale memories through
/// the lifecycle states: Active → Cooldown → Candidate → Archived.
#[utoipa::path(
    post,
    path = "/api/v1/systems/{profile_id}/admin/eviction",
    tag = "admin",
    params(
        ("profile_id" = uuid::Uuid, Path, description = "System profile ID")
    ),
    request_body = EvictionRequest,
    responses(
        (status = 200, description = "Eviction completed", body = EvictionResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn run_eviction(
    State(state): State<AppState>,
    Path(profile_id): Path<uuid::Uuid>,
    Json(request): Json<EvictionRequest>,
) -> AppResult<Json<EvictionResponse>> {
    // For now, we require an owner_id
    let owner_id = request
        .owner_id
        .ok_or_else(|| crate::error::AppError::Validation("owner_id is required".to_string()))?;

    if request.dry_run {
        // Dry run: just get candidates without making changes
        let candidates = state
            .lifecycle_manager
            .get_eviction_candidates(profile_id, &owner_id, 100)
            .await?;

        let response = EvictionResponse {
            dry_run: true,
            cooldown_count: candidates
                .iter()
                .filter(|m| m.status == crate::domain::Status::Active)
                .count(),
            candidate_count: candidates
                .iter()
                .filter(|m| m.status == crate::domain::Status::Cooldown)
                .count(),
            archived_count: candidates
                .iter()
                .filter(|m| m.status == crate::domain::Status::Candidate)
                .count(),
            capacity_limit_reached: false,
            total_affected: candidates.len(),
        };

        return Ok(Json(response));
    }

    // Run actual eviction
    let result = state
        .lifecycle_manager
        .run_eviction(profile_id, &owner_id)
        .await?;

    Ok(Json(EvictionResponse::from(result)))
}

/// POST /api/v1/systems/{profile_id}/admin/decay-update - Update decay scores
///
/// Recalculates decay scores for all memories of an owner based on
/// hit count and time since last activity.
#[utoipa::path(
    post,
    path = "/api/v1/systems/{profile_id}/admin/decay-update",
    tag = "admin",
    params(
        ("profile_id" = uuid::Uuid, Path, description = "System profile ID")
    ),
    request_body = DecayUpdateRequest,
    responses(
        (status = 200, description = "Decay scores updated", body = DecayUpdateResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn update_decay_scores(
    State(state): State<AppState>,
    Path(profile_id): Path<uuid::Uuid>,
    Json(request): Json<DecayUpdateRequest>,
) -> AppResult<Json<DecayUpdateResponse>> {
    // For now, we require an owner_id
    let owner_id = request
        .owner_id
        .clone()
        .ok_or_else(|| crate::error::AppError::Validation("owner_id is required".to_string()))?;

    // Build config from request or use defaults
    let config = if let Some(req_config) = request.config {
        DecayConfig {
            decay_half_life_days: req_config.decay_half_life_days.unwrap_or(7.0),
            hit_boost_factor: req_config.hit_boost_factor.unwrap_or(0.1),
            global_boost: req_config.global_boost.unwrap_or(2.0),
        }
    } else {
        DecayConfig::default()
    };

    // Update decay scores
    let updated_count = state
        .lifecycle_manager
        .update_decay_scores(profile_id, &owner_id, &config)
        .await?;

    let response = DecayUpdateResponse {
        updated_count,
        owner_id: Some(owner_id),
        config_used: config.into(),
    };

    Ok(Json(response))
}

/// POST /api/v1/systems/{profile_id}/admin/embeddings/rebuild - Rebuild embeddings
///
/// Recreates Qdrant vectors for memories whose embedding status is pending
/// or failed. This is a synchronous small-batch repair endpoint for recovering
/// from embedding or vector database failures.
#[utoipa::path(
    post,
    path = "/api/v1/systems/{profile_id}/admin/embeddings/rebuild",
    tag = "admin",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID")
    ),
    request_body = RebuildEmbeddingsRequest,
    responses(
        (status = 200, description = "Embedding rebuild completed", body = RebuildEmbeddingsResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn rebuild_embeddings(
    State(state): State<AppState>,
    Path(profile_id): Path<Uuid>,
    Json(request): Json<RebuildEmbeddingsRequest>,
) -> AppResult<Json<RebuildEmbeddingsResponse>> {
    let mut statuses = Vec::new();
    if request.include_pending {
        statuses.push(EmbeddingStatus::Pending);
    }
    if request.include_failed {
        statuses.push(EmbeddingStatus::Failed);
    }

    if statuses.is_empty() {
        return Err(crate::error::AppError::Validation(
            "at least one embedding status must be selected".to_string(),
        ));
    }

    let limit = request.limit.unwrap_or(100).min(1000) as i64;
    let memories = state
        .memory_guard
        .memory_repo()
        .find_embeddings_to_rebuild(profile_id, request.owner_id.as_deref(), &statuses, limit)
        .await?;

    let scanned_count = memories.len();
    let provider_name = state.embedding_provider_name.clone();
    let mut rebuilt_memory_ids = Vec::new();
    let mut failures = Vec::new();

    for memory in memories {
        let embedding_request = EmbeddingRequest::new(&memory.content);
        let embedding_response = match state.embedding_provider.embed(embedding_request).await {
            Ok(response) => response,
            Err(error) => {
                let _ = state
                    .memory_guard
                    .memory_repo()
                    .update_embedding_status_and_provider(memory.id, EmbeddingStatus::Failed, None)
                    .await;
                failures.push(RebuildEmbeddingFailure {
                    memory_id: memory.id,
                    error: format!("embedding generation failed: {}", error),
                });
                continue;
            }
        };

        if let Err(error) = state
            .qdrant_repo
            .upsert_vector(
                &provider_name,
                memory.id,
                embedding_response.embedding,
                vector_payload(&memory),
            )
            .await
        {
            let _ = state
                .memory_guard
                .memory_repo()
                .update_embedding_status_and_provider(memory.id, EmbeddingStatus::Failed, None)
                .await;
            failures.push(RebuildEmbeddingFailure {
                memory_id: memory.id,
                error: format!("vector upsert failed: {}", error),
            });
            continue;
        }

        state
            .memory_guard
            .memory_repo()
            .update_embedding_status_and_provider(
                memory.id,
                EmbeddingStatus::Completed,
                Some(&provider_name),
            )
            .await?;
        rebuilt_memory_ids.push(memory.id);
    }

    let response = RebuildEmbeddingsResponse {
        scanned_count,
        rebuilt_count: rebuilt_memory_ids.len(),
        failed_count: failures.len(),
        rebuilt_memory_ids,
        failures,
    };

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eviction_request_defaults() {
        let json = r#"{}"#;
        let request: EvictionRequest = serde_json::from_str(json).unwrap();
        assert!(request.owner_id.is_none());
        assert!(!request.dry_run);
    }

    #[test]
    fn test_eviction_request_with_dry_run() {
        let json = r#"{"owner_id": "owner123", "dry_run": true}"#;
        let request: EvictionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.owner_id, Some("owner123".to_string()));
        assert!(request.dry_run);
    }

    #[test]
    fn test_decay_update_request_defaults() {
        let json = r#"{"owner_id": "owner123"}"#;
        let request: DecayUpdateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.owner_id, Some("owner123".to_string()));
        assert!(request.config.is_none());
    }

    #[test]
    fn test_decay_update_request_with_config() {
        let json = r#"{
            "owner_id": "owner123",
            "config": {
                "decay_half_life_days": 14.0,
                "hit_boost_factor": 0.2,
                "global_boost": 3.0
            }
        }"#;
        let request: DecayUpdateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.owner_id, Some("owner123".to_string()));
        let config = request.config.unwrap();
        assert_eq!(config.decay_half_life_days, Some(14.0));
        assert_eq!(config.hit_boost_factor, Some(0.2));
        assert_eq!(config.global_boost, Some(3.0));
    }

    #[test]
    fn test_rebuild_embeddings_request_defaults() {
        let json = r#"{}"#;
        let request: RebuildEmbeddingsRequest = serde_json::from_str(json).unwrap();

        assert!(request.owner_id.is_none());
        assert!(request.limit.is_none());
        assert!(request.include_pending);
        assert!(request.include_failed);
    }

    #[test]
    fn test_rebuild_embeddings_request_custom_statuses() {
        let json = r#"{
            "owner_id": "owner123",
            "limit": 25,
            "include_pending": false,
            "include_failed": true
        }"#;
        let request: RebuildEmbeddingsRequest = serde_json::from_str(json).unwrap();

        assert_eq!(request.owner_id, Some("owner123".to_string()));
        assert_eq!(request.limit, Some(25));
        assert!(!request.include_pending);
        assert!(request.include_failed);
    }

    #[test]
    fn test_eviction_response_serialization() {
        let response = EvictionResponse {
            dry_run: false,
            cooldown_count: 5,
            candidate_count: 3,
            archived_count: 2,
            capacity_limit_reached: false,
            total_affected: 10,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"cooldown_count\":5"));
        assert!(json.contains("\"total_affected\":10"));
    }

    #[test]
    fn test_decay_config_response() {
        let config = DecayConfig {
            decay_half_life_days: 7.0,
            hit_boost_factor: 0.1,
            global_boost: 2.0,
        };
        let response: DecayConfigResponse = config.into();
        assert_eq!(response.decay_half_life_days, 7.0);
        assert_eq!(response.hit_boost_factor, 0.1);
        assert_eq!(response.global_boost, 2.0);
    }
}
