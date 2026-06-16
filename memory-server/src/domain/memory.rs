//! Memory entity and validation logic
//!
//! The Memory struct represents a structured context/conclusion/rule stored in the system.
//! In the new architecture:
//! - Memory content is immutable after creation
//! - Conflicts create new versions (version chain)
//! - LFU eviction replaces TTL-based expiration

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{EmbeddingStatus, InferenceType, ProcessingStatus, Status};
use crate::error::AppError;

/// Memory entity representing a structured context/conclusion/rule
///
/// Content is immutable after creation. When content conflicts occur,
/// a new version is created with supersedes/superseded_by links.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Unique identifier
    pub id: Uuid,
    /// System profile ID - which business system this memory belongs to
    pub profile_id: Uuid,
    /// Owner ID - user-defined, not validated semantically
    pub owner_id: String,
    /// Scope identifier - user-defined, null = global memory
    pub scope_id: Option<String>,

    // === Content fields (immutable after creation) ===
    /// Memory content
    pub content: String,
    /// Hierarchical category (e.g., "work.code.eslint")
    pub category: Option<String>,
    /// Tags/keywords extracted from content
    pub tags: Option<Vec<String>>,
    /// Importance score (0.0 - 1.0)
    pub importance: f32,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,

    // === Version chain (materialized path for O(1) queries) ===
    /// Root of the version chain (null or self.id for first version)
    pub root_memory_id: Option<Uuid>,
    /// Version number in the chain (starts at 1)
    pub version_number: i32,
    /// Whether this is the current (latest) version
    pub is_current_version: bool,
    /// ID of the memory this one supersedes (previous version)
    pub supersedes: Option<Uuid>,
    /// ID of the memory that superseded this one (next version)
    pub superseded_by: Option<Uuid>,

    // === Lifecycle management (LFU eviction) ===
    /// Whether this is a global (cross-scope) memory
    pub is_global: bool,
    /// Number of times this memory was retrieved
    pub hit_count: i64,
    /// Last time this memory was retrieved
    pub last_hit_at: Option<DateTime<Utc>>,
    /// Number of times this memory was reinforced
    pub reinforcement_count: i64,
    /// Last time this memory was reinforced
    pub last_reinforced_at: Option<DateTime<Utc>>,
    /// Decay score for LFU eviction (higher = more valuable)
    pub decay_score: f32,

    // === Source tracking ===
    /// Source event that created this memory
    pub source_event_id: Option<Uuid>,

    // === Status ===
    /// Memory lifecycle status
    pub status: Status,

    // === Processing status ===
    /// Embedding generation status
    pub embedding_status: EmbeddingStatus,
    /// Embedding provider used
    pub embedding_provider: Option<String>,
    /// LLM processing status
    pub processing_status: ProcessingStatus,
    /// LLM provider used for processing
    pub llm_provider: Option<String>,

    // === Inference fields (for memories extracted from events) ===
    /// Inference type (fact, preference, pattern, rule)
    pub inference_type: Option<InferenceType>,
    /// Inference confidence score (0.0 - 1.0)
    pub inference_confidence: Option<f32>,
    /// Reasoning for the inference
    pub inference_reasoning: Option<String>,
    /// Reason why this memory superseded another one, if any
    pub conflict_reason: Option<String>,
    /// Confidence of the consistency decision that led to the current state
    pub consistency_confidence: Option<f32>,

    // === Promotion tracking ===
    /// When the memory was promoted to global
    pub promoted_at: Option<DateTime<Utc>>,
    /// Reason for promotion
    pub promotion_reason: Option<String>,

    // === Timestamps ===
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp (for mutable metadata only)
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a new memory directly (not from event)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMemoryInput {
    /// System profile ID - which business system this memory belongs to
    pub profile_id: Uuid,
    /// Owner ID - user-defined, not validated semantically
    pub owner_id: String,
    /// Scope identifier - user-defined, null = global memory
    pub scope_id: Option<String>,
    /// Memory content
    pub content: String,
    /// Hierarchical category (e.g., "work.code.eslint")
    pub category: Option<String>,
    /// Tags/keywords
    pub tags: Option<Vec<String>>,
    /// Importance score (0.0 - 1.0), defaults to 0.5
    pub importance: Option<f32>,
    /// Confidence score (0.0 - 1.0), defaults to 1.0
    pub confidence: Option<f32>,
    /// Whether this is a global memory
    #[serde(default)]
    pub is_global: bool,
    /// Embedding provider to use (uses default if not specified)
    pub embedding_provider: Option<String>,
    /// Whether to process content with LLM
    #[serde(default)]
    pub process_with_llm: bool,
    /// LLM provider to use for processing
    pub llm_provider: Option<String>,
}

