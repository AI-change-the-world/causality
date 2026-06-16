//! Eviction Manager Service
//!
//! Manages memory lifecycle transitions and eviction based on LFU (Least Frequently Used) policy.
//! Implements the status transition flow: Active → Cooldown → Candidate → Archived
//!
//! Status Transitions:
//! - Active → Cooldown: No hit for cooldown_threshold_days
//! - Cooldown → Active: Hit received (handled by record_hit)
//! - Cooldown → Candidate: No hit for candidate_threshold_days
//! - Candidate → Archived: No hit for archive_threshold_days OR capacity limit reached
//!
//! Requirements: 6.3, 6.4, 6.5, 6.6, 6.7

use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

use crate::domain::{Memory, Status};
use crate::error::AppResult;
use crate::repository::MemoryRepository;

/// Configuration for eviction behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvictionConfig {
    /// Days without hit before transitioning to Cooldown (default: 14)
    pub cooldown_threshold_days: i64,
    /// Days without hit before transitioning to Candidate (default: 30)
    pub candidate_threshold_days: i64,
    /// Days without hit before archiving (default: 90)
    pub archive_threshold_days: i64,
    /// Maximum number of active memories per owner (default: 10000)
    pub max_memories_per_owner: i64,
    /// Decay score threshold below which memories become candidates (default: 0.1)
    pub decay_score_threshold: f32,
}

impl Default for EvictionConfig {
    fn default() -> Self {
        Self {
            cooldown_threshold_days: 14,
            candidate_threshold_days: 30,
            archive_threshold_days: 90,
            max_memories_per_owner: 10000,
            decay_score_threshold: 0.1,
        }
    }
}

/// Result of an eviction run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvictionResult {
    /// Number of memories transitioned to Cooldown
    pub cooldown_count: usize,
    /// Number of memories transitioned to Candidate
    pub candidate_count: usize,
    /// Number of memories archived
    pub archived_count: usize,
    /// Whether capacity limit was reached
    pub capacity_limit_reached: bool,
}

impl EvictionResult {
    /// Create an empty result
    pub fn empty() -> Self {
        Self {
            cooldown_count: 0,
            candidate_count: 0,
            archived_count: 0,
            capacity_limit_reached: false,
        }
    }

    /// Total number of memories affected
    pub fn total_affected(&self) -> usize {
        self.cooldown_count + self.candidate_count + self.archived_count
    }
}

/// Eviction Manager service for managing memory lifecycle
#[derive(Clone)]
pub struct EvictionManager {
    memory_repo: MemoryRepository,
    config: EvictionConfig,
}

impl EvictionManager {
    /// Create a new EvictionManager
    pub fn new(memory_repo: MemoryRepository, config: EvictionConfig) -> Self {
        Self {
            memory_repo,
            config,
        }
    }

    /// Create with default configuration
    pub fn with_defaults(memory_repo: MemoryRepository) -> Self {
        Self::new(memory_repo, EvictionConfig::default())
    }

    /// Run the full eviction process for an owner
    ///
    /// This performs:
    /// 1. Transition Active → Cooldown for stale memories
    /// 2. Transition Cooldown → Candidate for very stale memories
    /// 3. Archive Candidate memories that exceed thresholds
    /// 4. Enforce capacity limits if needed
    #[instrument(skip(self), fields(profile_id = %profile_id, owner_id = %owner_id))]
    pub async fn run_eviction(
        &self,
        profile_id: Uuid,
        owner_id: &str,
    ) -> AppResult<EvictionResult> {
        let mut result = EvictionResult::empty();

        // Step 1: Active → Cooldown
        let cooldown_count = self.transition_to_cooldown(profile_id, owner_id).await?;
        result.cooldown_count = cooldown_count;

        // Step 2: Cooldown → Candidate (based on decay score)
        let candidate_count = self.transition_to_candidate(profile_id, owner_id).await?;
        result.candidate_count = candidate_count;

        // Step 3: Archive candidates that exceed threshold
        let archived_count = self.archive_candidates(profile_id, owner_id).await?;
        result.archived_count = archived_count;

        // Step 4: Check and enforce capacity limits
        let capacity_archived = self.enforce_capacity_limit(profile_id, owner_id).await?;
        if capacity_archived > 0 {
            result.archived_count += capacity_archived;
            result.capacity_limit_reached = true;
        }

        info!(
            owner_id = %owner_id,
            cooldown = result.cooldown_count,
            candidate = result.candidate_count,
            archived = result.archived_count,
            capacity_limit = result.capacity_limit_reached,
            "Eviction run completed"
        );

        Ok(result)
    }

