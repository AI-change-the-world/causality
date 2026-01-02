//! RetrievalEngine service
//!
//! Responsible for multi-stage memory retrieval:
//! 1. PostgreSQL structured filtering (scope, scene, layer, event_source_prefix)
//! 2. PostgreSQL full-text search (optional, using tsvector/tsquery)
//! 3. Qdrant vector similarity search
//! 4. Composite scoring (similarity, fulltext, importance, recency, hit_count)
//! 5. Hit count updates for returned memories

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::RetrievalConfig;
use crate::domain::{Layer, Memory, MemoryCategory, ScopeType, Status};
use crate::error::AppResult;
use crate::repository::{AuditOperation, AuditRepository, MemoryRepository};

/// Request for memory retrieval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveRequest {
    /// Query text for semantic search and full-text search
    pub query: String,
    /// Filter by scope type
    pub scope_type: Option<ScopeType>,
    /// Filter by scope ID
    pub scope_id: Option<String>,
    /// Filter by layers
    pub layers: Option<Vec<Layer>>,
    /// Filter by scenes
    pub scenes: Option<Vec<String>>,
    /// Filter by memory categories (LLM-classified)
    pub categories: Option<Vec<MemoryCategory>>,
    /// Filter by tags (extracted keywords)
    pub tags: Option<Vec<String>>,
    /// Filter by event source prefix
    pub event_source_prefix: Option<String>,
    /// Maximum number of results (default: 10)
    pub top_k: Option<usize>,
    /// Minimum score threshold
    pub min_score: Option<f32>,
    /// Whether to use full-text search (default: true)
    #[serde(default = "default_use_fulltext")]
    pub use_fulltext: Option<bool>,
    /// Whether to use vector search (default: true)
    #[serde(default = "default_use_vector")]
    pub use_vector: Option<bool>,
    /// Custom weight for full-text search score (overrides config)
    pub fulltext_weight: Option<f32>,
    /// Whether to return highlighted snippets (default: false)
    #[serde(default)]
    pub highlight: Option<bool>,
}

fn default_use_fulltext() -> Option<bool> {
    Some(true)
}

fn default_use_vector() -> Option<bool> {
    Some(true)
}

/// A memory with retrieval scores
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedMemory {
    /// The memory record
    pub memory: Memory,
    /// Composite score
    pub score: f32,
    /// Vector similarity score
    pub similarity: f32,
    /// Full-text search match score (if full-text search was used)
    pub text_match_score: Option<f32>,
    /// Highlighted snippets from the content (if highlighting was requested)
    pub highlights: Option<Vec<String>>,
}

/// Response from retrieval operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveResponse {
    /// Retrieved memories sorted by score
    pub memories: Vec<RetrievedMemory>,
    /// Total candidates after structured filtering
    pub total_candidates: usize,
}

/// RetrievalEngine service for memory search and retrieval
#[derive(Clone)]
pub struct RetrievalEngine {
    memory_repo: MemoryRepository,
    audit_repo: AuditRepository,
    config: RetrievalConfig,
    audit_enabled: bool,
}

impl RetrievalEngine {
    /// Create a new RetrievalEngine service
    pub fn new(
        memory_repo: MemoryRepository,
        audit_repo: AuditRepository,
        config: RetrievalConfig,
        audit_enabled: bool,
    ) -> Self {
        Self {
            memory_repo,
            audit_repo,
            config,
            audit_enabled,
        }
    }

