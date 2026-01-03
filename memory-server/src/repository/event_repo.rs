//! Event Repository implementation
//!
//! Provides CRUD operations for Event entities and event-memory relations.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::domain::{Event, EventMemoryRelation, EventMemoryRelationType, ScopeType};
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
                id, owner_id, content, context, event_type,
                scope_type, scope_id, scene, summary, processed,
                event_time, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING
                id, owner_id, content, context, event_type,
                scope_type, scope_id, scene, summary, processed,
                event_time, created_at
            "#,
        )
        .bind(event.id)
        .bind(&event.owner_id)
        .bind(&event.content)
        .bind(&event.context)
        .bind(&event.event_type)
        .bind(&event.scope_type)
        .bind(&event.scope_id)
        .bind(&event.scene)
        .bind(&event.summary)
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
                id, owner_id, content, context, event_type,
                scope_type, scope_id, scene, summary, processed,
                event_time, created_at
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
                id, owner_id, content, context, event_type,
                scope_type, scope_id, scene, summary, processed,
                event_time, created_at
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
                id, owner_id, content, context, event_type,
                scope_type, scope_id, scene, summary, processed,
                event_time, created_at
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
        scope_type: &ScopeType,
        scope_id: &str,
        limit: i64,
    ) -> AppResult<Vec<Event>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT
                id, owner_id, content, context, event_type,
                scope_type, scope_id, scene, summary, processed,
                event_time, created_at
            FROM events
            WHERE owner_id = $1 AND scope_type = $2 AND scope_id = $3
            ORDER BY event_time DESC
            LIMIT $4
            "#,
        )
        .bind(owner_id)
        .bind(scope_type)
        .bind(scope_id)
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
                id, event_id, memory_id, relation_type, similarity_score, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (event_id, memory_id, relation_type) DO UPDATE
            SET similarity_score = COALESCE(EXCLUDED.similarity_score, event_memory_relations.similarity_score)
            RETURNING id, event_id, memory_id, relation_type, similarity_score, created_at
            "#,
        )
        .bind(relation.id)
        .bind(relation.event_id)
        .bind(relation.memory_id)
        .bind(&relation.relation_type)
        .bind(relation.similarity_score)
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
            SELECT id, event_id, memory_id, relation_type, similarity_score, created_at
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
            SELECT id, event_id, memory_id, relation_type, similarity_score, created_at
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
            SELECT e.id, e.owner_id, e.content, e.context, e.event_type,
                   e.scope_type, e.scope_id, e.scene, e.summary, e.processed,
                   e.event_time, e.created_at
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
}

/// Internal row type for Event
#[derive(Debug, FromRow)]
struct EventRow {
    id: Uuid,
    owner_id: String,
    content: String,
    context: Option<String>,
    event_type: Option<String>,
    scope_type: ScopeType,
    scope_id: String,
    scene: String,
    summary: Option<String>,
    processed: bool,
    event_time: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

impl From<EventRow> for Event {
    fn from(row: EventRow) -> Self {
        Event {
            id: row.id,
            owner_id: row.owner_id,
            content: row.content,
            context: row.context,
            event_type: row.event_type,
            scope_type: row.scope_type,
            scope_id: row.scope_id,
            scene: row.scene,
            summary: row.summary,
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
    similarity_score: Option<f32>,
    created_at: DateTime<Utc>,
}

impl From<RelationRow> for EventMemoryRelation {
    fn from(row: RelationRow) -> Self {
        EventMemoryRelation {
            id: row.id,
            event_id: row.event_id,
            memory_id: row.memory_id,
            relation_type: row.relation_type,
            similarity_score: row.similarity_score,
            created_at: row.created_at,
        }
    }
}
