//! Memory Reconciler Service
//!
//! Responsible for deciding how to handle extracted memories based on matching results.
//! Implements the three reconciliation outcomes:
//! - CreateNew: No match found, create a new memory
//! - Reinforce: Match found with consistent content, reinforce existing memory
//! - Supersede: Match found with conflicting content, create new version
//!
//! Requirements: 3.3, 3.4

use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};
use uuid::Uuid;

use crate::domain::{
    CreateMemoryFromEventInput, CreateSupersedingMemoryInput, Event, EventMemoryRelation,
    InferenceType, Memory,
};
use crate::error::AppResult;
use crate::repository::{EventRepository, MemoryRepository};
use crate::service::MatchResult;

/// Extracted memory content from LLM processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMemory {
    /// Memory content
    pub content: String,
    /// Hierarchical category (e.g., "work.code.eslint")
    pub category: Option<String>,
    /// Tags/keywords
    pub tags: Option<Vec<String>>,
    /// Importance score (0.0 - 1.0)
    pub importance: f32,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    /// Inference type
    pub inference_type: InferenceType,
    /// Inference confidence
    pub inference_confidence: f32,
    /// Reasoning for the inference
    pub inference_reasoning: String,
}

/// Outcome of the reconciliation process
#[derive(Debug, Clone)]
pub enum ReconcileOutcome {
    /// No match found, create a new memory
    CreateNew {
        /// The newly created memory
        memory: Memory,
        /// The event-memory relation
        relation: EventMemoryRelation,
    },
    /// Match found with consistent content, reinforce existing memory
    Reinforce {
        /// ID of the reinforced memory
        memory_id: Uuid,
        /// Confidence increase applied
        confidence_delta: f32,
        /// The event-memory relation
        relation: EventMemoryRelation,
    },
    /// Match found with conflicting content, create new version
    Supersede {
        /// The newly created memory (new version)
        new_memory: Memory,
        /// ID of the superseded memory
        superseded_id: Uuid,
        /// The event-memory relation
        relation: EventMemoryRelation,
    },
}

/// Result of LLM consistency check
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsistencyResult {
    /// Content is semantically consistent (can reinforce)
    Consistent,
    /// Content conflicts (should supersede)
    Conflicting,
}

/// Trait for checking semantic consistency between memories
/// This is typically implemented using LLM
#[async_trait::async_trait]
pub trait ConsistencyChecker: Send + Sync {
    /// Check if the new content is consistent with the existing memory
    async fn check_consistency(
        &self,
        existing_content: &str,
        new_content: &str,
    ) -> AppResult<ConsistencyResult>;
}

/// Configuration for the reconciler
#[derive(Debug, Clone)]
pub struct ReconcilerConfig {
    /// Confidence delta to apply when reinforcing (default: 0.05)
    pub reinforce_confidence_delta: f32,
    /// Maximum confidence value (default: 1.0)
    pub max_confidence: f32,
}

impl Default for ReconcilerConfig {
    fn default() -> Self {
        Self {
            reinforce_confidence_delta: 0.05,
            max_confidence: 1.0,
        }
    }
}

/// Memory Reconciler service
pub struct MemoryReconciler<C: ConsistencyChecker> {
    memory_repo: MemoryRepository,
    event_repo: EventRepository,
    consistency_checker: C,
    config: ReconcilerConfig,
}

impl<C: ConsistencyChecker> MemoryReconciler<C> {
    /// Create a new MemoryReconciler
    pub fn new(
        memory_repo: MemoryRepository,
        event_repo: EventRepository,
        consistency_checker: C,
        config: ReconcilerConfig,
    ) -> Self {
        Self {
            memory_repo,
            event_repo,
            consistency_checker,
            config,
        }
    }

    /// Create with default configuration
    pub fn with_defaults(
        memory_repo: MemoryRepository,
        event_repo: EventRepository,
        consistency_checker: C,
    ) -> Self {
        Self::new(
            memory_repo,
            event_repo,
            consistency_checker,
            ReconcilerConfig::default(),
        )
    }

    /// Reconcile an extracted memory with existing matches
    ///
    /// Decision logic:
    /// 1. If no matches: CreateNew
    /// 2. If best match is consistent: Reinforce
    /// 3. If best match conflicts: Supersede
    #[instrument(skip(self, event, extracted, matches), fields(event_id = %event.id))]
    pub async fn reconcile(
        &self,
        event: &Event,
        extracted: &ExtractedMemory,
        matches: Vec<MatchResult>,
    ) -> AppResult<ReconcileOutcome> {
        // Case A: No matches - create new memory
        if matches.is_empty() {
            debug!("No matches found, creating new memory");
            return self.create_new(event, extracted).await;
        }

        // Get the best match (highest similarity)
        let best_match = &matches[0];
        debug!(
            memory_id = %best_match.memory.id,
            similarity = best_match.similarity_score,
            "Found best match"
        );

        // Check semantic consistency with LLM
        let consistency = self
            .consistency_checker
            .check_consistency(&best_match.memory.content, &extracted.content)
            .await?;

        match consistency {
            // Case B: Consistent - reinforce existing memory
            ConsistencyResult::Consistent => {
                debug!("Content is consistent, reinforcing memory");
                self.reinforce(event, &best_match.memory).await
            }
            // Case C: Conflicting - create new version (supersede)
            ConsistencyResult::Conflicting => {
                debug!("Content conflicts, creating new version");
                self.supersede(event, extracted, &best_match.memory).await
            }
        }
    }

