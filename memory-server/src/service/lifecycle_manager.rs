//! LifecycleManager service
//!
//! Responsible for memory lifecycle management:
//! - LFU-based cooldown state transitions
//! - Status change auditing
//!
//! Note: TTL-based expiration has been removed in the new architecture.
//! Eviction is now handled by EvictionManager using LFU algorithm.

use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::LifecycleConfig;
use crate::domain::{Memory, Status};
use crate::error::AppResult;
use crate::repository::{AuditOperation, AuditRepository, MemoryRepository};

/// LifecycleManager service for memory state management
#[derive(Clone)]
pub struct LifecycleManager {
    memory_repo: MemoryRepository,
    audit_repo: AuditRepository,
    config: LifecycleConfig,
    audit_enabled: bool,
}

impl LifecycleManager {
    /// Create a new LifecycleManager service
    pub fn new(
        memory_repo: MemoryRepository,
        audit_repo: AuditRepository,
        config: LifecycleConfig,
        audit_enabled: bool,
    ) -> Self {
        Self {
            memory_repo,
            audit_repo,
            config,
            audit_enabled,
        }
    }

    /// Process cooldown candidates
    ///
    /// Finds all memories that haven't been hit within the cooldown threshold
    /// and transitions them to cooldown status.
    /// Returns the number of memories transitioned to cooldown.
    pub async fn process_cooldown(&self) -> AppResult<usize> {
        debug!("Processing cooldown candidates");

        // Use the eviction config threshold
        let threshold_days = self.config.eviction_config.cooldown_threshold_days;

        let candidates = self
            .memory_repo
            .find_cooldown_candidates(threshold_days)
            .await?;

        let count = candidates.len();

        if count == 0 {
            debug!("No cooldown candidates found");
            return Ok(0);
        }

        info!(count = count, "Found cooldown candidates");

        for memory in candidates {
            if let Err(e) = self
                .transition_status(
                    memory.id,
                    Status::Cooldown,
                    Some("Inactivity threshold exceeded".to_string()),
                    None,
                )
                .await
            {
                warn!(
                    memory_id = %memory.id,
                    error = %e,
                    "Failed to transition memory to cooldown"
                );
            }
        }

        Ok(count)
    }

    /// Transition a memory's status with audit logging
    pub async fn transition_status(
        &self,
        memory_id: Uuid,
        new_status: Status,
        reason: Option<String>,
        actor_id: Option<String>,
    ) -> AppResult<Memory> {
        debug!(
            memory_id = %memory_id,
            new_status = %new_status,
            "Transitioning memory status"
        );

        // Get current memory for audit
        let old_memory = self.memory_repo.get_by_id(memory_id).await?;
        let old_status = old_memory.status;

        // Skip if already in target status
        if old_status == new_status {
            debug!(
                memory_id = %memory_id,
                status = %new_status,
                "Memory already in target status"
            );
            return Ok(old_memory);
        }

        // Perform status update
        let updated = self
            .memory_repo
            .update_status(memory_id, new_status)
            .await?;

        // Create audit log entry
        if self.audit_enabled {
            self.audit_repo
                .create(
                    memory_id,
                    AuditOperation::StatusChange,
                    actor_id,
                    Some(serde_json::json!({
                        "status": old_status.to_string()
                    })),
                    Some(serde_json::json!({
                        "status": new_status.to_string()
                    })),
                    reason,
                )
                .await?;
        }

        info!(
            memory_id = %memory_id,
            old_status = %old_status,
            new_status = %new_status,
            "Memory status transitioned"
        );

        Ok(updated)
    }

    /// Archive a memory
    ///
    /// Used by administrators to explicitly archive a memory.
    pub async fn archive_memory(
        &self,
        memory_id: Uuid,
        reason: String,
        actor_id: Option<String>,
    ) -> AppResult<Memory> {
        self.transition_status(memory_id, Status::Archived, Some(reason), actor_id)
            .await
    }

    /// Reactivate a cooldown memory
    ///
    /// Called when a cooldown memory is hit during retrieval.
    /// This is typically handled by the repository's record_hit method,
    /// but this provides an explicit API for manual reactivation.
    pub async fn reactivate_memory(
        &self,
        memory_id: Uuid,
        actor_id: Option<String>,
    ) -> AppResult<Memory> {
        let memory = self.memory_repo.get_by_id(memory_id).await?;

        if memory.status != Status::Cooldown {
            debug!(
                memory_id = %memory_id,
                status = %memory.status,
                "Memory not in cooldown, skipping reactivation"
            );
            return Ok(memory);
        }

        self.transition_status(
            memory_id,
            Status::Active,
            Some("Manual reactivation".to_string()),
            actor_id,
        )
        .await
    }

    /// Run a full lifecycle check
    ///
    /// Processes cooldown candidates.
    /// Returns the number of memories transitioned to cooldown.
    pub async fn run_lifecycle_check(&self) -> AppResult<usize> {
        info!("Running lifecycle check");

        let cooldown_count = self.process_cooldown().await?;

        info!(cooldown = cooldown_count, "Lifecycle check complete");

        Ok(cooldown_count)
    }

    /// Get the cooldown check interval in seconds
    pub fn check_interval_seconds(&self) -> u64 {
        self.config.cooldown_check_interval_seconds
    }

    /// Query audit logs with filters
    pub async fn query_audit_logs(
        &self,
        params: &crate::repository::AuditQueryParams,
    ) -> AppResult<Vec<crate::repository::AuditLogEntry>> {
        self.audit_repo.query(params).await
    }

    /// Count audit logs matching the query
    pub async fn count_audit_logs(
        &self,
        params: &crate::repository::AuditQueryParams,
    ) -> AppResult<i64> {
        self.audit_repo.count(params).await
    }

