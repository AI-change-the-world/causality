//! Memory Repository implementation
//!
//! Provides CRUD operations for Memory entities with PostgreSQL.
//! Includes full-text search support and version chain queries.
//!
//! In the new architecture:
//! - Memory content is immutable (conflicts create new versions)
//! - Version chain uses materialized path for O(1) queries
//! - LFU eviction replaces TTL-based expiration

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, Row};
use uuid::Uuid;

use crate::domain::{EmbeddingStatus, InferenceType, Memory, ProcessingStatus, Status};
use crate::error::{AppError, AppResult};

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

// SQL column list for Memory (used in multiple queries)
const MEMORY_COLUMNS: &str = r#"
    id, profile_id, owner_id, scope_id, content, category, tags, importance, confidence,
    root_memory_id, version_number, is_current_version, supersedes, superseded_by,
    is_global, hit_count, last_hit_at, decay_score, source_event_id,
    status, embedding_status, embedding_provider, processing_status, llm_provider,
    inference_type, inference_confidence, inference_reasoning,
    promoted_at, promotion_reason, created_at, updated_at
"#;

impl MemoryRepository {
    /// Create a new MemoryRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new memory in the database
    pub async fn create(&self, memory: &Memory) -> AppResult<Memory> {
        let inference_type_str = memory.inference_type.as_ref().map(|t| t.to_string());

        let row = sqlx::query_as::<_, MemoryRow>(
            &format!(
                r#"
                INSERT INTO memories (
                    id, profile_id, owner_id, scope_id, content, category, tags, importance, confidence,
                    root_memory_id, version_number, is_current_version, supersedes, superseded_by,
                    is_global, hit_count, last_hit_at, decay_score, source_event_id,
                    status, embedding_status, embedding_provider, processing_status, llm_provider,
                    inference_type, inference_confidence, inference_reasoning,
                    promoted_at, promotion_reason, created_at, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31)
                RETURNING {}
                "#,
                MEMORY_COLUMNS
            ),
        )
        .bind(memory.id)
        .bind(memory.profile_id)
        .bind(&memory.owner_id)
        .bind(&memory.scope_id)
        .bind(&memory.content)
        .bind(&memory.category)
        .bind(&memory.tags)
        .bind(memory.importance)
        .bind(memory.confidence)
        .bind(memory.root_memory_id)
        .bind(memory.version_number)
        .bind(memory.is_current_version)
        .bind(memory.supersedes)
        .bind(memory.superseded_by)
        .bind(memory.is_global)
        .bind(memory.hit_count)
        .bind(memory.last_hit_at)
        .bind(memory.decay_score)
        .bind(memory.source_event_id)
        .bind(&memory.status)
        .bind(&memory.embedding_status)
        .bind(&memory.embedding_provider)
        .bind(&memory.processing_status)
        .bind(&memory.llm_provider)
        .bind(&inference_type_str)
        .bind(memory.inference_confidence)
        .bind(&memory.inference_reasoning)
        .bind(memory.promoted_at)
        .bind(&memory.promotion_reason)
        .bind(memory.created_at)
        .bind(memory.updated_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into())
    }