    /// Retrieve memories based on query and filters
    ///
    /// Process:
    /// 1. Apply structured filters (PostgreSQL)
    /// 2. Perform full-text search (PostgreSQL tsvector/tsquery) - optional
    /// 3. Perform vector similarity search (Qdrant) - uses provided similarities
    /// 4. Compute composite scores (combining all signals)
    /// 5. Sort and return top-K results
    /// 6. Update hit counts for returned memories
    pub async fn retrieve(
        &self,
        request: RetrieveRequest,
        similarities: Option<Vec<(Uuid, f32)>>,
        actor_id: Option<String>,
    ) -> AppResult<RetrieveResponse> {
        debug!(
            query = %request.query,
            scope_type = ?request.scope_type,
            scope_id = ?request.scope_id,
            categories = ?request.categories,
            tags = ?request.tags,
            top_k = ?request.top_k,
            use_fulltext = ?request.use_fulltext,
            use_vector = ?request.use_vector,
            "Retrieving memories"
        );

        let top_k = request
            .top_k
            .unwrap_or(self.config.default_top_k)
            .min(self.config.max_top_k);

        let use_fulltext = request.use_fulltext.unwrap_or(true);
        let use_vector = request.use_vector.unwrap_or(true);
        let highlight = request.highlight.unwrap_or(false);
        let fulltext_weight = request
            .fulltext_weight
            .unwrap_or(self.config.score_weights.fulltext);

        // Step 1: Structured filtering (PostgreSQL)
        let candidates = self
            .memory_repo
            .find_for_retrieval(
                request.scope_type.as_ref(),
                request.scope_id.as_deref(),
                request.layers.as_deref(),
                request.scenes.as_deref(),
                request.event_source_prefix.as_deref(),
                request.categories.as_deref(),
                request.tags.as_deref(),
            )
            .await?;

        let total_candidates = candidates.len();

        debug!(
            total_candidates = total_candidates,
            "Structured filtering complete"
        );

        if candidates.is_empty() {
            return Ok(RetrieveResponse {
                memories: vec![],
                total_candidates: 0,
            });
        }

        // Step 2: Full-text search scores (PostgreSQL)
        let fulltext_scores: HashMap<Uuid, f32> =
            if use_fulltext && !request.query.trim().is_empty() {
                let memory_ids: Vec<Uuid> = candidates.iter().map(|m| m.id).collect();
                self.memory_repo
                    .get_fulltext_scores(&memory_ids, &request.query)
                    .await
                    .unwrap_or_default()
            } else {
                HashMap::new()
            };

        debug!(
            fulltext_matches = fulltext_scores.len(),
            "Full-text search complete"
        );

        // Step 3: Build similarity map from Qdrant results
        let similarity_map: HashMap<Uuid, f32> = if use_vector {
            similarities.unwrap_or_default().into_iter().collect()
        } else {
            HashMap::new()
        };

        // Step 4: Compute composite scores
        let mut scored_memories: Vec<RetrievedMemory> = candidates
            .into_iter()
            .map(|memory| {
                // Get similarity from Qdrant results, default to 0.5 if not found
                let similarity = if use_vector {
                    similarity_map.get(&memory.id).copied().unwrap_or(0.5)
                } else {
                    0.0
                };

                // Get full-text score
                let text_match_score = if use_fulltext {
                    fulltext_scores.get(&memory.id).copied()
                } else {
                    None
                };

                let score = self.compute_composite_score_with_fulltext(
                    &memory,
                    similarity,
                    text_match_score.unwrap_or(0.0),
                    fulltext_weight,
                    use_vector,
                    use_fulltext,
                );

                RetrievedMemory {
                    memory,
                    score,
                    similarity,
                    text_match_score,
                    highlights: None, // Will be populated later if requested
                }
            })
            .collect();

        // Apply minimum score filter
        if let Some(min_score) = request.min_score {
            scored_memories.retain(|m| m.score >= min_score);
        }

        // Step 5: Sort by composite score (descending)
        scored_memories.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Take top-K
        scored_memories.truncate(top_k);

        // Step 6: Get highlights if requested
        if highlight && use_fulltext && !request.query.trim().is_empty() {
            for retrieved in &mut scored_memories {
                if retrieved.text_match_score.is_some() {
                    if let Ok(highlights) = self
                        .memory_repo
                        .get_highlights(retrieved.memory.id, &request.query)
                        .await
                    {
                        retrieved.highlights = highlights;
                    }
                }
            }
        }

        // Step 7: Update hit counts for returned memories
        for retrieved in &scored_memories {
            if let Err(e) = self.record_hit(retrieved.memory.id, actor_id.clone()).await {
                warn!(
                    memory_id = %retrieved.memory.id,
                    error = %e,
                    "Failed to record hit"
                );
            }
        }

        info!(
            returned = scored_memories.len(),
            total_candidates = total_candidates,
            "Retrieval complete"
        );

        Ok(RetrieveResponse {
            memories: scored_memories,
            total_candidates,
        })
    }

    /// Compute composite score for a memory (legacy method without full-text)
    ///
    /// Score = w_sim * similarity + w_imp * importance + w_rec * recency + w_hit * hit_factor
    /// With cooldown penalty applied if status is Cooldown
    pub fn compute_composite_score(&self, memory: &Memory, similarity: f32) -> f32 {
        self.compute_composite_score_with_fulltext(memory, similarity, 0.0, 0.0, true, false)
    }

