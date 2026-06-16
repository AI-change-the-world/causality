//! Profile Repository implementation
//!
//! Provides CRUD operations for SystemProfile entities with PostgreSQL.
//! Each SystemProfile is a business-system namespace for memories and events.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::domain::SystemProfile;
use crate::error::{AppError, AppResult};

/// Repository for SystemProfile CRUD operations
#[derive(Clone)]
pub struct ProfileRepository {
    pool: PgPool,
}

impl ProfileRepository {
    /// Create a new ProfileRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new system profile namespace.
    pub async fn create(&self, profile: &SystemProfile) -> AppResult<SystemProfile> {
        let row = sqlx::query_as::<_, ProfileRow>(
            r#"
            INSERT INTO system_profiles (
                id, name, description, purpose, domain, target_audience,
                event_categories, memory_focus, boundaries, extraction_prompt,
                created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING
                id, name, description, purpose, domain, target_audience,
                event_categories, memory_focus, boundaries, extraction_prompt,
                created_at, updated_at
            "#,
        )
        .bind(profile.id)
        .bind(&profile.name)
        .bind(&profile.description)
        .bind(&profile.purpose)
        .bind(&profile.domain)
        .bind(&profile.target_audience)
        .bind(&profile.event_categories)
        .bind(&profile.memory_focus)
        .bind(&profile.boundaries)
        .bind(&profile.extraction_prompt)
        .bind(profile.created_at)
        .bind(profile.updated_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            // Check for unique constraint violation on the human-readable system name.
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err.constraint() == Some("idx_system_profiles_name") {
                    return AppError::Validation(
                        "System profile already exists. Use update instead.".to_string(),
                    );
                }
            }
            AppError::Database(e)
        })?;

        Ok(row.into())
    }

    /// Get the first system profile.
    ///
    /// Prefer `get_by_id` when handling memory/event data so system namespace
    /// boundaries stay explicit.
    pub async fn get(&self) -> AppResult<Option<SystemProfile>> {
        let row = sqlx::query_as::<_, ProfileRow>(
            r#"
            SELECT
                id, name, description, purpose, domain, target_audience,
                event_categories, memory_focus, boundaries, extraction_prompt,
                created_at, updated_at
            FROM system_profiles
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    /// Get a system profile by ID.
    pub async fn get_by_id(&self, id: Uuid) -> AppResult<Option<SystemProfile>> {
        let row = sqlx::query_as::<_, ProfileRow>(
            r#"
            SELECT
                id, name, description, purpose, domain, target_audience,
                event_categories, memory_focus, boundaries, extraction_prompt,
                created_at, updated_at
            FROM system_profiles
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    /// Get a system profile by name.
    pub async fn get_by_name(&self, name: &str) -> AppResult<Option<SystemProfile>> {
        let row = sqlx::query_as::<_, ProfileRow>(
            r#"
            SELECT
                id, name, description, purpose, domain, target_audience,
                event_categories, memory_focus, boundaries, extraction_prompt,
                created_at, updated_at
            FROM system_profiles
            WHERE name = $1
            ORDER BY created_at ASC
            LIMIT 1
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    /// List all system profiles.
    pub async fn list(&self) -> AppResult<Vec<SystemProfile>> {
        let rows = sqlx::query_as::<_, ProfileRow>(
            r#"
            SELECT
                id, name, description, purpose, domain, target_audience,
                event_categories, memory_focus, boundaries, extraction_prompt,
                created_at, updated_at
            FROM system_profiles
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Update the system profile
    ///
    /// Returns an error if no profile exists.
    pub async fn update(&self, profile: &SystemProfile) -> AppResult<SystemProfile> {
        let row = sqlx::query_as::<_, ProfileRow>(
            r#"
            UPDATE system_profiles
            SET name = $2,
                description = $3,
                purpose = $4,
                domain = $5,
                target_audience = $6,
                event_categories = $7,
                memory_focus = $8,
                boundaries = $9,
                extraction_prompt = $10,
                updated_at = $11
            WHERE id = $1
            RETURNING
                id, name, description, purpose, domain, target_audience,
                event_categories, memory_focus, boundaries, extraction_prompt,
                created_at, updated_at
            "#,
        )
        .bind(profile.id)
        .bind(&profile.name)
        .bind(&profile.description)
        .bind(&profile.purpose)
        .bind(&profile.domain)
        .bind(&profile.target_audience)
        .bind(&profile.event_categories)
        .bind(&profile.memory_focus)
        .bind(&profile.boundaries)
        .bind(&profile.extraction_prompt)
        .bind(profile.updated_at)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            AppError::Validation("System profile not found. Initialize first.".to_string())
        })?;

        Ok(row.into())
    }

    /// Check if any system profile exists.
    ///
    /// This is a convenience check for management flows. Namespace-aware code
    /// should query by profile ID.
    pub async fn exists(&self) -> AppResult<bool> {
        let count = sqlx::query_scalar::<_, i64>(r#"SELECT COUNT(*) FROM system_profiles"#)
            .fetch_one(&self.pool)
            .await?;

        Ok(count > 0)
    }
}

/// Internal row type for SystemProfile
#[derive(Debug, FromRow)]
struct ProfileRow {
    id: Uuid,
    name: String,
    description: String,
    purpose: String,
    domain: String,
    target_audience: String,
    event_categories: Vec<String>,
    memory_focus: Vec<String>,
    boundaries: Vec<String>,
    extraction_prompt: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<ProfileRow> for SystemProfile {
    fn from(row: ProfileRow) -> Self {
        SystemProfile {
            id: row.id,
            name: row.name,
            description: row.description,
            purpose: row.purpose,
            domain: row.domain,
            target_audience: row.target_audience,
            event_categories: row.event_categories,
            memory_focus: row.memory_focus,
            boundaries: row.boundaries,
            extraction_prompt: row.extraction_prompt,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
