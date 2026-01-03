//! Global Promoter Service
//!
//! Responsible for promoting memories to global status based on cross-scope reinforcement.
//! A memory becomes global when it has been reinforced across multiple scopes,
//! indicating it represents knowledge that transcends individual contexts.
//!
//! Requirements: 5.1, 5.2, 5.3

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};
use uuid::Uuid;

use crate::domain::Memory;
use crate::error::AppResult;
use crate::repository::{EventRepository, MemoryRepository};

/// Criteria for promoting a memory to global status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionCriteria {
    /// Minimum number of different scopes that reinforced the memory
    pub min_scope_diversity: i32,
    /// Minimum number of reinforcements
    pub min_reinforcements: i32,
    /// Minimum confidence score
    pub min_confidence: f32,
    /// Minimum age in hours before promotion is considered
    pub min_age_hours: i64,
}

impl Default for PromotionCriteria {
    fn default() -> Self {
        Self {
            min_scope_diversity: 2,
            min_reinforcements: 3,
            min_confidence: 0.7,
            min_age_hours: 24,
        }
    }
}

/// Result of a promotion eligibility check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionCheck {
    /// Whether the memory is eligible for promotion
    pub eligible: bool,
    /// Number of different scopes that reinforced the memory
    pub scope_count: i32,
    /// Total number of reinforcements
    pub total_reinforcements: i64,
    /// Current confidence score
    pub confidence: f32,
    /// Age in hours
    pub age_hours: i64,
    /// Reason for eligibility/ineligibility
    pub reason: Option<String>,
}

/// Global Promoter service for managing memory promotion to global status
#[derive(Clone)]
pub struct GlobalPromoter {
    memory_repo: MemoryRepository,
    event_repo: EventRepository,
    criteria: PromotionCriteria,
}

impl GlobalPromoter {
    /// Create a new GlobalPromoter
    pub fn new(
        memory_repo: MemoryRepository,
        event_repo: EventRepository,
        criteria: PromotionCriteria,
    ) -> Self {
        Self {
            memory_repo,
            event_repo,
            criteria,
        }
    }

    /// Create with default criteria
    pub fn with_defaults(memory_repo: MemoryRepository, event_repo: EventRepository) -> Self {
        Self::new(memory_repo, event_repo, PromotionCriteria::default())
    }

    /// Check if a memory is eligible for promotion to global status
    #[instrument(skip(self), fields(memory_id = %memory_id))]
    pub async fn check_eligibility(&self, memory_id: Uuid) -> AppResult<PromotionCheck> {
        // Get the memory
        let memory = self.memory_repo.get_by_id(memory_id).await?;

        // Already global
        if memory.is_global {
            return Ok(PromotionCheck {
                eligible: false,
                scope_count: 0,
                total_reinforcements: 0,
                confidence: memory.confidence,
                age_hours: self.calculate_age_hours(&memory),
                reason: Some("Memory is already global".to_string()),
            });
        }

        // Not current version
        if !memory.is_current_version {
            return Ok(PromotionCheck {
                eligible: false,
                scope_count: 0,
                total_reinforcements: 0,
                confidence: memory.confidence,
                age_hours: self.calculate_age_hours(&memory),
                reason: Some("Memory is not the current version".to_string()),
            });
        }

        // Get scope diversity and reinforcement count
        let scope_count = self.event_repo.count_scope_diversity(memory_id).await? as i32;
        let total_reinforcements = self.event_repo.count_evidence(memory_id).await?;
        let age_hours = self.calculate_age_hours(&memory);

        // Check each criterion
        let mut reasons = Vec::new();

        if scope_count < self.criteria.min_scope_diversity {
            reasons.push(format!(
                "Scope diversity {} < required {}",
                scope_count, self.criteria.min_scope_diversity
            ));
        }

        if total_reinforcements < self.criteria.min_reinforcements as i64 {
            reasons.push(format!(
                "Reinforcements {} < required {}",
                total_reinforcements, self.criteria.min_reinforcements
            ));
        }

        if memory.confidence < self.criteria.min_confidence {
            reasons.push(format!(
                "Confidence {:.2} < required {:.2}",
                memory.confidence, self.criteria.min_confidence
            ));
        }

        if age_hours < self.criteria.min_age_hours {
            reasons.push(format!(
                "Age {} hours < required {} hours",
                age_hours, self.criteria.min_age_hours
            ));
        }

        let eligible = reasons.is_empty();
        let reason = if eligible {
            Some("All criteria met".to_string())
        } else {
            Some(reasons.join("; "))
        };

        debug!(
            memory_id = %memory_id,
            eligible = eligible,
            scope_count = scope_count,
            reinforcements = total_reinforcements,
            confidence = memory.confidence,
            age_hours = age_hours,
            "Checked promotion eligibility"
        );

        Ok(PromotionCheck {
            eligible,
            scope_count,
            total_reinforcements,
            confidence: memory.confidence,
            age_hours,
            reason,
        })
    }

