//! Memory Matcher Service
//!
//! Responsible for finding similar existing memories when processing new events.
//! Uses vector similarity search to match extracted memories against existing ones.
//!
//! Key behaviors:
//! - Only searches `is_current_version = true` memories
//! - Search scope includes `scope_id` match OR `is_global = true`
//! - Uses configurable similarity threshold (default 0.85)

use std::sync::Arc;
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::config::MatchingConfig;
use crate::domain::Memory;
use crate::error::AppResult;
use crate::repository::{MemoryRepository, QdrantRepository, VectorFilter};

/// Result of a memory match operation
#[derive(Debug, Clone)]
pub struct MatchResult {
    /// The matched memory
    pub memory: Memory,
    /// Vector similarity score (0.0 - 1.0)
    pub similarity_score: f32,
}

/// Configuration for memory matching
#[derive(Debug, Clone)]
pub struct MatcherConfig {
    /// Minimum similarity threshold for a match (default: 0.85)
    pub similarity_threshold: f32,
    /// Maximum number of candidates to consider (default: 10)
    pub max_candidates: usize,
}

impl Default for MatcherConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.85,
            max_candidates: 10,
        }
    }
}

impl From<MatchingConfig> for MatcherConfig {
    fn from(config: MatchingConfig) -> Self {
        Self {
            similarity_threshold: config.match_similarity_threshold.clamp(0.0, 1.0),
            max_candidates: config.max_match_candidates.max(1),
        }
    }
}

/// Memory Matcher service for finding similar existing memories
#[derive(Clone)]
pub struct MemoryMatcher {
    memory_repo: MemoryRepository,
    qdrant_repo: Arc<QdrantRepository>,
    config: MatcherConfig,
}

impl MemoryMatcher {
    /// Create a new MemoryMatcher
    pub fn new(
        memory_repo: MemoryRepository,
        qdrant_repo: Arc<QdrantRepository>,
        config: MatcherConfig,
    ) -> Self {
        Self {
            memory_repo,
            qdrant_repo,
            config,
        }
    }

    /// Create a new MemoryMatcher with default configuration
    pub fn with_defaults(
        memory_repo: MemoryRepository,
        qdrant_repo: Arc<QdrantRepository>,
    ) -> Self {
        Self::new(memory_repo, qdrant_repo, MatcherConfig::default())
    }

    /// Find similar existing memories for a given embedding
    ///
    /// Search criteria:
    /// - Same owner_id
    /// - Same scope_id OR is_global = true
    /// - is_current_version = true
    /// - status is active (not superseded or archived)
    /// - Similarity score >= threshold
    ///
    /// # Arguments
    /// * `owner_id` - Owner identifier
    /// * `scope_id` - Optional scope identifier (if None, only searches global memories)
    /// * `embedding` - The embedding vector to match against
    /// * `provider_name` - The embedding provider name (determines which Qdrant collection to search)
    ///
    /// # Returns
    /// Vector of MatchResult sorted by similarity score (highest first)
    #[instrument(skip(self, embedding), fields(profile_id = %profile_id, owner_id = %owner_id, scope_id = ?scope_id))]
    pub async fn find_similar(
        &self,
        profile_id: Uuid,
        owner_id: &str,
        scope_id: Option<&str>,
        embedding: &[f32],
        provider_name: &str,
    ) -> AppResult<Vec<MatchResult>> {
        self.find_similar_with_threshold(
            profile_id,
            owner_id,
            scope_id,
            embedding,
            provider_name,
            self.config.similarity_threshold,
        )
        .await
    }

