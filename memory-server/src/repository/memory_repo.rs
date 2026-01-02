//! Memory Repository implementation
//!
//! Provides CRUD operations for Memory entities with PostgreSQL.
//! Includes full-text search support using PostgreSQL tsvector/tsquery.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, Row};
use uuid::Uuid;

use crate::domain::{
    EmbeddingStatus, Layer, Memory, MemoryCategory, ProcessingStatus, ScopeType, Status, UpdateMode,
};
use crate::error::{AppError, AppResult};

/// Input for updating a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateMemoryInput {
    /// Update mode (append, merge, supersede)
    pub mode: UpdateMode,
    /// New content (optional)
    pub content: Option<String>,
    /// New importance value (optional)
    pub importance: Option<f32>,
    /// New confidence value (optional)
    pub confidence: Option<f32>,
    /// New TTL in seconds (optional)
    pub ttl_seconds: Option<i64>,
}

/// Result of a full-text search query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullTextSearchResult {
    /// The memory record
    pub memory: Memory,
    /// Full-text search rank/score
    pub text_match_score: f32,
    /// Highlighted snippets from the content
    pub highlights: Option<Vec<String>>,
}

/// Options for full-text search
#[derive(Debug, Clone, Default)]
pub struct FullTextSearchOptions {
    /// Whether to return highlighted snippets
    pub highlight: bool,
    /// Maximum number of results
    pub limit: Option<usize>,
}

/// Repository for Memory CRUD operations
#[derive(Clone)]
pub struct MemoryRepository {
    pool: PgPool,
}