    /// Reconcile an extracted memory with existing matches using a dynamic LLM provider
    ///
    /// This method allows passing an LLM provider at runtime for consistency checking,
    /// which is useful when the LLM provider is determined by the request context.
    #[instrument(skip(self, event, extracted, matches, llm_provider), fields(event_id = %event.id))]
    pub async fn reconcile_with_llm(
        &self,
        event: &Event,
        extracted: &ExtractedMemory,
        matches: Vec<MatchResult>,
        llm_provider: std::sync::Arc<dyn crate::llm::LlmProvider>,
    ) -> AppResult<ReconcileOutcome> {
        // Case A: No matches - create new memory
        if matches.is_empty() {
            debug!("No matches found, creating new memory");
            return self.create_new(event, extracted).await;
        }

        // Get the best match (highest similarity)
        let best_match = &matches[0];
        debug!(
            memory_id = %best_match.memory.id,
            similarity = best_match.similarity_score,
            "Found best match, checking consistency with LLM"
        );

        // Use LLM to check semantic consistency
        let llm_checker = LlmConsistencyChecker::new(llm_provider);
        let consistency = llm_checker
            .check_consistency(&best_match.memory.content, &extracted.content)
            .await?;

        match consistency {
            // Case B: Consistent - reinforce existing memory
            ConsistencyResult::Consistent => {
                debug!("Content is consistent (LLM), reinforcing memory");
                self.reinforce(event, &best_match.memory).await
            }
            // Case C: Conflicting - create new version (supersede)
            ConsistencyResult::Conflicting => {
                debug!("Content conflicts (LLM), creating new version");
                self.supersede(event, extracted, &best_match.memory).await
            }
        }
    }

    /// Create a new memory (no match found)
    async fn create_new(
        &self,
        event: &Event,
        extracted: &ExtractedMemory,
    ) -> AppResult<ReconcileOutcome> {
        let input = CreateMemoryFromEventInput {
            profile_id: event.profile_id,
            owner_id: event.owner_id.clone(),
            scope_id: event.scope_id.clone(),
            content: extracted.content.clone(),
            category: extracted.category.clone(),
            tags: extracted.tags.clone(),
            importance: extracted.importance,
            confidence: extracted.confidence,
            source_event_id: event.id,
            embedding_provider: None,
            llm_provider: None,
            inference_type: extracted.inference_type,
            inference_confidence: extracted.inference_confidence,
            inference_reasoning: extracted.inference_reasoning.clone(),
        };

        let memory = Memory::new_from_event(input);
        let created = self.memory_repo.create(&memory).await?;

        // Create event-memory relation
        let relation = EventMemoryRelation::created_from(event.id, created.id);
        let relation = self.event_repo.create_relation(&relation).await?;

        info!(
            memory_id = %created.id,
            event_id = %event.id,
            "Created new memory from event"
        );

        Ok(ReconcileOutcome::CreateNew {
            memory: created,
            relation,
        })
    }

    /// Reinforce an existing memory (consistent match)
    async fn reinforce(&self, event: &Event, existing: &Memory) -> AppResult<ReconcileOutcome> {
        // Reinforce the memory (increase confidence, increment hit count)
        let _updated = self
            .memory_repo
            .reinforce(existing.id, self.config.reinforce_confidence_delta)
            .await?;

        // Create event-memory relation
        let relation = EventMemoryRelation::reinforced_by(event.id, existing.id);
        let relation = self.event_repo.create_relation(&relation).await?;

        info!(
            memory_id = %existing.id,
            event_id = %event.id,
            confidence_delta = self.config.reinforce_confidence_delta,
            "Reinforced existing memory"
        );

        Ok(ReconcileOutcome::Reinforce {
            memory_id: existing.id,
            confidence_delta: self.config.reinforce_confidence_delta,
            relation,
        })
    }

