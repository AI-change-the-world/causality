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

use crate::domain::{Event, EventMemoryRelation, EventMemoryRelationType};
use crate::error::{AppError, AppResult};

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
                summary, source, processed, event_time, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            RETURNING
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processed, event_time, created_at
            "#,
        )
        .bind(event.id)
        .bind(event.profile_id)
        .bind(&event.owner_id)
        .bind(&event.scope_id)
        .bind(&event.content)
        .bind(&event.context)
        .bind(&event.summary)
        .bind(&event.source)
        .bind(event.processed)
        .bind(event.event_time)
        .bind(event.created_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into())
    }

    /// Get an event by ID
    pub async fn get_by_id(&self, id: Uuid) -> AppResult<Event> {
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processed, event_time, created_at
            FROM events
            WHERE id = $1
            "#,
        )
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
            SET processed = true, summary = $2
            WHERE id = $1
            RETURNING
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processed, event_time, created_at
            "#,
        )
        .bind(id)
        .bind(summary)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::EventNotFound(id))?;

        Ok(row.into())
    }

    /// Find unprocessed events for a given owner
    pub async fn find_unprocessed(&self, owner_id: &str, limit: i64) -> AppResult<Vec<Event>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processed, event_time, created_at
            FROM events
            WHERE owner_id = $1 AND processed = false
            ORDER BY event_time ASC
            LIMIT $2
            "#,
        )
        .bind(owner_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Find events by owner and scope
    pub async fn find_by_scope(
        &self,
        owner_id: &str,
        scope_id: Option<&str>,
        limit: i64,
    ) -> AppResult<Vec<Event>> {
        let rows = if let Some(scope) = scope_id {
            sqlx::query_as::<_, EventRow>(
                r#"
                SELECT
                    id, profile_id, owner_id, scope_id, content, context,
                    summary, source, processed, event_time, created_at
                FROM events
                WHERE owner_id = $1 AND scope_id = $2
                ORDER BY event_time DESC
                LIMIT $3
                "#,
            )
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
                    summary, source, processed, event_time, created_at
                FROM events
                WHERE owner_id = $1 AND scope_id IS NULL
                ORDER BY event_time DESC
                LIMIT $2
                "#,
            )
            .bind(owner_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Find events by owner (all scopes)
    pub async fn find_by_owner(&self, owner_id: &str, limit: i64) -> AppResult<Vec<Event>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT
                id, profile_id, owner_id, scope_id, content, context,
                summary, source, processed, event_time, created_at
            FROM events
            WHERE owner_id = $1
            ORDER BY event_time DESC
            LIMIT $2
            "#,
        )
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
    pub async fn get_supporting_events(&self, memory_id: Uuid) -> AppResult<Vec<Event>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT e.id, e.owner_id, e.scope_id, e.content, e.context,
                   e.summary, e.source, e.processed, e.event_time, e.created_at
            FROM events e
            JOIN event_memory_relations emr ON e.id = emr.event_id
            WHERE emr.memory_id = $1 AND emr.relation_type IN ('created_from', 'reinforced_by')
            ORDER BY e.event_time DESC
            "#,
        )
        .bind(memory_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Get the source event for a memory (created_from relation)
    pub async fn get_source_event(&self, memory_id: Uuid) -> AppResult<Option<Event>> {
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT e.id, e.owner_id, e.scope_id, e.content, e.context,
                   e.summary, e.source, e.processed, e.event_time, e.created_at
            FROM events e
            JOIN event_memory_relations emr ON e.id = emr.event_id
            WHERE emr.memory_id = $1 AND emr.relation_type = 'created_from'
            LIMIT 1
            "#,
        )
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
    processed: bool,
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
            source: row.source,
            processed: row.processed,
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