    /// Get a memory by ID
    pub async fn get_by_id(&self, id: Uuid) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"SELECT {} FROM memories WHERE id = $1"#,
            MEMORY_COLUMNS
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Update mutable metadata fields of a memory
    pub async fn update_metadata(
        &self,
        id: Uuid,
        confidence: Option<f32>,
        decay_score: Option<f32>,
    ) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                UPDATE memories
                SET confidence = COALESCE($2, confidence),
                    decay_score = COALESCE($3, decay_score),
                    updated_at = NOW()
                WHERE id = $1
                RETURNING {}
                "#,
            MEMORY_COLUMNS
        ))
        .bind(id)
        .bind(confidence)
        .bind(decay_score)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Delete (archive) a memory by ID
    pub async fn delete(&self, id: Uuid) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                UPDATE memories
                SET status = 'archived', updated_at = NOW()
                WHERE id = $1
                RETURNING {}
                "#,
            MEMORY_COLUMNS
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Update hit count and last_hit_at for a memory
    pub async fn record_hit(&self, id: Uuid) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                UPDATE memories
                SET hit_count = hit_count + 1,
                    last_hit_at = NOW(),
                    status = CASE WHEN status = 'cooldown' THEN 'active'::status ELSE status END,
                    updated_at = NOW()
                WHERE id = $1
                RETURNING {}
                "#,
            MEMORY_COLUMNS
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Update the status of a memory
    pub async fn update_status(&self, id: Uuid, status: Status) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                UPDATE memories
                SET status = $2, updated_at = NOW()
                WHERE id = $1
                RETURNING {}
                "#,
            MEMORY_COLUMNS
        ))
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
        let row = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                UPDATE memories
                SET embedding_status = $2, updated_at = NOW()
                WHERE id = $1
                RETURNING {}
                "#,
            MEMORY_COLUMNS
        ))
        .bind(id)
        .bind(&embedding_status)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Update the embedding status and provider of a memory
    pub async fn update_embedding_status_and_provider(
        &self,
        id: Uuid,
        embedding_status: EmbeddingStatus,
        embedding_provider: Option<&str>,
    ) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                UPDATE memories
                SET embedding_status = $2, embedding_provider = $3, updated_at = NOW()
                WHERE id = $1
                RETURNING {}
                "#,
            MEMORY_COLUMNS
        ))
        .bind(id)
        .bind(&embedding_status)
        .bind(embedding_provider)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Mark a memory as superseded by a new version
    pub async fn update_superseded(&self, id: Uuid, superseded_by_id: Uuid) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                UPDATE memories
                SET superseded_by = $2,
                    is_current_version = false,
                    status = 'superseded',
                    updated_at = NOW()
                WHERE id = $1
                RETURNING {}
                "#,
            MEMORY_COLUMNS
        ))
        .bind(id)
        .bind(superseded_by_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Find memories by root_memory_id (version chain query)
    pub async fn find_by_root_memory_id(&self, root_id: Uuid) -> AppResult<Vec<Memory>> {
        let rows = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"SELECT {} FROM memories WHERE root_memory_id = $1 ORDER BY version_number ASC"#,
            MEMORY_COLUMNS
        ))
        .bind(root_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Get version history for a memory
    pub async fn get_version_history(&self, memory_id: Uuid) -> AppResult<Vec<Memory>> {
        let memory = self.get_by_id(memory_id).await?;
        let root_id = memory.root_memory_id.unwrap_or(memory.id);
        self.find_by_root_memory_id(root_id).await
    }

    /// Find the current version of a memory chain
    pub async fn find_current_version(&self, root_id: Uuid) -> AppResult<Option<Memory>> {
        let row = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"SELECT {} FROM memories WHERE root_memory_id = $1 AND is_current_version = true"#,
            MEMORY_COLUMNS
        ))
        .bind(root_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    /// Find memories that should transition to cooldown (LFU eviction)
    pub async fn find_cooldown_candidates(&self, threshold_days: i64) -> AppResult<Vec<Memory>> {
        let rows = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                SELECT {}
                FROM memories
                WHERE status = 'active'
                  AND is_current_version = true
                  AND COALESCE(last_hit_at, created_at) < NOW() - INTERVAL '1 day' * $1
                "#,
            MEMORY_COLUMNS
        ))
        .bind(threshold_days as f64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Find memories with structured filters for retrieval
    ///
    /// Scope logic:
    /// - If scope_id is NULL: returns all memories for this owner (no scope filter)
    /// - If scope_id is provided: returns matching scope_id OR global (if include_global is true)
    pub async fn find_for_retrieval(
        &self,
        owner_id: &str,
        scope_id: Option<&str>,
        category_prefix: Option<&str>,
        tags: Option<&[String]>,
        include_global: bool,
    ) -> AppResult<Vec<Memory>> {
        let rows = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                SELECT {}
                FROM memories
                WHERE owner_id = $1
                  AND is_current_version = true
                  AND status NOT IN ('superseded', 'archived')
                  AND (
                    $2::text IS NULL
                    OR scope_id = $2
                    OR ($3 = true AND is_global = true)
                  )
                  AND ($4::text IS NULL OR category LIKE $4 || '%')
                ORDER BY decay_score DESC, updated_at DESC
                LIMIT 1000
                "#,
            MEMORY_COLUMNS
        ))
        .bind(owner_id)
        .bind(scope_id)
        .bind(include_global)
        .bind(category_prefix)
        .fetch_all(&self.pool)
        .await?;

        let mut memories: Vec<Memory> = rows.into_iter().map(Into::into).collect();

        // Apply tags filter in memory
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

        let rows = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"SELECT {} FROM memories WHERE id = ANY($1)"#,
            MEMORY_COLUMNS
        ))
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Perform full-text search on memory content
    pub async fn fulltext_search(
        &self,
        query: &str,
        owner_id: &str,
        scope_id: Option<&str>,
        include_global: bool,
        options: &FullTextSearchOptions,
    ) -> AppResult<Vec<FullTextSearchResult>> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }

        let limit = options.limit.unwrap_or(100).min(1000) as i64;

        let rows = sqlx::query(
            r#"
            SELECT
                id, owner_id, scope_id, content, category, tags, importance, confidence,
                root_memory_id, version_number, is_current_version, supersedes, superseded_by,
                is_global, hit_count, last_hit_at, decay_score, source_event_id,
                status, embedding_status, embedding_provider, processing_status, llm_provider,
                inference_type, inference_confidence, inference_reasoning,
                promoted_at, promotion_reason, created_at, updated_at,
                ts_rank(content_tsv, plainto_tsquery('simple', $1)) as rank,
                CASE WHEN $5 THEN
                    ts_headline('simple', content, plainto_tsquery('simple', $1),
                        'StartSel=<mark>, StopSel=</mark>, MaxWords=35, MinWords=15, MaxFragments=3')
                ELSE NULL END as headline
            FROM memories
            WHERE content_tsv @@ plainto_tsquery('simple', $1)
              AND owner_id = $2
              AND is_current_version = true
              AND status NOT IN ('superseded', 'archived')
              AND (
                ($3::text IS NULL AND is_global = true)
                OR scope_id = $3
                OR ($6 = true AND is_global = true)
              )
            ORDER BY rank DESC
            LIMIT $4
            "#,
        )
        .bind(query)
        .bind(owner_id)
        .bind(scope_id)
        .bind(limit)
        .bind(options.highlight)
        .bind(include_global)
        .fetch_all(&self.pool)
        .await?;

        let mut results: Vec<FullTextSearchResult> = Vec::with_capacity(rows.len());

        for row in rows {
            let memory = Memory {
                id: row.get("id"),
                profile_id: row.get("profile_id"),
                owner_id: row.get("owner_id"),
                scope_id: row.get("scope_id"),
                content: row.get("content"),
                category: row.get("category"),
                tags: row.get("tags"),
                importance: row.get("importance"),
                confidence: row.get("confidence"),
                root_memory_id: row.get("root_memory_id"),
                version_number: row.get("version_number"),
                is_current_version: row.get("is_current_version"),
                supersedes: row.get("supersedes"),
                superseded_by: row.get("superseded_by"),
                is_global: row.get("is_global"),
                hit_count: row.get("hit_count"),
                last_hit_at: row.get("last_hit_at"),
                decay_score: row.get("decay_score"),
                source_event_id: row.get("source_event_id"),
                status: row.get("status"),
                embedding_status: row.get("embedding_status"),
                embedding_provider: row.get("embedding_provider"),
                processing_status: row.get("processing_status"),
                llm_provider: row.get("llm_provider"),
                inference_type: row.get("inference_type"),
                inference_confidence: row.get("inference_confidence"),
                inference_reasoning: row.get("inference_reasoning"),
                promoted_at: row.get("promoted_at"),
                promotion_reason: row.get("promotion_reason"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            };

            let rank: f32 = row.get("rank");
            let headline: Option<String> = row.get("headline");

            let highlights = headline.map(|h| {
                h.split(" ... ")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });

            results.push(FullTextSearchResult {
                memory,
                text_match_score: rank,
                highlights,
            });
        }

        Ok(results)
    }

    /// Get full-text search score for a specific memory and query
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

    /// Promote a memory to global status
    pub async fn promote_to_global(&self, id: Uuid, reason: &str) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                UPDATE memories
                SET is_global = true,
                    scope_id = NULL,
                    promoted_at = NOW(),
                    promotion_reason = $2,
                    updated_at = NOW()
                WHERE id = $1
                RETURNING {}
                "#,
            MEMORY_COLUMNS
        ))
        .bind(id)
        .bind(reason)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Find memories eligible for promotion to global
    pub async fn find_global_promotion_candidates(
        &self,
        min_scope_diversity: i32,
        min_reinforcements: i32,
        min_confidence: f32,
        min_age_hours: i64,
    ) -> AppResult<Vec<Memory>> {
        let rows = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                SELECT {}
                FROM memories m
                WHERE m.is_global = false
                  AND m.is_current_version = true
                  AND m.promoted_at IS NULL
                  AND m.status = 'active'
                  AND m.confidence >= $3
                  AND m.created_at < NOW() - INTERVAL '1 hour' * $4
                  AND (
                    SELECT COUNT(DISTINCT e.scope_id)
                    FROM event_memory_relations emr
                    JOIN events e ON emr.event_id = e.id
                    WHERE emr.memory_id = m.id
                      AND emr.relation_type = 'reinforced_by'
                  ) >= $1
                  AND (
                    SELECT COUNT(*)
                    FROM event_memory_relations emr
                    WHERE emr.memory_id = m.id
                      AND emr.relation_type = 'reinforced_by'
                  ) >= $2
                ORDER BY m.confidence DESC, m.hit_count DESC
                "#,
            MEMORY_COLUMNS
        ))
        .bind(min_scope_diversity as i64)
        .bind(min_reinforcements as i64)
        .bind(min_confidence)
        .bind(min_age_hours as f64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Find similar memories by content for deduplication/reinforcement
    pub async fn find_similar_by_content(
        &self,
        owner_id: &str,
        scope_id: Option<&str>,
        content: &str,
        include_global: bool,
        limit: i64,
    ) -> AppResult<Vec<(Memory, f32)>> {
        if content.trim().is_empty() {
            return Ok(vec![]);
        }

        let rows = sqlx::query(
            r#"
            SELECT
                id, owner_id, scope_id, content, category, tags, importance, confidence,
                root_memory_id, version_number, is_current_version, supersedes, superseded_by,
                is_global, hit_count, last_hit_at, decay_score, source_event_id,
                status, embedding_status, embedding_provider, processing_status, llm_provider,
                inference_type, inference_confidence, inference_reasoning,
                promoted_at, promotion_reason, created_at, updated_at,
                ts_rank(content_tsv, plainto_tsquery('simple', $2)) as rank
            FROM memories
            WHERE owner_id = $1
              AND is_current_version = true
              AND status NOT IN ('superseded', 'archived')
              AND content_tsv @@ plainto_tsquery('simple', $2)
              AND (
                ($3::text IS NULL AND is_global = true)
                OR scope_id = $3
                OR ($4 = true AND is_global = true)
              )
            ORDER BY rank DESC
            LIMIT $5
            "#,
        )
        .bind(owner_id)
        .bind(content)
        .bind(scope_id)
        .bind(include_global)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let memory = Memory {
                id: row.get("id"),
                profile_id: row.get("profile_id"),
                owner_id: row.get("owner_id"),
                scope_id: row.get("scope_id"),
                content: row.get("content"),
                category: row.get("category"),
                tags: row.get("tags"),
                importance: row.get("importance"),
                confidence: row.get("confidence"),
                root_memory_id: row.get("root_memory_id"),
                version_number: row.get("version_number"),
                is_current_version: row.get("is_current_version"),
                supersedes: row.get("supersedes"),
                superseded_by: row.get("superseded_by"),
                is_global: row.get("is_global"),
                hit_count: row.get("hit_count"),
                last_hit_at: row.get("last_hit_at"),
                decay_score: row.get("decay_score"),
                source_event_id: row.get("source_event_id"),
                status: row.get("status"),
                embedding_status: row.get("embedding_status"),
                embedding_provider: row.get("embedding_provider"),
                processing_status: row.get("processing_status"),
                llm_provider: row.get("llm_provider"),
                inference_type: row.get("inference_type"),
                inference_confidence: row.get("inference_confidence"),
                inference_reasoning: row.get("inference_reasoning"),
                promoted_at: row.get("promoted_at"),
                promotion_reason: row.get("promotion_reason"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            };
            let rank: f32 = row.get("rank");
            results.push((memory, rank));
        }

        Ok(results)
    }

    /// Update decay scores for memories belonging to an owner
    pub async fn update_decay_scores(
        &self,
        owner_id: &str,
        decay_half_life_days: f32,
        hit_boost_factor: f32,
        global_boost: f32,
    ) -> AppResult<usize> {
        let result = sqlx::query(
            r#"
            UPDATE memories
            SET decay_score = (1.0 + ln(hit_count + 1) * $2)
                            * exp(-EXTRACT(EPOCH FROM (NOW() - COALESCE(last_hit_at, created_at))) / 86400.0 / $3)
                            * CASE WHEN is_global THEN $4 ELSE 1.0 END,
                updated_at = NOW()
            WHERE owner_id = $1
              AND is_current_version = true
              AND status NOT IN ('superseded', 'archived')
            "#,
        )
        .bind(owner_id)
        .bind(hit_boost_factor)
        .bind(decay_half_life_days)
        .bind(global_boost)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() as usize)
    }

    /// Find memories with low decay scores (eviction candidates)
    pub async fn find_eviction_candidates(
        &self,
        owner_id: &str,
        decay_threshold: f32,
        limit: i64,
    ) -> AppResult<Vec<Memory>> {
        let rows = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                SELECT {}
                FROM memories
                WHERE owner_id = $1
                  AND is_current_version = true
                  AND status IN ('active', 'cooldown', 'candidate')
                  AND decay_score < $2
                ORDER BY decay_score ASC
                LIMIT $3
                "#,
            MEMORY_COLUMNS
        ))
        .bind(owner_id)
        .bind(decay_threshold)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Reinforce a memory (increase confidence and hit count)
    pub async fn reinforce(&self, id: Uuid, confidence_delta: f32) -> AppResult<Memory> {
        let row = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                UPDATE memories
                SET confidence = LEAST(confidence + $2, 1.0),
                    hit_count = hit_count + 1,
                    last_hit_at = NOW(),
                    status = CASE WHEN status = 'cooldown' THEN 'active'::status ELSE status END,
                    updated_at = NOW()
                WHERE id = $1
                RETURNING {}
                "#,
            MEMORY_COLUMNS
        ))
        .bind(id)
        .bind(confidence_delta)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::MemoryNotFound(id))?;

        Ok(row.into())
    }

    /// Find all memories for an owner/scope, grouped by embedding_provider
    ///
    /// Returns memories that are:
    /// - Current version
    /// - Not superseded or archived
    /// - Have completed embedding
    /// - Matching the owner_id
    /// - If scope_id is provided: matching scope_id OR global (if include_global is true)
    /// - If scope_id is NULL: all memories for this owner (no scope filter)
    pub async fn find_by_owner_scope_grouped_by_provider(
        &self,
        owner_id: &str,
        scope_id: Option<&str>,
        include_global: bool,
    ) -> AppResult<std::collections::HashMap<String, Vec<Memory>>> {
        let rows = sqlx::query_as::<_, MemoryRow>(&format!(
            r#"
                SELECT {}
                FROM memories
                WHERE owner_id = $1
                  AND is_current_version = true
                  AND status NOT IN ('superseded', 'archived')
                  AND embedding_status = 'completed'
                  AND embedding_provider IS NOT NULL
                  AND (
                    $2::text IS NULL
                    OR scope_id = $2
                    OR ($3 = true AND is_global = true)
                  )
                ORDER BY embedding_provider, decay_score DESC
                "#,
            MEMORY_COLUMNS
        ))
        .bind(owner_id)
        .bind(scope_id)
        .bind(include_global)
        .fetch_all(&self.pool)
        .await?;

        let mut grouped: std::collections::HashMap<String, Vec<Memory>> =
            std::collections::HashMap::new();

        for row in rows {
            let memory: Memory = row.into();
            if let Some(ref provider) = memory.embedding_provider {
                grouped.entry(provider.clone()).or_default().push(memory);
            }
        }

        Ok(grouped)
    }
}