    /// Find similar memories with a custom similarity threshold
    #[instrument(skip(self, embedding), fields(profile_id = %profile_id, owner_id = %owner_id, scope_id = ?scope_id, threshold = %threshold))]
    pub async fn find_similar_with_threshold(
        &self,
        profile_id: Uuid,
        owner_id: &str,
        scope_id: Option<&str>,
        embedding: &[f32],
        provider_name: &str,
        threshold: f32,
    ) -> AppResult<Vec<MatchResult>> {
        // Build vector filter for Qdrant search
        let filter = VectorFilter {
            profile_id: Some(profile_id),
            owner_id: Some(owner_id.to_string()),
            scope_id: scope_id.map(|s| s.to_string()),
            include_global: Some(true),
            statuses: Some(vec!["active".to_string()]),
            ..Default::default()
        };

        // Search for similar vectors in Qdrant
        let vector_results = self
            .qdrant_repo
            .search(
                provider_name,
                embedding.to_vec(),
                self.config.max_candidates * 2, // Get more candidates for filtering
                Some(filter),
            )
            .await?;

        debug!(
            candidates = vector_results.len(),
            "Found vector search candidates"
        );

        if vector_results.is_empty() {
            return Ok(vec![]);
        }

        // Get memory IDs from vector results
        let memory_ids: Vec<Uuid> = vector_results.iter().map(|r| r.memory_id).collect();

        // Fetch memories from PostgreSQL
        let memories = self.memory_repo.get_by_ids(&memory_ids).await?;

        // Build a map of memory_id -> similarity_score
        let score_map: std::collections::HashMap<Uuid, f32> = vector_results
            .iter()
            .map(|r| (r.memory_id, r.score))
            .collect();

        // Filter and build results
        let mut results: Vec<MatchResult> = memories
            .into_iter()
            .filter(|m| {
                // Must be current version
                if !m.is_current_version {
                    return false;
                }

                // Must be active status (not superseded, archived, etc.)
                if m.status != crate::domain::Status::Active {
                    return false;
                }

                // Must match owner
                if m.owner_id != owner_id {
                    return false;
                }

                // Must stay within the requested business system
                if m.profile_id != profile_id {
                    return false;
                }

                // Must match scope OR be global
                let scope_matches = match (scope_id, &m.scope_id) {
                    (Some(s), Some(ms)) => s == ms,
                    (Some(_), None) => m.is_global, // Global memories match any scope
                    (None, _) => m.is_global,       // No scope specified, only match global
                };

                if !scope_matches && !m.is_global {
                    return false;
                }

                // Check similarity threshold
                let score = score_map.get(&m.id).copied().unwrap_or(0.0);
                score >= threshold
            })
            .map(|memory| {
                let similarity_score = score_map.get(&memory.id).copied().unwrap_or(0.0);
                MatchResult {
                    memory,
                    similarity_score,
                }
            })
            .collect();

        // Sort by similarity score (highest first)
        results.sort_by(|a, b| {
            b.similarity_score
                .partial_cmp(&a.similarity_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Limit to max_candidates
        results.truncate(self.config.max_candidates);

        debug!(
            matches = results.len(),
            threshold = threshold,
            "Filtered memory matches"
        );

        Ok(results)
    }

    /// Find the best matching memory (highest similarity above threshold)
    #[instrument(skip(self, embedding), fields(profile_id = %profile_id, owner_id = %owner_id, scope_id = ?scope_id))]
    pub async fn find_best_match(
        &self,
        profile_id: Uuid,
        owner_id: &str,
        scope_id: Option<&str>,
        embedding: &[f32],
        provider_name: &str,
    ) -> AppResult<Option<MatchResult>> {
        let matches = self
            .find_similar(profile_id, owner_id, scope_id, embedding, provider_name)
            .await?;
        Ok(matches.into_iter().next())
    }

    /// Check if a similar memory exists (without fetching full details)
    #[instrument(skip(self, embedding), fields(profile_id = %profile_id, owner_id = %owner_id, scope_id = ?scope_id))]
    pub async fn has_similar(
        &self,
        profile_id: Uuid,
        owner_id: &str,
        scope_id: Option<&str>,
        embedding: &[f32],
        provider_name: &str,
    ) -> AppResult<bool> {
        let matches = self
            .find_similar(profile_id, owner_id, scope_id, embedding, provider_name)
            .await?;
        Ok(!matches.is_empty())
    }

    /// Get the current configuration
    pub fn config(&self) -> &MatcherConfig {
        &self.config
    }

    /// Update the similarity threshold
    pub fn set_similarity_threshold(&mut self, threshold: f32) {
        self.config.similarity_threshold = threshold.clamp(0.0, 1.0);
    }

    /// Update the max candidates
    pub fn set_max_candidates(&mut self, max: usize) {
        self.config.max_candidates = max.max(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matcher_config_default() {
        let config = MatcherConfig::default();
        assert!((config.similarity_threshold - 0.85).abs() < f32::EPSILON);
        assert_eq!(config.max_candidates, 10);
    }

    #[test]
    fn test_match_result_creation() {
        use crate::domain::{CreateMemoryInput, Memory};

        let input = CreateMemoryInput {
            profile_id: Uuid::new_v4(),
            owner_id: "owner123".to_string(),
            scope_id: Some("scope456".to_string()),
            content: "Test memory".to_string(),
            category: None,
            tags: None,
            importance: None,
            confidence: None,
            is_global: false,
            embedding_provider: None,
            process_with_llm: false,
            llm_provider: None,
        };

        let memory = Memory::new(input);
        let result = MatchResult {
            memory,
            similarity_score: 0.92,
        };

        assert!((result.similarity_score - 0.92).abs() < f32::EPSILON);
    }
}
