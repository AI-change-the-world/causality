//! Audit Repository implementation
//!
//! Provides audit log operations for tracking memory changes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::error::AppResult;

/// Audit operation types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOperation {
    /// Memory created
    Create,
    /// Memory updated
    Update,
    /// Memory deleted (archived)
    Delete,
    /// Memory status changed
    StatusChange,
    /// Memory hit recorded
    Hit,
}

impl std::fmt::Display for AuditOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditOperation::Create => write!(f, "create"),
            AuditOperation::Update => write!(f, "update"),
            AuditOperation::Delete => write!(f, "delete"),
            AuditOperation::StatusChange => write!(f, "status_change"),
            AuditOperation::Hit => write!(f, "hit"),
        }
    }
}

impl AuditOperation {
    /// Parse from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "create" => Some(AuditOperation::Create),
            "update" => Some(AuditOperation::Update),
            "delete" => Some(AuditOperation::Delete),
            "status_change" => Some(AuditOperation::StatusChange),
            "hit" => Some(AuditOperation::Hit),
            _ => None,
        }
    }
}

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    /// Unique identifier
    pub id: Uuid,
    /// System profile ID this audit entry belongs to
    pub profile_id: Uuid,
    /// Memory ID this audit entry relates to
    pub memory_id: Uuid,
    /// Operation type
    pub operation: String,
    /// Actor who performed the operation (optional)
    pub actor_id: Option<String>,
    /// Previous value (for updates)
    pub old_value: Option<serde_json::Value>,
    /// New value (for creates and updates)
    pub new_value: Option<serde_json::Value>,
    /// Reason for the operation (optional)
    pub reason: Option<String>,
    /// Timestamp of the operation
    pub created_at: DateTime<Utc>,
}

/// Query parameters for audit log retrieval
#[derive(Debug, Clone, Default)]
pub struct AuditQueryParams {
    /// Filter by system profile ID
    pub profile_id: Uuid,
    /// Filter by memory ID
    pub memory_id: Option<Uuid>,
    /// Filter by start time
    pub start_time: Option<DateTime<Utc>>,
    /// Filter by end time
    pub end_time: Option<DateTime<Utc>>,
    /// Filter by operation type
    pub operation: Option<AuditOperation>,
    /// Maximum number of results
    pub limit: Option<i64>,
    /// Offset for pagination
    pub offset: Option<i64>,
}

/// Repository for audit log operations
#[derive(Clone)]
pub struct AuditRepository {
    pool: PgPool,
}

impl AuditRepository {
    /// Create a new AuditRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new audit log entry
    pub async fn create(
        &self,
        profile_id: Uuid,
        memory_id: Uuid,
        operation: AuditOperation,
        actor_id: Option<String>,
        old_value: Option<serde_json::Value>,
        new_value: Option<serde_json::Value>,
        reason: Option<String>,
    ) -> AppResult<AuditLogEntry> {
        let row = sqlx::query_as::<_, AuditLogRow>(
            r#"
            INSERT INTO audit_logs (profile_id, memory_id, operation, actor_id, old_value, new_value, reason)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, profile_id, memory_id, operation, actor_id, old_value, new_value, reason, created_at
            "#,
        )
        .bind(profile_id)
        .bind(memory_id)
        .bind(operation.to_string())
        .bind(&actor_id)
        .bind(&old_value)
        .bind(&new_value)
        .bind(&reason)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into())
    }

    /// Query audit logs with filters
    pub async fn query(&self, params: &AuditQueryParams) -> AppResult<Vec<AuditLogEntry>> {
        let limit = params.limit.unwrap_or(100);
        let offset = params.offset.unwrap_or(0);
        let operation_str = params.operation.map(|op| op.to_string());

        let rows = sqlx::query_as::<_, AuditLogRow>(
            r#"
            SELECT id, profile_id, memory_id, operation, actor_id, old_value, new_value, reason, created_at
            FROM audit_logs
            WHERE profile_id = $1
              AND ($2::uuid IS NULL OR memory_id = $2)
              AND ($3::timestamptz IS NULL OR created_at >= $3)
              AND ($4::timestamptz IS NULL OR created_at <= $4)
              AND ($5::text IS NULL OR operation = $5)
            ORDER BY created_at DESC
            LIMIT $6
            OFFSET $7
            "#,
        )
        .bind(params.profile_id)
        .bind(params.memory_id)
        .bind(params.start_time)
        .bind(params.end_time)
        .bind(&operation_str)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Get audit logs for a specific memory
    pub async fn get_by_memory_id(&self, memory_id: Uuid) -> AppResult<Vec<AuditLogEntry>> {
        let rows = sqlx::query_as::<_, AuditLogRow>(
            r#"
            SELECT id, profile_id, memory_id, operation, actor_id, old_value, new_value, reason, created_at
            FROM audit_logs
            WHERE memory_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(memory_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Count audit logs matching the query
    pub async fn count(&self, params: &AuditQueryParams) -> AppResult<i64> {
        let operation_str = params.operation.map(|op| op.to_string());

        let result: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM audit_logs
            WHERE profile_id = $1
              AND ($2::uuid IS NULL OR memory_id = $2)
              AND ($3::timestamptz IS NULL OR created_at >= $3)
              AND ($4::timestamptz IS NULL OR created_at <= $4)
              AND ($5::text IS NULL OR operation = $5)
            "#,
        )
        .bind(params.profile_id)
        .bind(params.memory_id)
        .bind(params.start_time)
        .bind(params.end_time)
        .bind(&operation_str)
        .fetch_one(&self.pool)
        .await?;

        Ok(result.0)
    }
}

/// Internal row type for sqlx mapping
#[derive(Debug, FromRow)]
struct AuditLogRow {
    id: Uuid,
    profile_id: Uuid,
    memory_id: Uuid,
    operation: String,
    actor_id: Option<String>,
    old_value: Option<serde_json::Value>,
    new_value: Option<serde_json::Value>,
    reason: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<AuditLogRow> for AuditLogEntry {
    fn from(row: AuditLogRow) -> Self {
        AuditLogEntry {
            id: row.id,
            profile_id: row.profile_id,
            memory_id: row.memory_id,
            operation: row.operation,
            actor_id: row.actor_id,
            old_value: row.old_value,
            new_value: row.new_value,
            reason: row.reason,
            created_at: row.created_at,
        }
    }
}