impl MemoryRepository {
    /// Create a new MemoryRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new memory in the database
    pub async fn create(&self, memory: &Memory) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            INSERT INTO memories (
                id, layer, scope_type, scope_id, scene, status, content,
                importance, confidence, hit_count, last_hit_at, ttl_seconds,
                expires_at, event_source, event_time, embedding_status,
                embedding_provider, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
            RETURNING
                id, layer, scope_type, scope_id, scene, status, content,
                importance, confidence, hit_count, last_hit_at, ttl_seconds,
                expires_at, event_source, event_time, embedding_status,
                embedding_provider, created_at, updated_at
            "#,
        )
        .bind(memory.id)
        .bind(&memory.layer)
        .bind(&memory.scope_type)
        .bind(&memory.scope_id)
        .bind(&memory.scene)
        .bind(&memory.status)
        .bind(&memory.content)
        .bind(memory.importance)
        .bind(memory.confidence)
        .bind(memory.hit_count)
        .bind(memory.last_hit_at)
        .bind(memory.ttl_seconds)
        .bind(memory.expires_at)
        .bind(&memory.event_source)
        .bind(memory.event_time)
        .bind(&memory.embedding_status)
        .bind(&memory.embedding_provider)
        .bind(memory.created_at)
        .bind(memory.updated_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into())
    }

    /// Get a memory by ID
    pub async fn get_by_id(&self, id: Uuid) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT
                id, layer, scope_type, scope_id, scene, status, content,
                importance, confidence, hit_count, last_hit_at, ttl_seconds,
                expires_at, event_source, event_time, embedding_status,
                embedding_provider, created_at, updated_at
            FROM memories
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Update a memory with the specified mode
    pub async fn update(&self, id: Uuid, input: &UpdateMemoryInput) -> AppResult<Memory> {
        // First get the existing memory
        let existing = self.get_by_id(id).await?;

        // Calculate new content based on update mode
        let new_content = match (&input.content, input.mode) {
            (Some(new), UpdateMode::Append) => {
                format!("{}\n\n{}", existing.content, new)
            }
            (Some(new), UpdateMode::Merge) => {
                format!("{}\n\n---\n\n{}", existing.content, new)
            }
            (Some(new), UpdateMode::Supersede) => new.clone(),
            (None, _) => existing.content.clone(),
        };

        // Calculate new importance and confidence
        let new_importance = input.importance.unwrap_or(existing.importance);
        let new_confidence = input.confidence.unwrap_or(existing.confidence);

        // Calculate new TTL and expires_at
        let (new_ttl, new_expires_at) = if let Some(ttl) = input.ttl_seconds {
            let expires = Utc::now() + chrono::Duration::seconds(ttl);
            (Some(ttl), Some(expires))
        } else {
            (existing.ttl_seconds, existing.expires_at)
        };

        // Determine if content changed (need to re-embed)
        let content_changed = input.content.is_some();
        let new_embedding_status = if content_changed {
            EmbeddingStatus::Pending
        } else {
            existing.embedding_status
        };

        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            UPDATE memories
            SET content = $2,
                importance = $3,
                confidence = $4,
                ttl_seconds = $5,
                expires_at = $6,
                embedding_status = $7,
                updated_at = NOW()
            WHERE id = $1
            RETURNING
                id, layer, scope_type, scope_id, scene, status, content,
                importance, confidence, hit_count, last_hit_at, ttl_seconds,
                expires_at, event_source, event_time, embedding_status,
                embedding_provider, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&new_content)
        .bind(new_importance)
        .bind(new_confidence)
        .bind(new_ttl)
        .bind(new_expires_at)
        .bind(&new_embedding_status)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into())
    }

    /// Delete (archive) a memory by ID
    pub async fn delete(&self, id: Uuid) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            UPDATE memories
            SET status = 'archived', updated_at = NOW()
            WHERE id = $1
            RETURNING
                id, layer, scope_type, scope_id, scene, status, content,
                importance, confidence, hit_count, last_hit_at, ttl_seconds,
                expires_at, event_source, event_time, embedding_status,
                embedding_provider, created_at, updated_at
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Update hit count and last_hit_at for a memory
    pub async fn record_hit(&self, id: Uuid) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            UPDATE memories
            SET hit_count = hit_count + 1,
                last_hit_at = NOW(),
                status = CASE WHEN status = 'cooldown' THEN 'active'::status ELSE status END,
                updated_at = NOW()
            WHERE id = $1
            RETURNING
                id, layer, scope_type, scope_id, scene, status, content,
                importance, confidence, hit_count, last_hit_at, ttl_seconds,
                expires_at, event_source, event_time, embedding_status,
                embedding_provider, created_at, updated_at
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Update the status of a memory
    pub async fn update_status(&self, id: Uuid, status: Status) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            UPDATE memories
            SET status = $2, updated_at = NOW()
            WHERE id = $1
            RETURNING
                id, layer, scope_type, scope_id, scene, status, content,
                importance, confidence, hit_count, last_hit_at, ttl_seconds,
                expires_at, event_source, event_time, embedding_status,
                embedding_provider, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&status)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Update the embedding status of a memory
    pub async fn update_embedding_status(
        &self,
        id: Uuid,
        embedding_status: EmbeddingStatus,
    ) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            UPDATE memories
            SET embedding_status = $2, updated_at = NOW()
            WHERE id = $1
            RETURNING
                id, layer, scope_type, scope_id, scene, status, content,
                importance, confidence, hit_count, last_hit_at, ttl_seconds,
                expires_at, event_source, event_time, embedding_status,
                embedding_provider, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&embedding_status)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Find memories that have expired
    pub async fn find_expired(&self) -> AppResult<Vec<Memory>> {
        let rows = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT
                id, layer, scope_type, scope_id, scene, status, content,
                importance, confidence, hit_count, last_hit_at, ttl_seconds,
                expires_at, event_source, event_time, embedding_status,
                embedding_provider, created_at, updated_at
            FROM memories
            WHERE expires_at IS NOT NULL
              AND expires_at < NOW()
              AND status NOT IN ('archived', 'ignored')
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Find memories that should transition to cooldown
    pub async fn find_cooldown_candidates(
        &self,
        session_threshold_secs: i64,
        task_threshold_secs: i64,
        longterm_threshold_secs: i64,
    ) -> AppResult<Vec<Memory>> {
        let rows = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT
                id, layer, scope_type, scope_id, scene, status, content,
                importance, confidence, hit_count, last_hit_at, ttl_seconds,
                expires_at, event_source, event_time, embedding_status,
                embedding_provider, created_at, updated_at
            FROM memories
            WHERE status IN ('active', 'stable', 'candidate')
              AND (
                (layer = 'session' AND COALESCE(last_hit_at, created_at) < NOW() - INTERVAL '1 second' * $1)
                OR (layer = 'task' AND COALESCE(last_hit_at, created_at) < NOW() - INTERVAL '1 second' * $2)
                OR (layer = 'long_term' AND COALESCE(last_hit_at, created_at) < NOW() - INTERVAL '1 second' * $3)
              )
            "#,
        )
        .bind(session_threshold_secs as f64)
        .bind(task_threshold_secs as f64)
        .bind(longterm_threshold_secs as f64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Find memories with structured filters for retrieval
    ///
    /// Supports filtering by:
    /// - scope_type and scope_id
    /// - layers (multiple)
    /// - scenes (multiple)
    /// - event_source prefix
    /// - categories (multiple, LLM-classified)
    /// - tags (multiple, extracted keywords)
    /// - excludes ignored and archived statuses
    pub async fn find_for_retrieval(
        &self,
        scope_type: Option<&ScopeType>,
        scope_id: Option<&str>,
        layers: Option<&[Layer]>,
        scenes: Option<&[String]>,
        event_source_prefix: Option<&str>,
        categories: Option<&[MemoryCategory]>,
        tags: Option<&[String]>,
    ) -> AppResult<Vec<Memory>> {
        // Build the query dynamically
        // Note: Using a simpler approach with optional filters
        let rows = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT
                id, layer, scope_type, scope_id, scene, status, content,
                raw_content, category, tags, importance, confidence, hit_count, 
                last_hit_at, ttl_seconds, expires_at, event_source, event_time, 
                embedding_status, embedding_provider, processing_status, llm_provider,
                created_at, updated_at
            FROM memories
            WHERE status NOT IN ('ignored', 'archived')
              AND ($1::scope_type IS NULL OR scope_type = $1)
              AND ($2::text IS NULL OR scope_id = $2)
              AND ($3::text IS NULL OR event_source LIKE $3 || '%')
            ORDER BY updated_at DESC
            LIMIT 1000
            "#,
        )
        .bind(scope_type)
        .bind(scope_id)
        .bind(event_source_prefix)
        .fetch_all(&self.pool)
        .await?;

        let mut memories: Vec<Memory> = rows.into_iter().map(Into::into).collect();

        // Apply layer filter in memory (sqlx doesn't support array parameters easily)
        if let Some(layer_filter) = layers {
            if !layer_filter.is_empty() {
                memories.retain(|m| layer_filter.contains(&m.layer));
            }
        }

        // Apply scene filter in memory
        if let Some(scene_filter) = scenes {
            if !scene_filter.is_empty() {
                memories.retain(|m| scene_filter.contains(&m.scene));
            }
        }

        // Apply category filter in memory
        if let Some(category_filter) = categories {
            if !category_filter.is_empty() {
                memories.retain(|m| {
                    m.category
                        .as_ref()
                        .map(|c| category_filter.contains(c))
                        .unwrap_or(false)
                });
            }
        }

        // Apply tags filter in memory (match if any tag matches)
        if let Some(tag_filter) = tags {
            if !tag_filter.is_empty() {
                memories.retain(|m| {
                    m.tags
                        .as_ref()
                        .map(|memory_tags| tag_filter.iter().any(|t| memory_tags.contains(t)))
                        .unwrap_or(false)
                });
            }
        }

        Ok(memories)
    }

    /// Get memories by a list of IDs
    pub async fn get_by_ids(&self, ids: &[Uuid]) -> AppResult<Vec<Memory>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }

        let rows = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT
                id, layer, scope_type, scope_id, scene, status, content,
                importance, confidence, hit_count, last_hit_at, ttl_seconds,
                expires_at, event_source, event_time, embedding_status,
                embedding_provider, created_at, updated_at
            FROM memories
            WHERE id = ANY($1)
            "#,
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Perform full-text search on memory content using PostgreSQL tsquery
    ///
    /// Supports:
    /// - Basic word matching
    /// - Phrase matching (words in sequence)
    /// - Ranking by relevance using ts_rank
    /// - Optional highlighting of matched terms
    ///
    /// The query is converted to a tsquery using plainto_tsquery for simple queries
    /// or to_tsquery for advanced queries with operators.
    pub async fn fulltext_search(
        &self,
        query: &str,
        scope_type: Option<&ScopeType>,
        scope_id: Option<&str>,
        layers: Option<&[Layer]>,
        scenes: Option<&[String]>,
        event_source_prefix: Option<&str>,
        options: &FullTextSearchOptions,
    ) -> AppResult<Vec<FullTextSearchResult>> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }

        let limit = options.limit.unwrap_or(100).min(1000) as i64;

        // Use plainto_tsquery for simple word-based search
        // This handles user input safely without requiring special syntax
        let rows = sqlx::query(
            r#"
            SELECT
                id, layer, scope_type, scope_id, scene, status, content,
                importance, confidence, hit_count, last_hit_at, ttl_seconds,
                expires_at, event_source, event_time, embedding_status,
                embedding_provider, created_at, updated_at,
                ts_rank(content_tsv, plainto_tsquery('simple', $1)) as rank,
                CASE WHEN $6 THEN
                    ts_headline('simple', content, plainto_tsquery('simple', $1),
                        'StartSel=<mark>, StopSel=</mark>, MaxWords=35, MinWords=15, MaxFragments=3')
                ELSE NULL END as headline
            FROM memories
            WHERE content_tsv @@ plainto_tsquery('simple', $1)
              AND status NOT IN ('ignored', 'archived')
              AND ($2::scope_type IS NULL OR scope_type = $2)
              AND ($3::text IS NULL OR scope_id = $3)
              AND ($4::text IS NULL OR event_source LIKE $4 || '%')
            ORDER BY rank DESC
            LIMIT $5
            "#,
        )
        .bind(query)
        .bind(scope_type)
        .bind(scope_id)
        .bind(event_source_prefix)
        .bind(limit)
        .bind(options.highlight)
        .fetch_all(&self.pool)
        .await?;

        let mut results: Vec<FullTextSearchResult> = Vec::with_capacity(rows.len());

        for row in rows {
            let memory = Memory {
                id: row.get("id"),
                layer: row.get("layer"),
                scope_type: row.get("scope_type"),
                scope_id: row.get("scope_id"),
                scene: row.get("scene"),
                status: row.get("status"),
                content: row.get("content"),
                raw_content: row.get("raw_content"),
                category: row.get("category"),
                tags: row.get("tags"),
                importance: row.get("importance"),
                confidence: row.get("confidence"),
                hit_count: row.get("hit_count"),
                last_hit_at: row.get("last_hit_at"),
                ttl_seconds: row.get("ttl_seconds"),
                expires_at: row.get("expires_at"),
                event_source: row.get("event_source"),
                event_time: row.get("event_time"),
                embedding_status: row.get("embedding_status"),
                embedding_provider: row.get("embedding_provider"),
                processing_status: row.get("processing_status"),
                llm_provider: row.get("llm_provider"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            };

            let rank: f32 = row.get("rank");
            let headline: Option<String> = row.get("headline");

            // Parse highlights from headline (split by fragment separator)
            let highlights = headline.map(|h| {
                h.split(" ... ")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });

            // Apply layer filter in memory
            if let Some(layer_filter) = layers {
                if !layer_filter.is_empty() && !layer_filter.contains(&memory.layer) {
                    continue;
                }
            }

            // Apply scene filter in memory
            if let Some(scene_filter) = scenes {
                if !scene_filter.is_empty() && !scene_filter.contains(&memory.scene) {
                    continue;
                }
            }

            results.push(FullTextSearchResult {
                memory,
                text_match_score: rank,
                highlights,
            });
        }

        Ok(results)
    }

    /// Get full-text search score for a specific memory and query
    ///
    /// Returns the ts_rank score for the given memory ID and query.
    /// Returns 0.0 if the memory doesn't match the query.
    pub async fn get_fulltext_score(&self, memory_id: Uuid, query: &str) -> AppResult<f32> {
        if query.trim().is_empty() {
            return Ok(0.0);
        }

        let result = sqlx::query_scalar::<_, f32>(
            r#"
            SELECT COALESCE(ts_rank(content_tsv, plainto_tsquery('simple', $2)), 0.0)
            FROM memories
            WHERE id = $1
            "#,
        )
        .bind(memory_id)
        .bind(query)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.unwrap_or(0.0))
    }

    /// Get full-text search scores for multiple memories
    ///
    /// Returns a map of memory_id -> ts_rank score for the given query.
    pub async fn get_fulltext_scores(
        &self,
        memory_ids: &[Uuid],
        query: &str,
    ) -> AppResult<std::collections::HashMap<Uuid, f32>> {
        if memory_ids.is_empty() || query.trim().is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let rows = sqlx::query(
            r#"
            SELECT id, ts_rank(content_tsv, plainto_tsquery('simple', $2)) as rank
            FROM memories
            WHERE id = ANY($1)
              AND content_tsv @@ plainto_tsquery('simple', $2)
            "#,
        )
        .bind(memory_ids)
        .bind(query)
        .fetch_all(&self.pool)
        .await?;

        let mut scores = std::collections::HashMap::new();
        for row in rows {
            let id: Uuid = row.get("id");
            let rank: f32 = row.get("rank");
            scores.insert(id, rank);
        }

        Ok(scores)
    }

    /// Get highlighted snippets for a memory matching a query
    ///
    /// Returns highlighted text snippets showing where the query matches.
    pub async fn get_highlights(
        &self,
        memory_id: Uuid,
        query: &str,
    ) -> AppResult<Option<Vec<String>>> {
        if query.trim().is_empty() {
            return Ok(None);
        }

        let result = sqlx::query_scalar::<_, Option<String>>(
            r#"
            SELECT ts_headline('simple', content, plainto_tsquery('simple', $2),
                'StartSel=<mark>, StopSel=</mark>, MaxWords=35, MinWords=15, MaxFragments=3')
            FROM memories
            WHERE id = $1
              AND content_tsv @@ plainto_tsquery('simple', $2)
            "#,
        )
        .bind(memory_id)
        .bind(query)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.flatten().map(|h| {
            h.split(" ... ")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }))
    }
}

