//! Event Repository implementation
//!
//! Provides CRUD operations for Event entities and event-memory relations.
//!
//! In the new architecture:
//! - Events are completely immutable after creation
//! - scope_id is optional (null = global context)
//! - No event_type, scope_type, or scene fields

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::domain::{Event, EventMemoryRelation, EventMemoryRelationType, ProcessingStatus};
use crate::error::{AppError, AppResult};

const EVENT_COLUMNS: &str = r#"
    id, profile_id, owner_id, scope_id, content, context,
    summary, source, processing_status, error_message, processed_at,
    skipped, skip_reason, relevance_score, event_time, created_at
"#;

/// Repository for Event CRUD operations
#[derive(Clone)]
pub struct EventRepository {
    pool: PgPool,
}

impl EventRepository {
    /// Create a new EventRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new event in the database
    pub async fn create(&self, event: &Event) -> AppResult<Event> {
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            INSERT INTO events (
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processing_status, error_message, processed_at,
                skipped, skip_reason, relevance_score, event_time, created_at
            )
            VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8,
                $9, $10, $11, $12, $13, $14, $15, $16
            )
            RETURNING
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processing_status, error_message, processed_at,
                skipped, skip_reason, relevance_score, event_time, created_at
            "#,
        )
        .bind(event.id)
        .bind(event.profile_id)
        .bind(&event.owner_id)
        .bind(&event.scope_id)
        .bind(&event.content)
        .bind(&event.context)
        .bind(&event.summary)
        .bind(event.source.map(|source| source.to_string()))
        .bind(event.processing_status)
        .bind(&event.error_message)
        .bind(event.processed_at)
        .bind(event.skipped)
        .bind(&event.skip_reason)
        .bind(event.relevance_score)
        .bind(event.event_time)
        .bind(event.created_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into())
    }

    /// Get an event by ID
    pub async fn get_by_id(&self, id: Uuid) -> AppResult<Event> {
        let row = sqlx::query_as::<_, EventRow>(&format!(
            r#"
            SELECT {}
            FROM events
            WHERE id = $1
            "#,
            EVENT_COLUMNS
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::EventNotFound(id))?;

        Ok(row.into())
    }

    /// Mark an event as processed with a summary
    pub async fn mark_processed(&self, id: Uuid, summary: &str) -> AppResult<Event> {
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            UPDATE events
            SET processing_status = 'completed'::processing_status,
                summary = $2,
                error_message = NULL,
                processed_at = NOW(),
                skipped = false,
                skip_reason = NULL
            WHERE id = $1
            RETURNING
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processing_status, error_message, processed_at,
                skipped, skip_reason, relevance_score, event_time, created_at
            "#,
        )
        .bind(id)
        .bind(summary)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::EventNotFound(id))?;

        Ok(row.into())
    }

    /// Mark an event as skipped with structured skip metadata.
    pub async fn mark_skipped(
        &self,
        id: Uuid,
        skip_reason: &str,
        relevance_score: Option<f32>,
    ) -> AppResult<Event> {
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            UPDATE events
            SET processing_status = 'skipped'::processing_status,
                error_message = NULL,
                processed_at = NOW(),
                skipped = true,
                skip_reason = $2,
                relevance_score = $3
            WHERE id = $1
            RETURNING
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processing_status, error_message, processed_at,
                skipped, skip_reason, relevance_score, event_time, created_at
            "#,
        )
        .bind(id)
        .bind(skip_reason)
        .bind(relevance_score)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::EventNotFound(id))?;

        Ok(row.into())
    }

    /// Mark an event as failed and persist the processing error for retry/debugging.
    pub async fn mark_failed(&self, id: Uuid, error_message: &str) -> AppResult<Event> {
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            UPDATE events
            SET processing_status = 'failed'::processing_status,
                error_message = $2,
                processed_at = NOW(),
                skipped = false
            WHERE id = $1
            RETURNING
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processing_status, error_message, processed_at,
                skipped, skip_reason, relevance_score, event_time, created_at
            "#,
        )
        .bind(id)
        .bind(error_message)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::EventNotFound(id))?;

        Ok(row.into())
    }

    /// Reset a failed event to pending before retrying it.
    pub async fn reset_for_retry(&self, profile_id: Uuid, id: Uuid) -> AppResult<Event> {
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            UPDATE events
            SET processing_status = 'pending'::processing_status,
                error_message = NULL,
                processed_at = NULL,
                skipped = false,
                skip_reason = NULL,
                relevance_score = NULL
            WHERE id = $1
              AND profile_id = $2
              AND processing_status = 'failed'::processing_status
            RETURNING
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processing_status, error_message, processed_at,
                skipped, skip_reason, relevance_score, event_time, created_at
            "#,
        )
        .bind(id)
        .bind(profile_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            AppError::Validation(
                "event is not failed or does not belong to the requested system profile"
                    .to_string(),
            )
        })?;

        Ok(row.into())
    }

    /// Find unprocessed events for a given owner within a system profile.
    pub async fn find_unprocessed(
        &self,
        profile_id: Uuid,
        owner_id: &str,
        limit: i64,
    ) -> AppResult<Vec<Event>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processing_status, error_message, processed_at,
                skipped, skip_reason, relevance_score, event_time, created_at
            FROM events
            WHERE profile_id = $1
              AND owner_id = $2
              AND processing_status = 'pending'::processing_status
            ORDER BY event_time ASC
            LIMIT $3
            "#,
        )
        .bind(profile_id)
        .bind(owner_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Find events by owner and scope within a system profile.
    pub async fn find_by_scope(
        &self,
        profile_id: Uuid,
        owner_id: &str,
        scope_id: Option<&str>,
        limit: i64,
    ) -> AppResult<Vec<Event>> {
        let rows = if let Some(scope) = scope_id {
            sqlx::query_as::<_, EventRow>(
                r#"
                SELECT
                    id, profile_id, owner_id, scope_id, content, context,
                    summary, source, processing_status, error_message, processed_at,
                    skipped, skip_reason, relevance_score, event_time, created_at
                FROM events
                WHERE profile_id = $1
                  AND owner_id = $2
                  AND scope_id = $3
                ORDER BY event_time DESC
                LIMIT $4
                "#,
            )
            .bind(profile_id)
            .bind(owner_id)
            .bind(scope)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            // Find events with null scope_id (global events)
            sqlx::query_as::<_, EventRow>(
                r#"
                SELECT
                    id, profile_id, owner_id, scope_id, content, context,
                    summary, source, processing_status, error_message, processed_at,
                    skipped, skip_reason, relevance_score, event_time, created_at
                FROM events
                WHERE profile_id = $1
                  AND owner_id = $2
                  AND scope_id IS NULL
                ORDER BY event_time DESC
                LIMIT $3
                "#,
            )
            .bind(profile_id)
            .bind(owner_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Find events by owner across all scopes within a system profile.
    pub async fn find_by_owner(
        &self,
        profile_id: Uuid,
        owner_id: &str,
        limit: i64,
    ) -> AppResult<Vec<Event>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processing_status, error_message, processed_at,
                skipped, skip_reason, relevance_score, event_time, created_at
            FROM events
            WHERE profile_id = $1
              AND owner_id = $2
            ORDER BY event_time DESC
            LIMIT $3
            "#,
        )
        .bind(profile_id)
        .bind(owner_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Create an event-memory relation
    pub async fn create_relation(
        &self,
        relation: &EventMemoryRelation,
    ) -> AppResult<EventMemoryRelation> {
        let row = sqlx::query_as::<_, RelationRow>(
            r#"
            INSERT INTO event_memory_relations (
                id, event_id, memory_id, relation_type, created_at
            )
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (event_id, memory_id) DO UPDATE
            SET relation_type = EXCLUDED.relation_type
            RETURNING id, event_id, memory_id, relation_type, created_at
            "#,
        )
        .bind(relation.id)
        .bind(relation.event_id)
        .bind(relation.memory_id)
        .bind(&relation.relation_type)
        .bind(relation.created_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into())
    }

    /// Get all relations for a memory
    pub async fn get_relations_for_memory(
        &self,
        memory_id: Uuid,
    ) -> AppResult<Vec<EventMemoryRelation>> {
        let rows = sqlx::query_as::<_, RelationRow>(
            r#"
            SELECT id, event_id, memory_id, relation_type, created_at
            FROM event_memory_relations
            WHERE memory_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(memory_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Get all relations for an event
    pub async fn get_relations_for_event(
        &self,
        event_id: Uuid,
    ) -> AppResult<Vec<EventMemoryRelation>> {
        let rows = sqlx::query_as::<_, RelationRow>(
            r#"
            SELECT id, event_id, memory_id, relation_type, created_at
            FROM event_memory_relations
            WHERE event_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(event_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Count evidence (reinforcing events) for a memory
    pub async fn count_evidence(&self, memory_id: Uuid) -> AppResult<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM event_memory_relations
            WHERE memory_id = $1 AND relation_type IN ('created_from', 'reinforced_by')
            "#,
        )
        .bind(memory_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    /// Get unique scope count for a memory's evidence
    pub async fn count_scope_diversity(&self, memory_id: Uuid) -> AppResult<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(DISTINCT e.scope_id)
            FROM event_memory_relations emr
            JOIN events e ON emr.event_id = e.id
            WHERE emr.memory_id = $1 AND emr.relation_type IN ('created_from', 'reinforced_by')
            "#,
        )
        .bind(memory_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    /// Get events that support a memory (created_from or reinforced_by)
    pub async fn get_supporting_events(
        &self,
        profile_id: Uuid,
        memory_id: Uuid,
    ) -> AppResult<Vec<Event>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT e.id, e.profile_id, e.owner_id, e.scope_id, e.content, e.context,
                   e.summary, e.source, e.processing_status, e.error_message, e.processed_at,
                   e.skipped, e.skip_reason, e.relevance_score, e.event_time, e.created_at
            FROM events e
            JOIN event_memory_relations emr ON e.id = emr.event_id
            WHERE e.profile_id = $1
              AND emr.memory_id = $2
              AND emr.relation_type IN ('created_from', 'reinforced_by')
            ORDER BY e.event_time DESC
            "#,
        )
        .bind(profile_id)
        .bind(memory_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Get the source event for a memory (created_from relation)
    pub async fn get_source_event(
        &self,
        profile_id: Uuid,
        memory_id: Uuid,
    ) -> AppResult<Option<Event>> {
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT e.id, e.profile_id, e.owner_id, e.scope_id, e.content, e.context,
                   e.summary, e.source, e.processing_status, e.error_message, e.processed_at,
                   e.skipped, e.skip_reason, e.relevance_score, e.event_time, e.created_at
            FROM events e
            JOIN event_memory_relations emr ON e.id = emr.event_id
            WHERE e.profile_id = $1
              AND emr.memory_id = $2
              AND emr.relation_type = 'created_from'
            LIMIT 1
            "#,
        )
        .bind(profile_id)
        .bind(memory_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }
}

/// Internal row type for Event
#[derive(Debug, FromRow)]
struct EventRow {
    id: Uuid,
    profile_id: Uuid,
    owner_id: String,
    scope_id: Option<String>,
    content: String,
    context: Option<String>,
    summary: Option<String>,
    source: Option<String>,
    processing_status: ProcessingStatus,
    error_message: Option<String>,
    processed_at: Option<DateTime<Utc>>,
    skipped: bool,
    skip_reason: Option<String>,
    relevance_score: Option<f32>,
    event_time: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

impl From<EventRow> for Event {
    fn from(row: EventRow) -> Self {
        Event {
            id: row.id,
            profile_id: row.profile_id,
            owner_id: row.owner_id,
            scope_id: row.scope_id,
            content: row.content,
            context: row.context,
            summary: row.summary,
            source: row.source.and_then(|source| source.parse().ok()),
            processing_status: row.processing_status,
            error_message: row.error_message,
            processed_at: row.processed_at,
            skipped: row.skipped,
            skip_reason: row.skip_reason,
            relevance_score: row.relevance_score,
            event_time: row.event_time,
            created_at: row.created_at,
        }
    }
}

/// Internal row type for EventMemoryRelation
#[derive(Debug, FromRow)]
struct RelationRow {
    id: Uuid,
    event_id: Uuid,
    memory_id: Uuid,
    relation_type: EventMemoryRelationType,
    created_at: DateTime<Utc>,
}

impl From<RelationRow> for EventMemoryRelation {
    fn from(row: RelationRow) -> Self {
        EventMemoryRelation {
            id: row.id,
            event_id: row.event_id,
            memory_id: row.memory_id,
            relation_type: row.relation_type,
            created_at: row.created_at,
        }
    }
}