    /// Supersede an existing memory (conflicting match)
    async fn supersede(
        &self,
        event: &Event,
        extracted: &ExtractedMemory,
        old_memory: &Memory,
    ) -> AppResult<ReconcileOutcome> {
        // Determine root_memory_id for the version chain
        let root_id = old_memory.root_memory_id.unwrap_or(old_memory.id);
        let new_version = old_memory.version_number + 1;

        // Create the superseding memory input
        let input = CreateSupersedingMemoryInput {
            old_memory_id: old_memory.id,
            root_memory_id: root_id,
            version_number: new_version,
            profile_id: old_memory.profile_id,
            owner_id: old_memory.owner_id.clone(),
            scope_id: old_memory.scope_id.clone(),
            content: extracted.content.clone(),
            category: extracted.category.clone(),
            tags: extracted.tags.clone(),
            importance: extracted.importance,
            confidence: extracted.confidence,
            is_global: old_memory.is_global, // Inherit global status
            source_event_id: event.id,
            embedding_provider: None,
            llm_provider: None,
            inference_type: extracted.inference_type,
            inference_confidence: extracted.inference_confidence,
            inference_reasoning: extracted.inference_reasoning.clone(),
        };

        // Create the new version
        let new_memory = Memory::new_superseding(input);
        let created = self.memory_repo.create(&new_memory).await?;

        // Update the old memory to mark it as superseded
        self.memory_repo
            .update_superseded(old_memory.id, created.id)
            .await?;

        // Create event-memory relation for the new memory
        let relation = EventMemoryRelation::created_from(event.id, created.id);
        let relation = self.event_repo.create_relation(&relation).await?;

        info!(
            new_memory_id = %created.id,
            old_memory_id = %old_memory.id,
            root_memory_id = %root_id,
            version = new_version,
            event_id = %event.id,
            "Created superseding memory version"
        );

        Ok(ReconcileOutcome::Supersede {
            new_memory: created,
            superseded_id: old_memory.id,
            relation,
        })
    }
}

/// Simple consistency checker that always returns Consistent
/// Useful for testing or when LLM is not available
pub struct AlwaysConsistentChecker;

#[async_trait::async_trait]
impl ConsistencyChecker for AlwaysConsistentChecker {
    async fn check_consistency(
        &self,
        _existing_content: &str,
        _new_content: &str,
    ) -> AppResult<ConsistencyResult> {
        Ok(ConsistencyResult::Consistent)
    }
}

/// Simple consistency checker that always returns Conflicting
/// Useful for testing
pub struct AlwaysConflictingChecker;

#[async_trait::async_trait]
impl ConsistencyChecker for AlwaysConflictingChecker {
    async fn check_consistency(
        &self,
        _existing_content: &str,
        _new_content: &str,
    ) -> AppResult<ConsistencyResult> {
        Ok(ConsistencyResult::Conflicting)
    }
}

/// LLM-based consistency checker
/// Uses LLM to determine if two pieces of content are semantically consistent or conflicting
pub struct LlmConsistencyChecker {
    llm_provider: std::sync::Arc<dyn crate::llm::LlmProvider>,
}

impl LlmConsistencyChecker {
    /// Create a new LLM-based consistency checker
    pub fn new(llm_provider: std::sync::Arc<dyn crate::llm::LlmProvider>) -> Self {
        Self { llm_provider }
    }
}

#[async_trait::async_trait]
impl ConsistencyChecker for LlmConsistencyChecker {
    async fn check_consistency(
        &self,
        existing_content: &str,
        new_content: &str,
    ) -> AppResult<ConsistencyResult> {
        use crate::llm::ChatRequest;

        let prompt = format!(
            r#"You are analyzing two pieces of information to determine if they are consistent or conflicting.

EXISTING MEMORY:
{}

NEW INFORMATION:
{}

Analyze whether the new information:
1. CONSISTENT: Supports, reinforces, or is compatible with the existing memory
2. CONFLICTING: Contradicts, opposes, or updates/changes the existing memory

Consider:
- If the new information expresses an opposite opinion or preference, it's CONFLICTING
- If the new information updates or changes a previous stance, it's CONFLICTING
- If the new information adds detail without contradiction, it's CONSISTENT
- If the new information reinforces the same viewpoint, it's CONSISTENT

Respond with ONLY one word: "CONSISTENT" or "CONFLICTING""#,
            existing_content, new_content
        );

        let request = ChatRequest::new(prompt);

        let response = self.llm_provider.chat(request).await.map_err(|e| {
            crate::error::AppError::Internal(format!("LLM consistency check failed: {}", e))
        })?;

        let result = response.content.trim().to_uppercase();

        tracing::debug!(
            existing = %existing_content,
            new = %new_content,
            result = %result,
            "LLM consistency check result"
        );

        if result.contains("CONFLICTING") {
            Ok(ConsistencyResult::Conflicting)
        } else {
            Ok(ConsistencyResult::Consistent)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconciler_config_default() {
        let config = ReconcilerConfig::default();
        assert!((config.reinforce_confidence_delta - 0.05).abs() < f32::EPSILON);
        assert!((config.max_confidence - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_extracted_memory_creation() {
        let extracted = ExtractedMemory {
            content: "User prefers dark mode".to_string(),
            category: Some("preference.ui".to_string()),
            tags: Some(vec!["preference".to_string(), "ui".to_string()]),
            importance: 0.8,
            confidence: 0.95,
            inference_type: InferenceType::Preference,
            inference_confidence: 0.9,
            inference_reasoning: "User explicitly stated preference".to_string(),
        };

        assert_eq!(extracted.content, "User prefers dark mode");
        assert!((extracted.importance - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_consistency_result() {
        assert_eq!(ConsistencyResult::Consistent, ConsistencyResult::Consistent);
        assert_ne!(
            ConsistencyResult::Consistent,
            ConsistencyResult::Conflicting
        );
    }
}
