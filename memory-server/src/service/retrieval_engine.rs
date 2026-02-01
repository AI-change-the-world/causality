//! RetrievalEngine service
//!
//! Responsible for multi-stage memory retrieval:
//! 1. PostgreSQL structured filtering (owner_id, scope_id, category_prefix, is_global)
//! 2. PostgreSQL full-text search (optional, using tsvector/tsquery)
//! 3. Qdrant vector similarity search
//! 4. Composite scoring (similarity, fulltext, importance, recency, hit_count)
//! 5. Hit count updates for returned memories
//!
//! New architecture:
//! - Default filter: is_current_version = true
//! - Default exclude: status IN ('superseded', 'archived')
//! - Support scope_id + is_global combined query
//! - Support category prefix matching
//! - Support evidence and history loading

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::RetrievalConfig;
use crate::domain::{Event, Memory, Status};
use crate::error::AppResult;
use crate::repository::{AuditOperation, AuditRepository, EventRepository, MemoryRepository};

/// Category query type for hierarchical category matching
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryQuery {
    /// Exact match: category = 'work.code.eslint'
    Exact(String),
    /// Prefix match: category LIKE 'work.code.%'
    Prefix(String),
}

impl CategoryQuery {
    /// Create a prefix query from a string
    pub fn prefix(s: impl Into<String>) -> Self {
        CategoryQuery::Prefix(s.into())
    }

    /// Create an exact query from a string
    pub fn exact(s: impl Into<String>) -> Self {
        CategoryQuery::Exact(s.into())
    }

    /// Get the category string for filtering
    pub fn as_prefix_pattern(&self) -> Option<String> {
        match self {
            CategoryQuery::Exact(cat) => Some(cat.clone()),
            CategoryQuery::Prefix(prefix) => {
                if prefix.ends_with('.') {
                    Some(prefix.clone())
                } else {
                    Some(format!("{}.", prefix))
                }
            }
        }
    }
}

/// Request for memory retrieval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveRequest {
    /// Query text for semantic search and full-text search
    pub query: String,
    /// Owner ID - the unique identifier of the memory owner (required)
    pub owner_id: String,
    /// Filter by scope ID (when provided, also includes global memories)
    pub scope_id: Option<String>,
    /// Filter by category prefix (e.g., "work.code" matches "work.code.eslint")
    pub category_prefix: Option<String>,
    /// Filter by tags (extracted keywords)
    pub tags: Option<Vec<String>>,
    /// Maximum number of results (default: 10)
    pub top_k: Option<usize>,
    /// Minimum score threshold
    pub min_score: Option<f32>,
    /// Minimum confidence threshold
    pub min_confidence: Option<f32>,
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
    /// Whether to include source events (evidence) for each memory
    #[serde(default)]
    pub include_evidence: Option<bool>,
    /// Whether to include version history for each memory
    #[serde(default)]
    pub include_history: Option<bool>,
}

fn default_use_fulltext() -> Option<bool> {
    Some(true)
}

fn default_use_vector() -> Option<bool> {
    Some(true)
}

/// Evidence (source events) for a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEvidence {
    /// Events that created or reinforced this memory
    pub events: Vec<Event>,
    /// Total count of supporting events
    pub total_count: usize,
}

/// Version history for a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryHistory {
    /// All versions of this memory (ordered by version_number)
    pub versions: Vec<Memory>,
    /// The current (latest) version
    pub current_version: Option<Memory>,
}

/// A memory with retrieval scores and optional evidence/history
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
    /// Source events (evidence) for this memory (if include_evidence was true)
    pub evidence: Option<MemoryEvidence>,
    /// Version history for this memory (if include_history was true)
    pub history: Option<MemoryHistory>,
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
///
/// Implements the new architecture requirements:
/// - Default filter: is_current_version = true
/// - Default exclude: status IN ('superseded', 'archived')
/// - Support scope_id + is_global combined query
/// - Support category prefix matching
/// - Support evidence and history loading
#[derive(Clone)]
pub struct RetrievalEngine {
    memory_repo: MemoryRepository,
    event_repo: EventRepository,
    audit_repo: AuditRepository,
    config: RetrievalConfig,
    audit_enabled: bool,
}

