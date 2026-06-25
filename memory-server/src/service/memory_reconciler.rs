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

use super::memory_processor::ExtractedMemory;
use crate::domain::{
    CreateMemoryFromEventInput, CreateSupersedingMemoryInput, Event, EventMemoryRelation, Memory,
};
use crate::error::AppResult;
use crate::repository::MemoryRepository;
use crate::service::MatchResult;

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

/// Consistency decision from semantic comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsistencyDecision {
    /// Content is semantically consistent (can reinforce)
    Consistent,
    /// Content conflicts (should supersede)
    Conflicting,
}

/// Structured result of a consistency check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsistencyResult {
    /// Whether the new content is consistent or conflicting.
    pub decision: ConsistencyDecision,
    /// Short explanation of the semantic relationship.
    pub reason: String,
    /// Confidence of this consistency decision (0.0 - 1.0).
    pub confidence: f32,
}

impl ConsistencyResult {
    pub fn consistent(reason: impl Into<String>, confidence: f32) -> Self {
        Self {
            decision: ConsistencyDecision::Consistent,
            reason: reason.into(),
            confidence: confidence.clamp(0.0, 1.0),
        }
    }

    pub fn conflicting(reason: impl Into<String>, confidence: f32) -> Self {
        Self {
            decision: ConsistencyDecision::Conflicting,
            reason: reason.into(),
            confidence: confidence.clamp(0.0, 1.0),
        }
    }
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
    consistency_checker: C,
    config: ReconcilerConfig,
}

impl<C: ConsistencyChecker> MemoryReconciler<C> {
    /// Create a new MemoryReconciler
    pub fn new(
        memory_repo: MemoryRepository,
        consistency_checker: C,
        config: ReconcilerConfig,
    ) -> Self {
        Self {
            memory_repo,
            consistency_checker,
            config,
        }
    }