/// Internal row type for sqlx mapping
#[derive(Debug, FromRow)]
struct MemoryRow {
    id: Uuid,
    layer: Layer,
    scope_type: ScopeType,
    scope_id: String,
    scene: String,
    status: Status,
    content: String,
    raw_content: Option<String>,
    category: Option<MemoryCategory>,
    tags: Option<Vec<String>>,
    importance: f32,
    confidence: f32,
    hit_count: i64,
    last_hit_at: Option<DateTime<Utc>>,
    ttl_seconds: Option<i64>,
    expires_at: Option<DateTime<Utc>>,
    event_source: Option<String>,
    event_time: Option<DateTime<Utc>>,
    embedding_status: EmbeddingStatus,
    embedding_provider: Option<String>,
    processing_status: ProcessingStatus,
    llm_provider: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<MemoryRow> for Memory {
    fn from(row: MemoryRow) -> Self {
        Memory {
            id: row.id,
            layer: row.layer,
            scope_type: row.scope_type,
            scope_id: row.scope_id,
            scene: row.scene,
            status: row.status,
            content: row.content,
            raw_content: row.raw_content,
            category: row.category,
            tags: row.tags,
            importance: row.importance,
            confidence: row.confidence,
            hit_count: row.hit_count,
            last_hit_at: row.last_hit_at,
            ttl_seconds: row.ttl_seconds,
            expires_at: row.expires_at,
            event_source: row.event_source,
            event_time: row.event_time,
            embedding_status: row.embedding_status,
            embedding_provider: row.embedding_provider,
            processing_status: row.processing_status,
            llm_provider: row.llm_provider,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