impl RetrievalEngine {
    /// Create a new RetrievalEngine service
    pub fn new(
        memory_repo: MemoryRepository,
        event_repo: EventRepository,
        audit_repo: AuditRepository,
        config: RetrievalConfig,
        audit_enabled: bool,
    ) -> Self {
        Self {
            memory_repo,
            event_repo,
            audit_repo,
            config,
            audit_enabled,
        }
    }

    /// Retrieve memories based on query and filters
    ///
    /// Process:
    /// 1. Apply structured filters (PostgreSQL)
    ///    - owner_id (required)
    ///    - scope_id + is_global (when scope_id provided, also includes global memories)
    ///    - is_current_version = true (default)
    ///    - status NOT IN ('superseded', 'archived') (default)
    ///    - category_prefix (optional)
    ///    - tags (optional)
    /// 2. Perform full-text search (PostgreSQL tsvector/tsquery) - optional
    /// 3. Perform vector similarity search (Qdrant) - uses provided similarities
    /// 4. Compute composite scores (combining all signals)
    /// 5. Sort and return top-K results
    /// 6. Update hit counts for returned memories
    /// 7. Load evidence and history if requested
    pub async fn retrieve(
        &self,
        request: RetrieveRequest,
        similarities: Option<Vec<(Uuid, f32)>>,
        actor_id: Option<String>,
    ) -> AppResult<RetrieveResponse> {
        debug!(
            query = %request.query,
            owner_id = %request.owner_id,
            scope_id = ?request.scope_id,
            category_prefix = ?request.category_prefix,
            tags = ?request.tags,
            top_k = ?request.top_k,
            use_fulltext = ?request.use_fulltext,
            use_vector = ?request.use_vector,
            include_evidence = ?request.include_evidence,
            include_history = ?request.include_history,
            "Retrieving memories"
        );

        let top_k = request
            .top_k
            .unwrap_or(self.config.default_top_k)
            .min(self.config.max_top_k);

        let use_fulltext = request.use_fulltext.unwrap_or(true);
        let use_vector = request.use_vector.unwrap_or(true);
        let highlight = request.highlight.unwrap_or(false);
        let include_evidence = request.include_evidence.unwrap_or(false);
        let include_history = request.include_history.unwrap_or(false);
        let fulltext_weight = request
            .fulltext_weight
            .unwrap_or(self.config.score_weights.fulltext);

        // Step 1: Structured filtering (PostgreSQL)
        // - is_current_version = true (default)
        // - status NOT IN ('superseded', 'archived') (default)
        // - scope_id + is_global combined query
        let include_global = request.scope_id.is_some(); // Include global when scope is specified
        let candidates = self
            .memory_repo
            .find_for_retrieval(
                &request.owner_id,
                request.scope_id.as_deref(),
                request.category_prefix.as_deref(),
                request.tags.as_deref(),
                include_global,
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
                    highlights: None,
                    evidence: None,
                    history: None,
                }
            })
            .collect();

        // Apply minimum score filter
        if let Some(min_score) = request.min_score {
            scored_memories.retain(|m| m.score >= min_score);
        }

        // Apply minimum confidence filter
        if let Some(min_confidence) = request.min_confidence {
            scored_memories.retain(|m| m.memory.confidence >= min_confidence);
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

        // Step 7: Load evidence if requested
        if include_evidence {
            for retrieved in &mut scored_memories {
                if let Ok(evidence) = self.load_evidence(retrieved.memory.id).await {
                    retrieved.evidence = Some(evidence);
                }
            }
        }

        // Step 8: Load history if requested
        if include_history {
            for retrieved in &mut scored_memories {
                if let Ok(history) = self.load_history(&retrieved.memory).await {
                    retrieved.history = Some(history);
                }
            }
        }

        // Step 9: Update hit counts for returned memories
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

    /// Load evidence (source events) for a memory
    async fn load_evidence(&self, memory_id: Uuid) -> AppResult<MemoryEvidence> {
        let events = self.event_repo.get_supporting_events(memory_id).await?;
        let total_count = events.len();
        Ok(MemoryEvidence {
            events,
            total_count,
        })
    }

    /// Load version history for a memory
    async fn load_history(&self, memory: &Memory) -> AppResult<MemoryHistory> {
        let root_id = memory.root_memory_id.unwrap_or(memory.id);
        let versions = self.memory_repo.find_by_root_memory_id(root_id).await?;
        let current_version = versions.iter().find(|m| m.is_current_version).cloned();
        Ok(MemoryHistory {
            versions,
            current_version,
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

    /// Get an event by ID
    pub async fn get_event(&self, id: Uuid) -> AppResult<Event> {
        self.event_repo.get_by_id(id).await
    }

    /// Get event-memory relations for an event
    ///
    /// Returns a list of (Memory, RelationType) tuples for all memories
    /// related to the given event.
    pub async fn get_event_memory_relations(
        &self,
        event_id: Uuid,
    ) -> AppResult<Vec<(Memory, crate::domain::EventMemoryRelationType)>> {
        let relations = self.event_repo.get_relations_for_event(event_id).await?;
        let memory_ids: Vec<Uuid> = relations.iter().map(|r| r.memory_id).collect();
        let memories = self.memory_repo.get_by_ids(&memory_ids).await?;

        // Create a map of memory_id -> Memory for quick lookup
        let memory_map: std::collections::HashMap<Uuid, Memory> =
            memories.into_iter().map(|m| (m.id, m)).collect();

        // Build the result with (Memory, RelationType) tuples
        let result: Vec<(Memory, crate::domain::EventMemoryRelationType)> = relations
            .into_iter()
            .filter_map(|r| {
                memory_map
                    .get(&r.memory_id)
                    .map(|m| (m.clone(), r.relation_type))
            })
            .collect();

        Ok(result)
    }

    /// Get memory version history by root_memory_id
    ///
    /// Returns all versions of a memory chain ordered by version_number.
    pub async fn get_memory_history(&self, root_id: Uuid) -> AppResult<MemoryHistory> {
        let versions = self.memory_repo.find_by_root_memory_id(root_id).await?;
        let current_version = versions.iter().find(|m| m.is_current_version).cloned();
        Ok(MemoryHistory {
            versions,
            current_version,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScoreWeights;
    use crate::domain::{EmbeddingStatus, ProcessingStatus, Status};

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
            profile_id: Uuid::new_v4(),
            id: Uuid::new_v4(),
            owner_id: "owner123".to_string(),
            scope_id: Some("scope456".to_string()),
            content: "Test content".to_string(),
            category: Some("work.code".to_string()),
            tags: None,
            importance: 0.5,
            confidence: 1.0,
            // Version chain
            root_memory_id: None,
            version_number: 1,
            is_current_version: true,
            supersedes: None,
            superseded_by: None,
            // Lifecycle
            is_global: false,
            hit_count: 0,
            last_hit_at: None,
            decay_score: 1.0,
            // Source
            source_event_id: None,
            // Status
            status,
            // Processing
            embedding_status: EmbeddingStatus::Completed,
            embedding_provider: Some("openai".to_string()),
            processing_status: ProcessingStatus::Skipped,
            llm_provider: None,
            // Inference
            inference_type: None,
            inference_confidence: None,
            inference_reasoning: None,
            // Promotion
            promoted_at: None,
            promotion_reason: None,
            // Timestamps
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_should_exclude() {
        // Active and Cooldown should NOT be excluded
        assert!(!RetrievalEngine::should_exclude(&test_memory(
            Status::Active
        )));
        assert!(!RetrievalEngine::should_exclude(&test_memory(
            Status::Cooldown
        )));

        // Candidate, Superseded, and Archived SHOULD be excluded
        assert!(RetrievalEngine::should_exclude(&test_memory(
            Status::Candidate
        )));
        assert!(RetrievalEngine::should_exclude(&test_memory(
            Status::Superseded
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
            Status::Candidate
        )));
        assert!(!RetrievalEngine::has_cooldown_penalty(&test_memory(
            Status::Superseded
        )));
        assert!(!RetrievalEngine::has_cooldown_penalty(&test_memory(
            Status::Archived
        )));
    }

    #[test]
    fn test_category_query_prefix() {
        let query = CategoryQuery::prefix("work.code");
        assert_eq!(query.as_prefix_pattern(), Some("work.code.".to_string()));

        let query_with_dot = CategoryQuery::prefix("work.code.");
        assert_eq!(
            query_with_dot.as_prefix_pattern(),
            Some("work.code.".to_string())
        );
    }

    #[test]
    fn test_category_query_exact() {
        let query = CategoryQuery::exact("work.code.eslint");
        assert_eq!(
            query.as_prefix_pattern(),
            Some("work.code.eslint".to_string())
        );
    }
}
