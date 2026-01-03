//! Admin API handlers
//!
//! Implements:
//! - POST /api/v1/admin/eviction - Trigger eviction process
//! - POST /api/v1/admin/decay-update - Update decay scores

use axum::{extract::State, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::AppState;
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
    /// Custom eviction configuration (optional, uses defaults if not specified)
    pub config: Option<EvictionConfigRequest>,
}

/// Custom eviction configuration
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct EvictionConfigRequest {
    /// Days without hit before transitioning to Cooldown
    pub cooldown_threshold_days: Option<i64>,
    /// Days without hit before transitioning to Candidate
    pub candidate_threshold_days: Option<i64>,
    /// Days without hit before archiving
    pub archive_threshold_days: Option<i64>,
    /// Maximum number of active memories per owner
    pub max_memories_per_owner: Option<i64>,
    /// Decay score threshold below which memories become candidates
    pub decay_score_threshold: Option<f32>,
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

/// Create admin routes
pub fn admin_routes() -> Router<AppState> {
    Router::new()
        .route("/eviction", post(run_eviction))
        .route("/decay-update", post(update_decay_scores))
}

/// POST /api/v1/admin/eviction - Trigger eviction process
///
/// Runs the eviction process to transition stale memories through
/// the lifecycle states: Active → Cooldown → Candidate → Archived.
#[utoipa::path(
    post,
    path = "/api/v1/admin/eviction",
    tag = "admin",
    request_body = EvictionRequest,
    responses(
        (status = 200, description = "Eviction completed", body = EvictionResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn run_eviction(
    State(state): State<AppState>,
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
            .get_eviction_candidates(&owner_id, 100)
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
    let result = state.lifecycle_manager.run_eviction(&owner_id).await?;

    Ok(Json(EvictionResponse::from(result)))
}

/// POST /api/v1/admin/decay-update - Update decay scores
///
/// Recalculates decay scores for all memories of an owner based on
/// hit count and time since last activity.
#[utoipa::path(
    post,
    path = "/api/v1/admin/decay-update",
    tag = "admin",
    request_body = DecayUpdateRequest,
    responses(
        (status = 200, description = "Decay scores updated", body = DecayUpdateResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::error::ErrorResponse)
    )
)]
pub async fn update_decay_scores(
    State(state): State<AppState>,
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
        .update_decay_scores(&owner_id, &config)
        .await?;

    let response = DecayUpdateResponse {
        updated_count,
        owner_id: Some(owner_id),
        config_used: config.into(),
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
        assert!(request.config.is_none());
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