    /// Promote a memory to global status
    ///
    /// This sets is_global = true and scope_id = null
    #[instrument(skip(self), fields(memory_id = %memory_id))]
    pub async fn promote(&self, memory_id: Uuid, reason: &str) -> AppResult<Memory> {
        let memory = self
            .memory_repo
            .promote_to_global(memory_id, reason)
            .await?;

        info!(
            memory_id = %memory_id,
            reason = %reason,
            "Promoted memory to global status"
        );

        Ok(memory)
    }

    /// Check eligibility and promote if eligible
    ///
    /// Returns the promotion check result and optionally the promoted memory
    #[instrument(skip(self), fields(memory_id = %memory_id))]
    pub async fn check_and_promote(
        &self,
        memory_id: Uuid,
    ) -> AppResult<(PromotionCheck, Option<Memory>)> {
        let check = self.check_eligibility(memory_id).await?;

        if check.eligible {
            let reason = format!(
                "Auto-promoted: {} scopes, {} reinforcements, {:.2} confidence",
                check.scope_count, check.total_reinforcements, check.confidence
            );
            let memory = self.promote(memory_id, &reason).await?;
            Ok((check, Some(memory)))
        } else {
            Ok((check, None))
        }
    }

    /// Find and promote all eligible memories for an owner
    #[instrument(skip(self), fields(owner_id = %owner_id))]
    pub async fn promote_eligible(&self, owner_id: &str) -> AppResult<Vec<Memory>> {
        let candidates = self
            .memory_repo
            .find_global_promotion_candidates(
                self.criteria.min_scope_diversity,
                self.criteria.min_reinforcements,
                self.criteria.min_confidence,
                self.criteria.min_age_hours,
            )
            .await?;

        // Filter by owner
        let owner_candidates: Vec<_> = candidates
            .into_iter()
            .filter(|m| m.owner_id == owner_id)
            .collect();

        let mut promoted = Vec::new();

        for memory in owner_candidates {
            let reason = format!(
                "Batch promotion: met all criteria (scope_diversity >= {}, reinforcements >= {}, confidence >= {:.2}, age >= {} hours)",
                self.criteria.min_scope_diversity,
                self.criteria.min_reinforcements,
                self.criteria.min_confidence,
                self.criteria.min_age_hours
            );

            match self.promote(memory.id, &reason).await {
                Ok(m) => promoted.push(m),
                Err(e) => {
                    tracing::warn!(
                        memory_id = %memory.id,
                        error = %e,
                        "Failed to promote memory"
                    );
                }
            }
        }

        info!(
            owner_id = %owner_id,
            promoted_count = promoted.len(),
            "Batch promotion completed"
        );

        Ok(promoted)
    }

    /// Get the current promotion criteria
    pub fn criteria(&self) -> &PromotionCriteria {
        &self.criteria
    }

    /// Update the promotion criteria
    pub fn set_criteria(&mut self, criteria: PromotionCriteria) {
        self.criteria = criteria;
    }

    /// Calculate age in hours for a memory
    fn calculate_age_hours(&self, memory: &Memory) -> i64 {
        let duration = Utc::now().signed_duration_since(memory.created_at);
        duration.num_hours()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_promotion_criteria_default() {
        let criteria = PromotionCriteria::default();
        assert_eq!(criteria.min_scope_diversity, 2);
        assert_eq!(criteria.min_reinforcements, 3);
        assert!((criteria.min_confidence - 0.7).abs() < f32::EPSILON);
        assert_eq!(criteria.min_age_hours, 24);
    }

    #[test]
    fn test_promotion_check_creation() {
        let check = PromotionCheck {
            eligible: true,
            scope_count: 3,
            total_reinforcements: 5,
            confidence: 0.85,
            age_hours: 48,
            reason: Some("All criteria met".to_string()),
        };

        assert!(check.eligible);
        assert_eq!(check.scope_count, 3);
        assert_eq!(check.total_reinforcements, 5);
    }

    #[test]
    fn test_promotion_check_ineligible() {
        let check = PromotionCheck {
            eligible: false,
            scope_count: 1,
            total_reinforcements: 2,
            confidence: 0.5,
            age_hours: 12,
            reason: Some(
                "Scope diversity 1 < required 2; Confidence 0.50 < required 0.70".to_string(),
            ),
        };

        assert!(!check.eligible);
        assert!(check.reason.unwrap().contains("Scope diversity"));
    }
}
