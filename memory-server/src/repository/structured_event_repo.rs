//! StructuredEvent Repository implementation
//!
//! Provides CRUD operations for StructuredEvent entities with PostgreSQL.
//! Each event can have at most one structured event record (1:1 relationship).

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::domain::StructuredEvent;
use crate::error::{AppError, AppResult};

/// Repository for StructuredEvent CRUD operations
#[derive(Clone)]
pub struct StructuredEventRepository {
    pool: PgPool,
}

impl StructuredEventRepository {
    /// Create a new StructuredEventRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new structured event
    ///
    /// Returns an error if a structured event already exists for the given event_id.
    pub async fn create(&self, structured_event: &StructuredEvent) -> AppResult<StructuredEvent> {
        let row = sqlx::query_as::<_, StructuredEventRow>(
            r#"
            INSERT INTO structured_events (
                id, event_id, time_element, location_element, actor_element,
                cause_element, process_element, result_element,
                background_element, details_element, category, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING
                id, event_id, time_element, location_element, actor_element,
                cause_element, process_element, result_element,
                background_element, details_element, category, created_at
            "#,
        )
        .bind(structured_event.id)
        .bind(structured_event.event_id)
        .bind(&structured_event.time_element)
        .bind(&structured_event.location_element)
        .bind(&structured_event.actor_element)
        .bind(&structured_event.cause_element)
        .bind(&structured_event.process_element)
        .bind(&structured_event.result_element)
        .bind(&structured_event.background_element)
        .bind(&structured_event.details_element)
        .bind(&structured_event.category)
        .bind(structured_event.created_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            // Check for unique constraint violation (one structured event per event)
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err.constraint() == Some("structured_events_event_id_key") {
                    return AppError::Validation(format!(
                        "Structured event already exists for event_id: {}",
                        structured_event.event_id
                    ));
                }
            }
            AppError::Database(e)
        })?;

        Ok(row.into())
    }

    /// Get a structured event by its event_id
    ///
    /// Returns None if no structured event exists for the given event_id.
    pub async fn get_by_event_id(&self, event_id: Uuid) -> AppResult<Option<StructuredEvent>> {
        let row = sqlx::query_as::<_, StructuredEventRow>(
            r#"
            SELECT
                id, event_id, time_element, location_element, actor_element,
                cause_element, process_element, result_element,
                background_element, details_element, category, created_at
            FROM structured_events
            WHERE event_id = $1
            "#,
        )
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }
}

/// Internal row type for StructuredEvent
#[derive(Debug, FromRow)]
struct StructuredEventRow {
    id: Uuid,
    event_id: Uuid,
    time_element: Option<String>,
    location_element: Option<String>,
    actor_element: String,
    cause_element: Option<String>,
    process_element: Option<String>,
    result_element: Option<String>,
    background_element: Option<String>,
    details_element: Option<String>,
    category: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<StructuredEventRow> for StructuredEvent {
    fn from(row: StructuredEventRow) -> Self {
        StructuredEvent {
            id: row.id,
            event_id: row.event_id,
            time_element: row.time_element,
            location_element: row.location_element,
            actor_element: row.actor_element,
            cause_element: row.cause_element,
            process_element: row.process_element,
            result_element: row.result_element,
            background_element: row.background_element,
            details_element: row.details_element,
            category: row.category,
            created_at: row.created_at,
        }
    }
}