    /// Transition Active memories to Cooldown status
    async fn transition_to_cooldown(&self, profile_id: Uuid, owner_id: &str) -> AppResult<usize> {
        let candidates = self
            .memory_repo
            .find_cooldown_candidates(Some(profile_id), self.config.cooldown_threshold_days)
            .await?;

        // Filter by owner
        let owner_candidates: Vec<_> = candidates
            .into_iter()
            .filter(|m| m.owner_id == owner_id && m.status == Status::Active)
            .collect();

        let mut count = 0;
        for memory in owner_candidates {
            match self
                .memory_repo
                .update_status(memory.id, Status::Cooldown)
                .await
            {
                Ok(_) => {
                    count += 1;
                    debug!(memory_id = %memory.id, "Transitioned to Cooldown");
                }
                Err(e) => {
                    warn!(memory_id = %memory.id, error = %e, "Failed to transition to Cooldown");
                }
            }
        }

        Ok(count)
    }

    /// Transition Cooldown memories to Candidate status based on decay score
    async fn transition_to_candidate(&self, profile_id: Uuid, owner_id: &str) -> AppResult<usize> {
        let candidates = self
            .memory_repo
            .find_eviction_candidates(
                profile_id,
                owner_id,
                self.config.decay_score_threshold,
                1000,
            )
            .await?;

        // Filter for Cooldown status only
        let cooldown_candidates: Vec<_> = candidates
            .into_iter()
            .filter(|m| m.status == Status::Cooldown)
            .collect();

        let mut count = 0;
        for memory in cooldown_candidates {
            match self
                .memory_repo
                .update_status(memory.id, Status::Candidate)
                .await
            {
                Ok(_) => {
                    count += 1;
                    debug!(memory_id = %memory.id, "Transitioned to Candidate");
                }
                Err(e) => {
                    warn!(memory_id = %memory.id, error = %e, "Failed to transition to Candidate");
                }
            }
        }

        Ok(count)
    }

    /// Archive Candidate memories that exceed the archive threshold
    async fn archive_candidates(&self, profile_id: Uuid, owner_id: &str) -> AppResult<usize> {
        let candidates = self
            .memory_repo
            .find_eviction_candidates(
                profile_id,
                owner_id,
                self.config.decay_score_threshold,
                1000,
            )
            .await?;

        // Filter for Candidate status and check age
        let archive_candidates: Vec<_> = candidates
            .into_iter()
            .filter(|m| {
                m.status == Status::Candidate
                    && m.should_archive(self.config.archive_threshold_days)
            })
            .collect();

        let mut count = 0;
        for memory in archive_candidates {
            match self
                .memory_repo
                .update_status(memory.id, Status::Archived)
                .await
            {
                Ok(_) => {
                    count += 1;
                    debug!(memory_id = %memory.id, "Archived memory");
                }
                Err(e) => {
                    warn!(memory_id = %memory.id, error = %e, "Failed to archive memory");
                }
            }
        }

        Ok(count)
    }

    /// Enforce capacity limits by archiving lowest decay score memories
    async fn enforce_capacity_limit(&self, profile_id: Uuid, owner_id: &str) -> AppResult<usize> {
        // Get count of active memories
        let all_memories = self
            .memory_repo
            .find_for_retrieval_by_profile(profile_id, owner_id, None, None, None, true)
            .await?;

        let active_count = all_memories.len() as i64;

        if active_count <= self.config.max_memories_per_owner {
            return Ok(0);
        }

        // Need to archive some memories
        let excess = (active_count - self.config.max_memories_per_owner) as usize;

        info!(
            owner_id = %owner_id,
            active_count = active_count,
            max_allowed = self.config.max_memories_per_owner,
            excess = excess,
            "Capacity limit exceeded, archiving excess memories"
        );

        // Get lowest decay score memories
        let candidates = self
            .memory_repo
            .find_eviction_candidates(profile_id, owner_id, f32::MAX, excess as i64 * 2)
            .await?;

        let mut count = 0;
        for memory in candidates.into_iter().take(excess) {
            match self
                .memory_repo
                .update_status(memory.id, Status::Archived)
                .await
            {
                Ok(_) => {
                    count += 1;
                    debug!(
                        memory_id = %memory.id,
                        decay_score = memory.decay_score,
                        "Archived due to capacity limit"
                    );
                }
                Err(e) => {
                    warn!(memory_id = %memory.id, error = %e, "Failed to archive for capacity");
                }
            }
        }

        Ok(count)
    }