    /// Compute composite score for a memory with full-text search support
    ///
    /// Score = w_sim * similarity + w_ft * fulltext + w_imp * importance + w_rec * recency + w_hit * hit_factor
    /// With cooldown penalty applied if status is Cooldown
    ///
    /// When only one search mode is enabled, the weights are normalized to sum to 1.0
    pub fn compute_composite_score_with_fulltext(
        &self,
        memory: &Memory,
        similarity: f32,
        fulltext_score: f32,
        fulltext_weight: f32,
        use_vector: bool,
        use_fulltext: bool,
    ) -> f32 {
        let weights = &self.config.score_weights;

        // Normalize hit count (log scale to prevent domination)
        let hit_factor = (1.0 + memory.hit_count as f32).ln() / 10.0;
        let hit_factor = hit_factor.min(1.0);

        // Compute recency score (exponential decay)
        let recency = self.compute_recency_score(memory);

        // Normalize full-text score (ts_rank typically returns values < 1.0)
        // We cap it at 1.0 for consistency
        let normalized_fulltext = fulltext_score.min(1.0);

        // Calculate effective weights based on which search modes are enabled
        let (effective_sim_weight, effective_ft_weight) = match (use_vector, use_fulltext) {
            (true, true) => (weights.similarity, fulltext_weight),
            (true, false) => (weights.similarity + fulltext_weight, 0.0),
            (false, true) => (0.0, weights.similarity + fulltext_weight),
            (false, false) => (0.0, 0.0), // Edge case: no search, just use other factors
        };

        // Base composite score
        let mut score = effective_sim_weight * similarity
            + effective_ft_weight * normalized_fulltext
            + weights.importance * memory.importance
            + weights.recency * recency
            + weights.hit_count * hit_factor;

        // Apply cooldown penalty
        if memory.status == Status::Cooldown {
            score *= self.config.cooldown_penalty;
        }

        score.clamp(0.0, 1.0)
    }

    /// Compute recency score based on last hit or creation time
    fn compute_recency_score(&self, memory: &Memory) -> f32 {
        let reference_time = memory.last_hit_at.unwrap_or(memory.created_at);
        let age_hours = (Utc::now() - reference_time).num_hours() as f32;

        // Exponential decay with half-life of 24 hours
        let half_life = 24.0;
        let decay = (-age_hours / half_life * 0.693).exp(); // ln(2) ≈ 0.693

        decay.clamp(0.0, 1.0)
    }

    /// Record a hit on a memory
    async fn record_hit(&self, memory_id: Uuid, actor_id: Option<String>) -> AppResult<Memory> {
        let updated = self.memory_repo.record_hit(memory_id).await?;

        // Create audit log for hit
        if self.audit_enabled {
            self.audit_repo
                .create(
                    memory_id,
                    AuditOperation::Hit,
                    actor_id,
                    None,
                    Some(serde_json::json!({
                        "hit_count": updated.hit_count,
                        "last_hit_at": updated.last_hit_at
                    })),
                    None,
                )
                .await?;
        }

        Ok(updated)
    }

    /// Check if a memory should be excluded based on status
    pub fn should_exclude(memory: &Memory) -> bool {
        memory.status.is_excluded()
    }

    /// Check if a memory has cooldown penalty
    pub fn has_cooldown_penalty(memory: &Memory) -> bool {
        memory.status.has_retrieval_penalty()
    }

    /// Get the configured cooldown penalty factor
    pub fn cooldown_penalty(&self) -> f32 {
        self.config.cooldown_penalty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScoreWeights;
    use crate::domain::{EmbeddingStatus, Layer, ProcessingStatus, ScopeType, Status};

    #[allow(dead_code)]
    fn test_config() -> RetrievalConfig {
        RetrievalConfig {
            default_top_k: 10,
            max_top_k: 100,
            cooldown_penalty: 0.5,
            score_weights: ScoreWeights {
                similarity: 0.35,
                importance: 0.25,
                recency: 0.15,
                hit_count: 0.1,
                fulltext: 0.15,
            },
        }
    }

    fn test_memory(status: Status) -> Memory {
        Memory {
            id: Uuid::new_v4(),
            layer: Layer::Session,
            scope_type: ScopeType::User,
            scope_id: "user123".to_string(),
            scene: "test.scene".to_string(),
            status,
            content: "Test content".to_string(),
            raw_content: None,
            category: None,
            tags: None,
            importance: 0.5,
            confidence: 1.0,
            hit_count: 0,
            last_hit_at: None,
            ttl_seconds: Some(3600),
            expires_at: None,
            event_source: None,
            event_time: None,
            embedding_status: EmbeddingStatus::Completed,
            embedding_provider: Some("openai".to_string()),
            processing_status: ProcessingStatus::Skipped,
            llm_provider: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            inference_type: None,
            inference_confidence: None,
            inference_reasoning: None,
        }
    }

    #[test]
    fn test_should_exclude() {
        assert!(!RetrievalEngine::should_exclude(&test_memory(
            Status::Active
        )));
        assert!(!RetrievalEngine::should_exclude(&test_memory(
            Status::Cooldown
        )));
        assert!(RetrievalEngine::should_exclude(&test_memory(
            Status::Ignored
        )));
        assert!(RetrievalEngine::should_exclude(&test_memory(
            Status::Archived
        )));
    }

    #[test]
    fn test_has_cooldown_penalty() {
        assert!(!RetrievalEngine::has_cooldown_penalty(&test_memory(
            Status::Active
        )));
        assert!(RetrievalEngine::has_cooldown_penalty(&test_memory(
            Status::Cooldown
        )));
        assert!(!RetrievalEngine::has_cooldown_penalty(&test_memory(
            Status::Ignored
        )));
    }
}