    /// Create with default configuration
    pub fn with_defaults(memory_repo: MemoryRepository, consistency_checker: C) -> Self {
        Self::new(
            memory_repo,
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

        match consistency.decision {
            // Case B: Consistent - reinforce existing memory
            ConsistencyDecision::Consistent => {
                debug!("Content is consistent, reinforcing memory");
                self.reinforce(event, &best_match.memory).await
            }
            // Case C: Conflicting - create new version (supersede)
            ConsistencyDecision::Conflicting => {
                debug!("Content conflicts, creating new version");
                self.supersede(event, extracted, &best_match.memory, &consistency)
                    .await
            }
        }
    }

    /// Reconcile an extracted memory with existing matches using the configured LLM provider.
    ///
    /// This keeps the consistency check explicit while avoiding hidden provider state.
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

        match consistency.decision {
            // Case B: Consistent - reinforce existing memory
            ConsistencyDecision::Consistent => {
                debug!("Content is consistent (LLM), reinforcing memory");
                self.reinforce(event, &best_match.memory).await
            }
            // Case C: Conflicting - create new version (supersede)
            ConsistencyDecision::Conflicting => {
                debug!("Content conflicts (LLM), creating new version");
                self.supersede(event, extracted, &best_match.memory, &consistency)
                    .await
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
            metadata: extracted.metadata.clone(),
            schema_version: extracted.schema_version,
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
        let relation = EventMemoryRelation::created_from(event.id, memory.id);
        let (created, relation) = self
            .memory_repo
            .create_with_relation(&memory, &relation)
            .await?;

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
        let relation = EventMemoryRelation::reinforced_by(event.id, existing.id);
        let (_updated, relation) = self
            .memory_repo
            .reinforce_with_relation(
                existing.id,
                self.config.reinforce_confidence_delta,
                &relation,
            )
            .await?;

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
        consistency: &ConsistencyResult,
    ) -> AppResult<ReconcileOutcome> {
        // Create the superseding memory input
        let input = CreateSupersedingMemoryInput {
            old_memory_id: old_memory.id,
            root_memory_id: old_memory.root_memory_id.unwrap_or(old_memory.id),
            version_number: old_memory.version_number + 1,
            profile_id: old_memory.profile_id,
            owner_id: old_memory.owner_id.clone(),
            scope_id: old_memory.scope_id.clone(),
            content: extracted.content.clone(),
            metadata: extracted.metadata.clone(),
            schema_version: extracted.schema_version,
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

        let new_memory = Memory::new_superseding(input);
        let relation = EventMemoryRelation::created_from(event.id, new_memory.id);
        let (created, superseded, relation) = self
            .memory_repo
            .supersede_with_relation(
                old_memory.id,
                &new_memory,
                &consistency.reason,
                consistency.confidence,
                &relation,
            )
            .await?;

        info!(
            new_memory_id = %created.id,
            old_memory_id = %superseded.id,
            root_memory_id = ?created.root_memory_id,
            version = created.version_number,
            event_id = %event.id,
            "Created superseding memory version"
        );

        Ok(ReconcileOutcome::Supersede {
            new_memory: created,
            superseded_id: superseded.id,
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
        Ok(ConsistencyResult::consistent(
            "Default checker treats all matches as consistent",
            1.0,
        ))
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
        Ok(ConsistencyResult::conflicting(
            "Default checker treats all matches as conflicting",
            1.0,
        ))
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

Respond with ONLY valid JSON in this shape:
{{
  "result": "consistent" | "conflicting",
  "reason": "short explanation, max 30 words",
  "confidence": 0.0-1.0
}}"#,
            existing_content, new_content
        );

        let request = ChatRequest::new(prompt).with_json_response();

        let response = self.llm_provider.chat(request).await.map_err(|e| {
            crate::error::AppError::Internal(format!("LLM consistency check failed: {}", e))
        })?;

        let result = Self::parse_response(&response.content);

        tracing::debug!(
            existing = %existing_content,
            new = %new_content,
            decision = ?result.decision,
            confidence = result.confidence,
            reason = %result.reason,
            "LLM consistency check result"
        );

        Ok(result)
    }
}

impl LlmConsistencyChecker {
    fn parse_response(content: &str) -> ConsistencyResult {
        #[derive(Deserialize)]
        struct LlmConsistencyResponse {
            result: Option<String>,
            decision: Option<String>,
            reason: Option<String>,
            confidence: Option<f32>,
        }

        let trimmed = content.trim();
        if let Ok(parsed) = serde_json::from_str::<LlmConsistencyResponse>(trimmed) {
            let reason = parsed
                .reason
                .filter(|reason| !reason.trim().is_empty())
                .unwrap_or_else(|| "LLM did not provide a reason".to_string());
            let confidence = parsed.confidence.unwrap_or(0.5);
            let result = parsed
                .result
                .or(parsed.decision)
                .unwrap_or_else(|| "consistent".to_string());

            if result.eq_ignore_ascii_case("conflicting") {
                return ConsistencyResult::conflicting(reason, confidence);
            }

            return ConsistencyResult::consistent(reason, confidence);
        }

        let upper = trimmed.to_uppercase();
        if upper.contains("CONFLICTING") {
            ConsistencyResult::conflicting(
                "LLM returned legacy CONFLICTING response without structured reason",
                0.5,
            )
        } else {
            ConsistencyResult::consistent(
                "LLM returned legacy CONSISTENT response without structured reason",
                0.5,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::InferenceType;

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
            metadata: serde_json::json!({
                "memory_type": "ui_preference",
                "theme": "dark"
            }),
            schema_version: 1,
            importance: 0.8,
            confidence: 0.95,
            inference_type: InferenceType::Preference,
            inference_confidence: 0.9,
            inference_reasoning: "User explicitly stated preference".to_string(),
        };

        assert_eq!(extracted.content, "User prefers dark mode");
        assert!((extracted.importance - 0.8).abs() < f32::EPSILON);
        assert_eq!(extracted.metadata["memory_type"], "ui_preference");
    }

    #[test]
    fn test_consistency_result() {
        assert_eq!(
            ConsistencyDecision::Consistent,
            ConsistencyDecision::Consistent
        );
        assert_ne!(
            ConsistencyDecision::Consistent,
            ConsistencyDecision::Conflicting
        );
    }

    #[test]
    fn test_parse_structured_consistency_response() {
        let result = LlmConsistencyChecker::parse_response(
            r#"{"result":"conflicting","reason":"New preference reverses the old one","confidence":0.82}"#,
        );

        assert_eq!(result.decision, ConsistencyDecision::Conflicting);
        assert_eq!(result.reason, "New preference reverses the old one");
        assert!((result.confidence - 0.82).abs() < f32::EPSILON);
    }

    #[test]
    fn test_parse_legacy_consistency_response() {
        let result = LlmConsistencyChecker::parse_response("CONFLICTING");

        assert_eq!(result.decision, ConsistencyDecision::Conflicting);
        assert!((result.confidence - 0.5).abs() < f32::EPSILON);
        assert!(result.reason.contains("legacy"));
    }
}