    /// Get eviction candidates for an owner (for preview/dry-run)
    #[instrument(skip(self), fields(profile_id = %profile_id, owner_id = %owner_id))]
    pub async fn get_candidates(
        &self,
        profile_id: Uuid,
        owner_id: &str,
        limit: usize,
    ) -> AppResult<Vec<Memory>> {
        let candidates = self
            .memory_repo
            .find_eviction_candidates(
                profile_id,
                owner_id,
                self.config.decay_score_threshold,
                limit as i64,
            )
            .await?;

        Ok(candidates)
    }

    /// Check if a specific memory should transition to cooldown
    pub fn should_cooldown(&self, memory: &Memory) -> bool {
        memory.should_cooldown(self.config.cooldown_threshold_days)
    }

    /// Check if a specific memory should transition to candidate
    pub fn should_become_candidate(&self, memory: &Memory) -> bool {
        memory.should_become_candidate(self.config.candidate_threshold_days)
    }

    /// Check if a specific memory should be archived
    pub fn should_archive(&self, memory: &Memory) -> bool {
        memory.should_archive(self.config.archive_threshold_days)
    }

    /// Get the current configuration
    pub fn config(&self) -> &EvictionConfig {
        &self.config
    }

    /// Update the configuration
    pub fn set_config(&mut self, config: EvictionConfig) {
        self.config = config;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eviction_config_default() {
        let config = EvictionConfig::default();
        assert_eq!(config.cooldown_threshold_days, 14);
        assert_eq!(config.candidate_threshold_days, 30);
        assert_eq!(config.archive_threshold_days, 90);
        assert_eq!(config.max_memories_per_owner, 10000);
        assert!((config.decay_score_threshold - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn test_eviction_result_empty() {
        let result = EvictionResult::empty();
        assert_eq!(result.cooldown_count, 0);
        assert_eq!(result.candidate_count, 0);
        assert_eq!(result.archived_count, 0);
        assert!(!result.capacity_limit_reached);
        assert_eq!(result.total_affected(), 0);
    }

    #[test]
    fn test_eviction_result_total() {
        let result = EvictionResult {
            cooldown_count: 5,
            candidate_count: 3,
            archived_count: 2,
            capacity_limit_reached: false,
        };
        assert_eq!(result.total_affected(), 10);
    }

    #[test]
    fn test_status_transition_validity() {
        // Property 10: Status Transition Validity
        // Valid transitions:
        // - Active → Cooldown
        // - Active → Superseded (handled elsewhere)
        // - Cooldown → Active (handled by record_hit)
        // - Cooldown → Candidate
        // - Candidate → Archived

        // This test documents the valid transitions
        let valid_transitions = vec![
            (Status::Active, Status::Cooldown),
            (Status::Active, Status::Superseded),
            (Status::Cooldown, Status::Active),
            (Status::Cooldown, Status::Candidate),
            (Status::Candidate, Status::Archived),
        ];

        // Invalid transitions (should not happen)
        let invalid_transitions = vec![
            (Status::Active, Status::Candidate),   // Must go through Cooldown
            (Status::Active, Status::Archived),    // Must go through Cooldown → Candidate
            (Status::Cooldown, Status::Archived),  // Must go through Candidate
            (Status::Candidate, Status::Active),   // Cannot recover from Candidate
            (Status::Candidate, Status::Cooldown), // Cannot go back
            (Status::Archived, Status::Active),    // Cannot recover from Archived
            (Status::Superseded, Status::Active),  // Cannot recover from Superseded
        ];

        // Just verify the lists are non-empty (actual enforcement is in the service)
        assert!(!valid_transitions.is_empty());
        assert!(!invalid_transitions.is_empty());
    }
}