    /// Check database health by performing a simple query
    pub async fn check_database_health(&self) -> AppResult<()> {
        // Try to find cooldown candidates - this will verify DB connectivity
        let _ = self.memory_repo.find_cooldown_candidates(0).await?;
        Ok(())
    }

    /// Promote a memory to global status
    ///
    /// Sets is_global = true and scope_id = null for the memory.
    /// Creates an audit log entry for the promotion.
    pub async fn promote_memory(&self, memory_id: Uuid, reason: &str) -> AppResult<Memory> {
        debug!(
            memory_id = %memory_id,
            reason = %reason,
            "Promoting memory to global status"
        );

        // Get current memory for audit
        let old_memory = self.memory_repo.get_by_id(memory_id).await?;

        // Skip if already global
        if old_memory.is_global {
            debug!(
                memory_id = %memory_id,
                "Memory already global, skipping promotion"
            );
            return Ok(old_memory);
        }

        // Perform promotion
        let promoted = self
            .memory_repo
            .promote_to_global(memory_id, reason)
            .await?;

        // Create audit log entry
        if self.audit_enabled {
            self.audit_repo
                .create(
                    memory_id,
                    AuditOperation::StatusChange,
                    None,
                    Some(serde_json::json!({
                        "is_global": false,
                        "scope_id": old_memory.scope_id
                    })),
                    Some(serde_json::json!({
                        "is_global": true,
                        "scope_id": serde_json::Value::Null,
                        "promotion_reason": reason
                    })),
                    Some(format!("Promoted to global: {}", reason)),
                )
                .await?;
        }

        info!(
            memory_id = %memory_id,
            reason = %reason,
            "Memory promoted to global status"
        );

        Ok(promoted)
    }

    /// Get eviction candidates for preview/dry-run
    pub async fn get_eviction_candidates(
        &self,
        owner_id: &str,
        limit: usize,
    ) -> AppResult<Vec<Memory>> {
        // Get memories with low decay scores
        let candidates = self
            .memory_repo
            .find_eviction_candidates(owner_id, 0.5, limit as i64)
            .await?;
        Ok(candidates)
    }

    /// Run the eviction process for an owner
    pub async fn run_eviction(&self, owner_id: &str) -> AppResult<crate::service::EvictionResult> {
        use crate::service::EvictionResult;

        let mut result = EvictionResult::empty();

        // Step 1: Find and transition Active → Cooldown
        let cooldown_threshold_days = self.config.eviction_config.cooldown_threshold_days;
        let cooldown_candidates = self
            .memory_repo
            .find_cooldown_candidates(cooldown_threshold_days)
            .await?;

        for memory in cooldown_candidates
            .into_iter()
            .filter(|m| m.owner_id == owner_id && m.status == Status::Active)
        {
            if let Ok(_) = self
                .transition_status(
                    memory.id,
                    Status::Cooldown,
                    Some("Eviction: inactivity threshold exceeded".to_string()),
                    None,
                )
                .await
            {
                result.cooldown_count += 1;
            }
        }

        // Step 2: Find and transition Cooldown → Candidate (based on decay score)
        let candidates = self
            .memory_repo
            .find_eviction_candidates(owner_id, 0.1, 1000)
            .await?;

        for memory in candidates
            .into_iter()
            .filter(|m| m.status == Status::Cooldown)
        {
            if let Ok(_) = self
                .transition_status(
                    memory.id,
                    Status::Candidate,
                    Some("Eviction: low decay score".to_string()),
                    None,
                )
                .await
            {
                result.candidate_count += 1;
            }
        }

        // Step 3: Archive Candidate memories
        let archive_candidates = self
            .memory_repo
            .find_eviction_candidates(owner_id, 0.05, 1000)
            .await?;

        for memory in archive_candidates
            .into_iter()
            .filter(|m| m.status == Status::Candidate)
        {
            if let Ok(_) = self
                .transition_status(
                    memory.id,
                    Status::Archived,
                    Some("Eviction: archived due to low activity".to_string()),
                    None,
                )
                .await
            {
                result.archived_count += 1;
            }
        }

        info!(
            owner_id = %owner_id,
            cooldown = result.cooldown_count,
            candidate = result.candidate_count,
            archived = result.archived_count,
            "Eviction completed"
        );

        Ok(result)
    }

    /// Update decay scores for an owner
    pub async fn update_decay_scores(
        &self,
        owner_id: &str,
        config: &crate::service::DecayConfig,
    ) -> AppResult<usize> {
        let updated = self
            .memory_repo
            .update_decay_scores(
                owner_id,
                config.decay_half_life_days,
                config.hit_boost_factor,
                config.global_boost,
            )
            .await?;

        info!(
            owner_id = %owner_id,
            updated_count = updated,
            "Decay scores updated"
        );

        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DecayConfig, EvictionConfig};

    fn test_config() -> LifecycleConfig {
        LifecycleConfig {
            cooldown_check_interval_seconds: 3600,
            decay_config: DecayConfig::default(),
            eviction_config: EvictionConfig {
                cooldown_threshold_days: 7,
                candidate_threshold_days: 14,
                archive_threshold_days: 30,
                max_memories_per_scope: 1000,
                max_global_memories: 10000,
                batch_size: 100,
            },
        }
    }

    #[test]
    fn test_check_interval() {
        let config = test_config();
        // We can't create a full LifecycleManager without repos,
        // but we can test the config values
        assert_eq!(config.cooldown_check_interval_seconds, 3600);
        assert_eq!(config.eviction_config.cooldown_threshold_days, 7);
        assert_eq!(config.eviction_config.candidate_threshold_days, 14);
        assert_eq!(config.eviction_config.archive_threshold_days, 30);
    }
}
