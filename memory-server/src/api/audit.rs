//! Audit API handlers
//!
//! Implements:
//! - GET /api/v1/systems/{profile_id}/audit - Query audit logs

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::api::AppState;
use crate::error::AppResult;
use crate::repository::{AuditLogEntry, AuditOperation, AuditQueryParams};

/// Query parameters for audit log retrieval
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct AuditQueryRequest {
    /// Filter by memory ID
    pub memory_id: Option<Uuid>,
    /// Filter by start time
    pub start_time: Option<DateTime<Utc>>,
    /// Filter by end time
    pub end_time: Option<DateTime<Utc>>,
    /// Filter by operation type
    pub operation: Option<String>,
    /// Maximum number of results (default: 100)
    pub limit: Option<i64>,
    /// Offset for pagination
    pub offset: Option<i64>,
}

impl AuditQueryRequest {
    fn into_query_params(self, profile_id: Uuid) -> AuditQueryParams {
        AuditQueryParams {
            profile_id,
            memory_id: self.memory_id,
            start_time: self.start_time,
            end_time: self.end_time,
            operation: self.operation.and_then(|op| AuditOperation::from_str(&op)),
            limit: self.limit,
            offset: self.offset,
        }
    }
}

/// Audit log entry response
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuditLogEntryResponse {
    /// Unique identifier
    pub id: Uuid,
    /// System profile ID this audit entry belongs to
    pub profile_id: Uuid,
    /// Memory ID this audit entry relates to
    pub memory_id: Uuid,
    /// Operation type
    pub operation: String,
    /// Actor who performed the operation
    pub actor_id: Option<String>,
    /// Previous value (for updates)
    pub old_value: Option<serde_json::Value>,
    /// New value (for creates and updates)
    pub new_value: Option<serde_json::Value>,
    /// Reason for the operation
    pub reason: Option<String>,
    /// Timestamp of the operation
    pub created_at: DateTime<Utc>,
}

impl From<AuditLogEntry> for AuditLogEntryResponse {
    fn from(entry: AuditLogEntry) -> Self {
        AuditLogEntryResponse {
            id: entry.id,
            profile_id: entry.profile_id,
            memory_id: entry.memory_id,
            operation: entry.operation,
            actor_id: entry.actor_id,
            old_value: entry.old_value,
            new_value: entry.new_value,
            reason: entry.reason,
            created_at: entry.created_at,
        }
    }
}

/// Response for audit log query
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuditQueryResponse {
    /// Audit log entries
    pub entries: Vec<AuditLogEntryResponse>,
    /// Total count matching the query
    pub total: i64,
}

/// Create audit routes
pub fn audit_routes() -> Router<AppState> {
    Router::new().route("/", get(query_audit_logs))
}

/// GET /api/v1/systems/{profile_id}/audit - Query audit logs
///
/// Retrieves audit logs with optional filters:
/// - memory_id: Filter by specific memory
/// - start_time/end_time: Filter by time range
/// - operation: Filter by operation type (create, update, delete, status_change, hit)
/// - limit/offset: Pagination
#[utoipa::path(
    get,
    path = "/api/v1/systems/{profile_id}/audit",
    tag = "audit",
    params(
        ("profile_id" = Uuid, Path, description = "System profile ID"),
        AuditQueryRequest
    ),
    responses(
        (status = 200, description = "Audit logs retrieved", body = AuditQueryResponse)
    )
)]
pub async fn query_audit_logs(
    State(state): State<AppState>,
    Path(profile_id): Path<Uuid>,
    Query(request): Query<AuditQueryRequest>,
) -> AppResult<Json<AuditQueryResponse>> {
    let params = request.into_query_params(profile_id);

    let entries = state.lifecycle_manager.query_audit_logs(&params).await?;
    let total = state.lifecycle_manager.count_audit_logs(&params).await?;

    let response = AuditQueryResponse {
        entries: entries.into_iter().map(Into::into).collect(),
        total,
    };

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_query_request_conversion() {
        let request = AuditQueryRequest {
            memory_id: Some(Uuid::new_v4()),
            start_time: Some(Utc::now()),
            end_time: Some(Utc::now()),
            operation: Some("create".to_string()),
            limit: Some(50),
            offset: Some(10),
        };

        let profile_id = Uuid::new_v4();
        let params = request.clone().into_query_params(profile_id);

        assert_eq!(params.profile_id, profile_id);
        assert_eq!(params.memory_id, request.memory_id);
        assert_eq!(params.start_time, request.start_time);
        assert_eq!(params.end_time, request.end_time);
        assert!(params.operation.is_some());
        assert_eq!(params.limit, request.limit);
        assert_eq!(params.offset, request.offset);
    }

    #[test]
    fn test_audit_query_request_minimal() {
        let request = AuditQueryRequest {
            memory_id: None,
            start_time: None,
            end_time: None,
            operation: None,
            limit: None,
            offset: None,
        };

        let profile_id = Uuid::new_v4();
        let params = request.into_query_params(profile_id);

        assert_eq!(params.profile_id, profile_id);
        assert!(params.memory_id.is_none());
        assert!(params.start_time.is_none());
        assert!(params.end_time.is_none());
        assert!(params.operation.is_none());
        assert!(params.limit.is_none());
        assert!(params.offset.is_none());
    }

    #[test]
    fn test_audit_log_entry_conversion() {
        let entry = AuditLogEntry {
            id: Uuid::new_v4(),
            profile_id: Uuid::new_v4(),
            memory_id: Uuid::new_v4(),
            operation: "create".to_string(),
            actor_id: Some("user123".to_string()),
            old_value: None,
            new_value: Some(serde_json::json!({"content": "test"})),
            reason: Some("Initial creation".to_string()),
            created_at: Utc::now(),
        };

        let response: AuditLogEntryResponse = entry.clone().into();

        assert_eq!(response.id, entry.id);
        assert_eq!(response.profile_id, entry.profile_id);
        assert_eq!(response.memory_id, entry.memory_id);
        assert_eq!(response.operation, entry.operation);
        assert_eq!(response.actor_id, entry.actor_id);
        assert_eq!(response.old_value, entry.old_value);
        assert_eq!(response.new_value, entry.new_value);
        assert_eq!(response.reason, entry.reason);
    }
}