/// Input for creating a memory from an event extraction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMemoryFromEventInput {
    /// System profile ID
    pub profile_id: Uuid,
    /// Owner ID
    pub owner_id: String,
    /// Scope identifier (from the source event)
    pub scope_id: Option<String>,
    /// Memory content (extracted by LLM)
    pub content: String,
    /// Hierarchical category
    pub category: Option<String>,
    /// Tags/keywords extracted from content
    pub tags: Option<Vec<String>>,
    /// Importance score (0.0 - 1.0)
    pub importance: f32,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    /// Source event ID
    pub source_event_id: Uuid,
    /// Embedding provider to use
    pub embedding_provider: Option<String>,
    /// LLM provider used for extraction
    pub llm_provider: Option<String>,
    /// Inference type (fact, preference, pattern, rule)
    pub inference_type: InferenceType,
    /// Inference confidence score (0.0 - 1.0)
    pub inference_confidence: f32,
    /// Reasoning for the inference
    pub inference_reasoning: String,
}

/// Input for creating a superseding memory (new version)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSupersedingMemoryInput {
    /// The memory being superseded
    pub old_memory_id: Uuid,
    /// Root memory ID of the version chain
    pub root_memory_id: Uuid,
    /// New version number
    pub version_number: i32,
    /// System profile ID (inherited from old memory)
    pub profile_id: Uuid,
    /// Owner ID (inherited from old memory)
    pub owner_id: String,
    /// Scope ID (inherited from old memory)
    pub scope_id: Option<String>,
    /// New content
    pub content: String,
    /// Category
    pub category: Option<String>,
    /// Tags
    pub tags: Option<Vec<String>>,
    /// Importance
    pub importance: f32,
    /// Confidence
    pub confidence: f32,
    /// Whether this is a global memory (inherited)
    pub is_global: bool,
    /// Source event ID
    pub source_event_id: Uuid,
    /// Embedding provider
    pub embedding_provider: Option<String>,
    /// LLM provider
    pub llm_provider: Option<String>,
    /// Inference type
    pub inference_type: InferenceType,
    /// Inference confidence
    pub inference_confidence: f32,
    /// Inference reasoning
    pub inference_reasoning: String,
}

/// Validation result for memory creation
#[derive(Debug)]
pub struct CreateMemoryValidation;

impl CreateMemoryValidation {
    /// Validate a memory creation request
    ///
    /// Returns Ok(()) if valid, or an AppError if validation fails.
    ///
    /// Validation rules:
    /// - owner_id cannot be empty
    /// - content cannot be empty
    /// - importance must be between 0.0 and 1.0
    /// - confidence must be between 0.0 and 1.0
    pub fn validate(input: &CreateMemoryInput) -> Result<(), AppError> {
        // Validate owner_id is not empty
        if input.owner_id.trim().is_empty() {
            return Err(AppError::Validation("owner_id cannot be empty".to_string()));
        }

        // Validate content is not empty
        if input.content.trim().is_empty() {
            return Err(AppError::Validation("content cannot be empty".to_string()));
        }

        // Validate importance range
        if let Some(importance) = input.importance {
            if !(0.0..=1.0).contains(&importance) {
                return Err(AppError::Validation(
                    "importance must be between 0.0 and 1.0".to_string(),
                ));
            }
        }

        // Validate confidence range
        if let Some(confidence) = input.confidence {
            if !(0.0..=1.0).contains(&confidence) {
                return Err(AppError::Validation(
                    "confidence must be between 0.0 and 1.0".to_string(),
                ));
            }
        }

        Ok(())
    }
}

