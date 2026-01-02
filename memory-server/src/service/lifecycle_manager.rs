//! LifecycleManager service
//!
//! Responsible for memory lifecycle management:
//! - TTL expiration handling
//! - Cooldown state transitions
//! - Status change auditing

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

    /// Process expired memories
    ///
    /// Finds all memories that have exceeded their TTL and transitions them to archived status.
    /// Returns the number of memories archived.
    pub async fn process_expired(&self) -> AppResult<usize> {
        debug!("Processing expired memories");

        let expired = self.memory_repo.find_expired().await?;
        let count = expired.len();

        if count == 0 {
            debug!("No expired memories found");
            return Ok(0);
        }

        info!(count = count, "Found expired memories to archive");

        for memory in expired {
            if let Err(e) = self
                .transition_status(
                    memory.id,
                    Status::Archived,
                    Some("TTL expired".to_string()),
                    None,
                )
                .await
            {
                warn!(
                    memory_id = %memory.id,
                    error = %e,
                    "Failed to archive expired memory"
                );
            }
        }

        Ok(count)
    }

    /// Process cooldown candidates
    ///
    /// Finds all memories that haven't been hit within their layer's cooldown threshold
    /// and transitions them to cooldown status.
    /// Returns the number of memories transitioned to cooldown.
    pub async fn process_cooldown(&self) -> AppResult<usize> {
        debug!("Processing cooldown candidates");

        let session_threshold = self.config.cooldown_thresholds.session as i64;
        let task_threshold = self.config.cooldown_thresholds.task as i64;
        let longterm_threshold = self.config.cooldown_thresholds.long_term as i64;

        let candidates = self
            .memory_repo
            .find_cooldown_candidates(session_threshold, task_threshold, longterm_threshold)
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

    /// Mark a memory as ignored
    ///
    /// Used by administrators to explicitly ignore a memory.
    pub async fn ignore_memory(
        &self,
        memory_id: Uuid,
        reason: String,
        actor_id: Option<String>,
    ) -> AppResult<Memory> {
        self.transition_status(memory_id, Status::Ignored, Some(reason), actor_id)
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
    /// Processes both expired memories and cooldown candidates.
    /// Returns (expired_count, cooldown_count).
    pub async fn run_lifecycle_check(&self) -> AppResult<(usize, usize)> {
        info!("Running lifecycle check");

        let expired_count = self.process_expired().await?;
        let cooldown_count = self.process_cooldown().await?;

        info!(
            expired = expired_count,
            cooldown = cooldown_count,
            "Lifecycle check complete"
        );

        Ok((expired_count, cooldown_count))
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
        // Try to get a non-existent memory - this will verify DB connectivity
        // without returning an error for "not found"
        let _ = self.memory_repo.find_expired().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CooldownThresholds, DefaultTtl};

    fn test_config() -> LifecycleConfig {
        LifecycleConfig {
            cooldown_check_interval_seconds: 3600,
            cooldown_thresholds: CooldownThresholds {
                session: 86400,     // 1 day
                task: 604800,       // 7 days
                long_term: 2592000, // 30 days
            },
            default_ttl: DefaultTtl {
                session: Some(3600),
                task: Some(604800),
                long_term: None,
            },
        }
    }

    #[test]
    fn test_check_interval() {
        let config = test_config();
        // We can't create a full LifecycleManager without repos,
        // but we can test the config values
        assert_eq!(config.cooldown_check_interval_seconds, 3600);
        assert_eq!(config.cooldown_thresholds.session, 86400);
        assert_eq!(config.cooldown_thresholds.task, 604800);
        assert_eq!(config.cooldown_thresholds.long_term, 2592000);
    }
}