/// Internal row type for sqlx mapping
#[derive(Debug, FromRow)]
struct MemoryRow {
    id: Uuid,
    profile_id: Uuid,
    owner_id: String,
    scope_id: Option<String>,
    content: String,
    category: Option<String>,
    tags: Option<Vec<String>>,
    importance: f32,
    confidence: f32,
    // Version chain
    root_memory_id: Option<Uuid>,
    version_number: i32,
    is_current_version: bool,
    supersedes: Option<Uuid>,
    superseded_by: Option<Uuid>,
    // Lifecycle
    is_global: bool,
    hit_count: i64,
    last_hit_at: Option<DateTime<Utc>>,
    decay_score: f32,
    // Source
    source_event_id: Option<Uuid>,
    // Status
    status: Status,
    // Processing
    embedding_status: EmbeddingStatus,
    embedding_provider: Option<String>,
    processing_status: ProcessingStatus,
    llm_provider: Option<String>,
    // Inference
    inference_type: Option<InferenceType>,
    inference_confidence: Option<f32>,
    inference_reasoning: Option<String>,
    // Promotion
    promoted_at: Option<DateTime<Utc>>,
    promotion_reason: Option<String>,
    // Timestamps
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<MemoryRow> for Memory {
    fn from(row: MemoryRow) -> Self {
        Memory {
            id: row.id,
            profile_id: row.profile_id,
            owner_id: row.owner_id,
            scope_id: row.scope_id,
            content: row.content,
            category: row.category,
            tags: row.tags,
            importance: row.importance,
            confidence: row.confidence,
            root_memory_id: row.root_memory_id,
            version_number: row.version_number,
            is_current_version: row.is_current_version,
            supersedes: row.supersedes,
            superseded_by: row.superseded_by,
            is_global: row.is_global,
            hit_count: row.hit_count,
            last_hit_at: row.last_hit_at,
            decay_score: row.decay_score,
            source_event_id: row.source_event_id,
            status: row.status,
            embedding_status: row.embedding_status,
            embedding_provider: row.embedding_provider,
            processing_status: row.processing_status,
            llm_provider: row.llm_provider,
            inference_type: row.inference_type,
            inference_confidence: row.inference_confidence,
            inference_reasoning: row.inference_reasoning,
            promoted_at: row.promoted_at,
            promotion_reason: row.promotion_reason,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