impl Memory {
    /// Create a new Memory from validated input (first version)
    ///
    /// This should only be called after validation passes.
    pub fn new(input: CreateMemoryInput) -> Self {
        let now = Utc::now();
        let id = Uuid::new_v4();

        // Determine initial processing status based on process_with_llm flag
        let processing_status = if input.process_with_llm {
            ProcessingStatus::Pending
        } else {
            ProcessingStatus::Skipped
        };

        Memory {
            id,
            profile_id: input.profile_id,
            owner_id: input.owner_id,
            scope_id: input.scope_id,
            content: input.content,
            category: input.category,
            tags: input.tags,
            importance: input.importance.unwrap_or(0.5),
            confidence: input.confidence.unwrap_or(1.0),
            // Version chain - first version
            root_memory_id: Some(id), // Points to self for first version
            version_number: 1,
            is_current_version: true,
            supersedes: None,
            superseded_by: None,
            // Lifecycle
            is_global: input.is_global,
            hit_count: 0,
            last_hit_at: None,
            reinforcement_count: 0,
            last_reinforced_at: None,
            decay_score: 1.0,
            // Source
            source_event_id: None,
            // Status
            status: Status::Active,
            // Processing
            embedding_status: EmbeddingStatus::Pending,
            embedding_provider: input.embedding_provider,
            processing_status,
            llm_provider: input.llm_provider,
            // Inference (not applicable for direct creation)
            inference_type: None,
            inference_confidence: None,
            inference_reasoning: None,
            conflict_reason: None,
            consistency_confidence: None,
            // Promotion
            promoted_at: None,
            promotion_reason: None,
            // Timestamps
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a new Memory from an event extraction (first version)
    ///
    /// This creates a memory with all inference-related fields populated,
    /// indicating that this memory was extracted from an event by LLM processing.
    pub fn new_from_event(input: CreateMemoryFromEventInput) -> Self {
        let now = Utc::now();
        let id = Uuid::new_v4();

        Memory {
            id,
            profile_id: input.profile_id,
            owner_id: input.owner_id,
            scope_id: input.scope_id,
            content: input.content,
            category: input.category,
            tags: input.tags,
            importance: input.importance,
            confidence: input.confidence,
            // Version chain - first version
            root_memory_id: Some(id),
            version_number: 1,
            is_current_version: true,
            supersedes: None,
            superseded_by: None,
            // Lifecycle
            is_global: false, // Can be promoted later
            hit_count: 0,
            last_hit_at: None,
            reinforcement_count: 0,
            last_reinforced_at: None,
            decay_score: 1.0,
            // Source
            source_event_id: Some(input.source_event_id),
            // Status
            status: Status::Active,
            // Processing - already processed by LLM
            embedding_status: EmbeddingStatus::Pending,
            embedding_provider: input.embedding_provider,
            processing_status: ProcessingStatus::Completed,
            llm_provider: input.llm_provider,
            // Inference
            inference_type: Some(input.inference_type),
            inference_confidence: Some(input.inference_confidence),
            inference_reasoning: Some(input.inference_reasoning),
            conflict_reason: None,
            consistency_confidence: None,
            // Promotion
            promoted_at: None,
            promotion_reason: None,
            // Timestamps
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a new superseding Memory (new version in chain)
    pub fn new_superseding(input: CreateSupersedingMemoryInput) -> Self {
        let now = Utc::now();
        let id = Uuid::new_v4();

        Memory {
            id,
            profile_id: input.profile_id,
            owner_id: input.owner_id,
            scope_id: input.scope_id,
            content: input.content,
            category: input.category,
            tags: input.tags,
            importance: input.importance,
            confidence: input.confidence,
            // Version chain - new version
            root_memory_id: Some(input.root_memory_id),
            version_number: input.version_number,
            is_current_version: true,
            supersedes: Some(input.old_memory_id),
            superseded_by: None,
            // Lifecycle - inherit global status
            is_global: input.is_global,
            hit_count: 0,
            last_hit_at: None,
            reinforcement_count: 0,
            last_reinforced_at: None,
            decay_score: 1.0,
            // Source
            source_event_id: Some(input.source_event_id),
            // Status
            status: Status::Active,
            // Processing
            embedding_status: EmbeddingStatus::Pending,
            embedding_provider: input.embedding_provider,
            processing_status: ProcessingStatus::Completed,
            llm_provider: input.llm_provider,
            // Inference
            inference_type: Some(input.inference_type),
            inference_confidence: Some(input.inference_confidence),
            inference_reasoning: Some(input.inference_reasoning),
            conflict_reason: None,
            consistency_confidence: None,
            // Promotion
            promoted_at: None,
            promotion_reason: None,
            // Timestamps
            created_at: now,
            updated_at: now,
        }
    }

    /// Mark this memory as superseded by a new version
    pub fn mark_superseded(&mut self, superseded_by_id: Uuid) {
        self.superseded_by = Some(superseded_by_id);
        self.is_current_version = false;
        self.status = Status::Superseded;
        self.updated_at = Utc::now();
    }

    /// Record a hit on this memory (for LFU tracking)
    pub fn record_hit(&mut self) {
        self.hit_count += 1;
        self.last_hit_at = Some(Utc::now());
        self.updated_at = Utc::now();

        // If in cooldown, transition back to active
        if self.status == Status::Cooldown {
            self.status = Status::Active;
        }
    }

    /// Reinforce this memory (increase confidence)
    pub fn reinforce(&mut self, confidence_delta: f32) {
        // Weighted average to increase confidence
        self.confidence = (self.confidence + confidence_delta).min(1.0);
        self.reinforcement_count += 1;
        self.last_reinforced_at = Some(Utc::now());
        self.record_hit();
    }

    /// Record conflict metadata for a superseding memory.
    pub fn record_conflict(&mut self, reason: String, confidence: f32) {
        self.conflict_reason = Some(reason);
        self.consistency_confidence = Some(confidence);
        self.updated_at = Utc::now();
    }

    /// Promote this memory to global
    pub fn promote_to_global(&mut self, reason: &str) {
        self.is_global = true;
        self.scope_id = None;
        self.promoted_at = Some(Utc::now());
        self.promotion_reason = Some(reason.to_string());
        self.updated_at = Utc::now();
    }

    /// Transition to cooldown status
    pub fn transition_to_cooldown(&mut self) {
        if self.status == Status::Active {
            self.status = Status::Cooldown;
            self.updated_at = Utc::now();
        }
    }

    /// Transition to candidate status
    pub fn transition_to_candidate(&mut self) {
        if self.status == Status::Cooldown {
            self.status = Status::Candidate;
            self.updated_at = Utc::now();
        }
    }

    /// Archive this memory (soft delete)
    pub fn archive(&mut self) {
        self.status = Status::Archived;
        self.updated_at = Utc::now();
    }

    /// Update decay score
    pub fn update_decay_score(&mut self, new_score: f32) {
        self.decay_score = new_score;
        self.updated_at = Utc::now();
    }

    /// Check if this memory should transition to cooldown
    pub fn should_cooldown(&self, threshold_days: i64) -> bool {
        if self.status != Status::Active {
            return false;
        }
        let last_activity = self.last_hit_at.unwrap_or(self.created_at);
        let days_since = (Utc::now() - last_activity).num_days();
        days_since > threshold_days
    }

    /// Check if this memory should transition to candidate
    pub fn should_become_candidate(&self, threshold_days: i64) -> bool {
        if self.status != Status::Cooldown {
            return false;
        }
        let last_activity = self.last_hit_at.unwrap_or(self.created_at);
        let days_since = (Utc::now() - last_activity).num_days();
        days_since > threshold_days
    }

    /// Check if this memory should be archived
    pub fn should_archive(&self, threshold_days: i64) -> bool {
        if self.status != Status::Candidate {
            return false;
        }
        let last_activity = self.last_hit_at.unwrap_or(self.created_at);
        let days_since = (Utc::now() - last_activity).num_days();
        days_since > threshold_days
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_input() -> CreateMemoryInput {
        CreateMemoryInput {
            profile_id: Uuid::new_v4(),
            owner_id: "owner123".to_string(),
            scope_id: Some("scope456".to_string()),
            content: "Test memory content".to_string(),
            category: Some("work.code".to_string()),
            tags: Some(vec!["test".to_string()]),
            importance: Some(0.7),
            confidence: Some(0.9),
            is_global: false,
            embedding_provider: None,
            process_with_llm: false,
            llm_provider: None,
        }
    }

    fn valid_event_input() -> CreateMemoryFromEventInput {
        CreateMemoryFromEventInput {
            profile_id: Uuid::new_v4(),
            owner_id: "owner123".to_string(),
            scope_id: Some("scope456".to_string()),
            content: "User prefers dark mode".to_string(),
            category: Some("preference.ui".to_string()),
            tags: Some(vec!["preference".to_string(), "ui".to_string()]),
            importance: 0.8,
            confidence: 0.95,
            source_event_id: Uuid::new_v4(),
            embedding_provider: Some("openai".to_string()),
            llm_provider: Some("openai".to_string()),
            inference_type: InferenceType::Preference,
            inference_confidence: 0.9,
            inference_reasoning: "User explicitly stated preference".to_string(),
        }
    }

    #[test]
    fn test_valid_memory_creation() {
        let input = valid_input();
        let result = CreateMemoryValidation::validate(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_owner_id_rejection() {
        let mut input = valid_input();
        input.owner_id = "".to_string();
        let result = CreateMemoryValidation::validate(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_whitespace_owner_id_rejection() {
        let mut input = valid_input();
        input.owner_id = "   ".to_string();
        let result = CreateMemoryValidation::validate(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_empty_content_rejection() {
        let mut input = valid_input();
        input.content = "".to_string();
        let result = CreateMemoryValidation::validate(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_importance_out_of_range_rejection() {
        let mut input = valid_input();
        input.importance = Some(1.5);
        let result = CreateMemoryValidation::validate(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));

        input.importance = Some(-0.1);
        let result = CreateMemoryValidation::validate(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_confidence_out_of_range_rejection() {
        let mut input = valid_input();
        input.confidence = Some(1.5);
        let result = CreateMemoryValidation::validate(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_memory_creation_with_defaults() {
        let input = CreateMemoryInput {
            profile_id: Uuid::new_v4(),
            owner_id: "owner123".to_string(),
            scope_id: None,
            content: "Test content".to_string(),
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
        assert_eq!(memory.importance, 0.5);
        assert_eq!(memory.confidence, 1.0);
        assert_eq!(memory.status, Status::Active);
        assert_eq!(memory.hit_count, 0);
        assert!(memory.last_hit_at.is_none());
        assert_eq!(memory.reinforcement_count, 0);
        assert!(memory.last_reinforced_at.is_none());
        assert_eq!(memory.embedding_status, EmbeddingStatus::Pending);
        assert_eq!(memory.processing_status, ProcessingStatus::Skipped);
        // Version chain
        assert_eq!(memory.root_memory_id, Some(memory.id));
        assert_eq!(memory.version_number, 1);
        assert!(memory.is_current_version);
        assert!(memory.supersedes.is_none());
        assert!(memory.superseded_by.is_none());
        // Inference fields should be None for directly created memories
        assert!(memory.inference_type.is_none());
        assert!(memory.inference_confidence.is_none());
        assert!(memory.inference_reasoning.is_none());
        assert!(memory.conflict_reason.is_none());
        assert!(memory.consistency_confidence.is_none());
    }

    #[test]
    fn test_memory_from_event() {
        let input = valid_event_input();
        let source_event_id = input.source_event_id;

        let memory = Memory::new_from_event(input);
        assert_eq!(memory.source_event_id, Some(source_event_id));
        assert_eq!(memory.inference_type, Some(InferenceType::Preference));
        assert_eq!(memory.inference_confidence, Some(0.9));
        assert!(memory.inference_reasoning.is_some());
        assert_eq!(memory.processing_status, ProcessingStatus::Completed);
        // Version chain
        assert_eq!(memory.root_memory_id, Some(memory.id));
        assert_eq!(memory.version_number, 1);
        assert!(memory.is_current_version);
    }

    #[test]
    fn test_memory_superseding() {
        let first_input = valid_event_input();
        let profile_id = first_input.profile_id;
        let first_memory = Memory::new_from_event(first_input);
        let first_id = first_memory.id;
        let root_id = first_memory.root_memory_id.unwrap();

        let superseding_input = CreateSupersedingMemoryInput {
            old_memory_id: first_id,
            root_memory_id: root_id,
            version_number: 2,
            profile_id,
            owner_id: first_memory.owner_id.clone(),
            scope_id: first_memory.scope_id.clone(),
            content: "User now prefers light mode".to_string(),
            category: Some("preference.ui".to_string()),
            tags: Some(vec!["preference".to_string()]),
            importance: 0.8,
            confidence: 0.95,
            is_global: false,
            source_event_id: Uuid::new_v4(),
            embedding_provider: None,
            llm_provider: None,
            inference_type: InferenceType::Preference,
            inference_confidence: 0.9,
            inference_reasoning: "User changed preference".to_string(),
        };

        let new_memory = Memory::new_superseding(superseding_input);
        assert_eq!(new_memory.root_memory_id, Some(root_id));
        assert_eq!(new_memory.version_number, 2);
        assert!(new_memory.is_current_version);
        assert_eq!(new_memory.supersedes, Some(first_id));
        assert!(new_memory.superseded_by.is_none());
    }

    #[test]
    fn test_mark_superseded() {
        let input = valid_event_input();
        let mut memory = Memory::new_from_event(input);
        let new_version_id = Uuid::new_v4();

        memory.mark_superseded(new_version_id);

        assert_eq!(memory.superseded_by, Some(new_version_id));
        assert!(!memory.is_current_version);
        assert_eq!(memory.status, Status::Superseded);
    }

    #[test]
    fn test_memory_record_hit() {
        let input = valid_input();
        let mut memory = Memory::new(input);

        assert_eq!(memory.hit_count, 0);
        assert!(memory.last_hit_at.is_none());

        memory.record_hit();

        assert_eq!(memory.hit_count, 1);
        assert!(memory.last_hit_at.is_some());
    }

    #[test]
    fn test_memory_cooldown_recovery() {
        let input = valid_input();
        let mut memory = Memory::new(input);
        memory.status = Status::Cooldown;

        memory.record_hit();

        assert_eq!(memory.status, Status::Active);
    }

    #[test]
    fn test_memory_reinforce() {
        let input = valid_input();
        let mut memory = Memory::new(input);
        let initial_confidence = memory.confidence;

        memory.reinforce(0.05);

        assert!(memory.confidence > initial_confidence);
        assert_eq!(memory.hit_count, 1);
        assert_eq!(memory.reinforcement_count, 1);
        assert!(memory.last_reinforced_at.is_some());
    }

    #[test]
    fn test_memory_record_conflict() {
        let input = valid_input();
        let mut memory = Memory::new(input);

        memory.record_conflict("Conflicts with prior preference".to_string(), 0.86);

        assert_eq!(
            memory.conflict_reason,
            Some("Conflicts with prior preference".to_string())
        );
        assert_eq!(memory.consistency_confidence, Some(0.86));
    }

    #[test]
    fn test_memory_reinforce_caps_at_one() {
        let input = valid_input();
        let mut memory = Memory::new(input);
        memory.confidence = 0.98;

        memory.reinforce(0.1);

        assert_eq!(memory.confidence, 1.0);
    }

    #[test]
    fn test_memory_promote_to_global() {
        let input = valid_input();
        let mut memory = Memory::new(input);
        assert!(!memory.is_global);
        assert!(memory.scope_id.is_some());

        memory.promote_to_global("Cross-scope reinforcement");

        assert!(memory.is_global);
        assert!(memory.scope_id.is_none());
        assert!(memory.promoted_at.is_some());
        assert_eq!(
            memory.promotion_reason,
            Some("Cross-scope reinforcement".to_string())
        );
    }

    #[test]
    fn test_memory_archive() {
        let input = valid_input();
        let mut memory = Memory::new(input);

        memory.archive();

        assert_eq!(memory.status, Status::Archived);
    }

    #[test]
    fn test_memory_status_transitions() {
        let input = valid_input();
        let mut memory = Memory::new(input);
        assert_eq!(memory.status, Status::Active);

        memory.transition_to_cooldown();
        assert_eq!(memory.status, Status::Cooldown);

        memory.transition_to_candidate();
        assert_eq!(memory.status, Status::Candidate);

        memory.archive();
        assert_eq!(memory.status, Status::Archived);
    }

    #[test]
    fn test_memory_with_llm_processing_enabled() {
        let mut input = valid_input();
        input.process_with_llm = true;
        input.llm_provider = Some("openai".to_string());

        let memory = Memory::new(input);
        assert_eq!(memory.processing_status, ProcessingStatus::Pending);
        assert_eq!(memory.llm_provider, Some("openai".to_string()));
    }

    #[test]
    fn test_global_memory_creation() {
        let mut input = valid_input();
        input.is_global = true;
        input.scope_id = None;

        let memory = Memory::new(input);
        assert!(memory.is_global);
        assert!(memory.scope_id.is_none());
    }
}
