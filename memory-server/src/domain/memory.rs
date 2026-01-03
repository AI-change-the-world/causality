//! Memory entity and validation logic
//!
//! The Memory struct represents a structured context/conclusion/rule stored in the system.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    EmbeddingStatus, InferenceType, Layer, MemoryCategory, ProcessingStatus, ScopeType, Status,
};
use crate::error::AppError;

/// Memory entity representing a structured context/conclusion/rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Unique identifier
    pub id: Uuid,
    /// Owner ID - the unique identifier of the memory owner
    pub owner_id: String,
    /// Memory layer (session, task, long_term)
    pub layer: Layer,
    /// Scope type (user, org, project, task, session)
    pub scope_type: ScopeType,
    /// Scope identifier
    pub scope_id: String,
    /// Usage scene (e.g., "work.contract_review")
    pub scene: String,
    /// Memory lifecycle status
    pub status: Status,
    /// Memory content (processed content if LLM processing was enabled)
    pub content: String,
    /// Memory category (auto-classified by LLM if processing was enabled)
    pub category: Option<MemoryCategory>,
    /// Tags/keywords extracted from content
    pub tags: Option<Vec<String>>,
    /// Importance score (0.0 - 1.0)
    pub importance: f32,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    /// Number of times this memory was retrieved
    pub hit_count: i64,
    /// Last time this memory was retrieved
    pub last_hit_at: Option<DateTime<Utc>>,
    /// Time-to-live in seconds
    pub ttl_seconds: Option<i64>,
    /// Expiration timestamp
    pub expires_at: Option<DateTime<Utc>>,
    /// Event source that triggered memory creation (e.g., "button_click:like")
    pub event_source: Option<String>,
    /// Timestamp when the triggering event occurred
    pub event_time: Option<DateTime<Utc>>,
    /// Embedding generation status
    pub embedding_status: EmbeddingStatus,
    /// Embedding provider used
    pub embedding_provider: Option<String>,
    /// LLM processing status
    pub processing_status: ProcessingStatus,
    /// LLM provider used for processing
    pub llm_provider: Option<String>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
    /// Inference type (if memory was extracted from an event)
    pub inference_type: Option<InferenceType>,
    /// Inference confidence score (0.0 - 1.0, if memory was extracted from an event)
    pub inference_confidence: Option<f32>,
    /// Reasoning for the inference (if memory was extracted from an event)
    pub inference_reasoning: Option<String>,
    /// When the memory was promoted to long-term
    pub promoted_at: Option<DateTime<Utc>>,
    /// Reason for promotion to long-term
    pub promotion_reason: Option<String>,
}

/// Input for creating a new memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMemoryInput {
    /// Owner ID - the unique identifier of the memory owner
    pub owner_id: String,
    /// Memory layer (session or task only - long_term is rejected)
    pub layer: Layer,
    /// Scope type
    pub scope_type: ScopeType,
    /// Scope identifier
    pub scope_id: String,
    /// Usage scene
    pub scene: String,
    /// Memory content
    pub content: String,
    /// Importance score (0.0 - 1.0), defaults to 0.5
    pub importance: Option<f32>,
    /// Confidence score (0.0 - 1.0), defaults to 1.0
    pub confidence: Option<f32>,
    /// Time-to-live in seconds
    pub ttl_seconds: Option<i64>,
    /// Event source that triggered memory creation
    pub event_source: Option<String>,
    /// Timestamp when the triggering event occurred
    pub event_time: Option<DateTime<Utc>>,
    /// Embedding provider to use (uses default if not specified)
    pub embedding_provider: Option<String>,
    /// Whether to process content with LLM (compression, classification, tag extraction)
    #[serde(default)]
    pub process_with_llm: bool,
    /// LLM provider to use for processing (uses default if not specified)
    pub llm_provider: Option<String>,
}

/// Input for creating a memory from an event extraction
///
/// This struct contains all fields needed to create a memory from an event
/// that has been processed by the LLM. It includes inference-related fields
/// that track the origin and confidence of the extracted memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMemoryFromEventInput {
    /// Owner ID - the unique identifier of the memory owner
    pub owner_id: String,
    /// Memory layer (session or task only - long_term is rejected)
    pub layer: Layer,
    /// Scope type
    pub scope_type: ScopeType,
    /// Scope identifier
    pub scope_id: String,
    /// Usage scene
    pub scene: String,
    /// Memory content (extracted by LLM)
    pub content: String,
    /// Memory category (auto-classified by LLM)
    pub category: Option<MemoryCategory>,
    /// Tags/keywords extracted from content
    pub tags: Option<Vec<String>>,
    /// Importance score (0.0 - 1.0), auto-evaluated by LLM
    pub importance: f32,
    /// Confidence score (0.0 - 1.0), auto-evaluated by LLM
    pub confidence: f32,
    /// Time-to-live in seconds
    pub ttl_seconds: Option<i64>,
    /// Event source that triggered memory creation
    pub event_source: Option<String>,
    /// Timestamp when the triggering event occurred
    pub event_time: Option<DateTime<Utc>>,
    /// Embedding provider to use (uses default if not specified)
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

/// Validation result for memory creation
#[derive(Debug)]
pub struct CreateMemoryValidation;

impl CreateMemoryValidation {
    /// Validate a memory creation request
    ///
    /// Returns Ok(()) if valid, or an AppError if validation fails.
    ///
    /// Validation rules:
    /// - Layer cannot be LongTerm (requires manual confirmation)
    /// - owner_id cannot be empty
    /// - scope_id cannot be empty
    /// - scene cannot be empty
    /// - content cannot be empty
    /// - importance must be between 0.0 and 1.0
    /// - confidence must be between 0.0 and 1.0
    pub fn validate(input: &CreateMemoryInput) -> Result<(), AppError> {
        // Rule: Long-term memories cannot be created directly
        if !input.layer.allows_direct_creation() {
            return Err(AppError::InvalidLayer);
        }

        // Validate owner_id is not empty
        if input.owner_id.trim().is_empty() {
            return Err(AppError::Validation("owner_id cannot be empty".to_string()));
        }

        // Validate scope_id is not empty
        if input.scope_id.trim().is_empty() {
            return Err(AppError::Validation("scope_id cannot be empty".to_string()));
        }

        // Validate scene is not empty
        if input.scene.trim().is_empty() {
            return Err(AppError::Validation("scene cannot be empty".to_string()));
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

        // Validate TTL is positive if provided
        if let Some(ttl) = input.ttl_seconds {
            if ttl <= 0 {
                return Err(AppError::Validation(
                    "ttl_seconds must be positive".to_string(),
                ));
            }
        }

        Ok(())
    }
}

impl Memory {
    /// Create a new Memory from validated input
    ///
    /// This should only be called after validation passes.
    /// Note: LLM processing should be done separately after creation.
    pub fn new(input: CreateMemoryInput) -> Self {
        let now = Utc::now();
        let ttl_seconds = input
            .ttl_seconds
            .or_else(|| input.layer.default_ttl_seconds());
        let expires_at = ttl_seconds.map(|ttl| now + chrono::Duration::seconds(ttl));

        // Determine initial processing status based on process_with_llm flag
        let processing_status = if input.process_with_llm {
            ProcessingStatus::Pending
        } else {
            ProcessingStatus::Skipped
        };

        Memory {
            id: Uuid::new_v4(),
            owner_id: input.owner_id,
            layer: input.layer,
            scope_type: input.scope_type,
            scope_id: input.scope_id,
            scene: input.scene,
            status: Status::Active,
            content: input.content,
            category: None,
            tags: None,
            importance: input.importance.unwrap_or(0.5),
            confidence: input.confidence.unwrap_or(1.0),
            hit_count: 0,
            last_hit_at: None,
            ttl_seconds,
            expires_at,
            event_source: input.event_source,
            event_time: input.event_time,
            embedding_status: EmbeddingStatus::Pending,
            embedding_provider: input.embedding_provider,
            processing_status,
            llm_provider: input.llm_provider,
            created_at: now,
            updated_at: now,
            inference_type: None,
            inference_confidence: None,
            inference_reasoning: None,
            promoted_at: None,
            promotion_reason: None,
        }
    }

    /// Create a new Memory from an event extraction
    ///
    /// This creates a memory with all inference-related fields populated,
    /// indicating that this memory was extracted from an event by LLM processing.
    pub fn new_from_event(input: CreateMemoryFromEventInput) -> Self {
        let now = Utc::now();
        let ttl_seconds = input
            .ttl_seconds
            .or_else(|| input.layer.default_ttl_seconds());
        let expires_at = ttl_seconds.map(|ttl| now + chrono::Duration::seconds(ttl));

        Memory {
            id: Uuid::new_v4(),
            owner_id: input.owner_id,
            layer: input.layer,
            scope_type: input.scope_type,
            scope_id: input.scope_id,
            scene: input.scene,
            status: Status::Active,
            content: input.content,
            category: input.category,
            tags: input.tags,
            importance: input.importance,
            confidence: input.confidence,
            hit_count: 0,
            last_hit_at: None,
            ttl_seconds,
            expires_at,
            event_source: input.event_source,
            event_time: input.event_time,
            embedding_status: EmbeddingStatus::Pending,
            embedding_provider: input.embedding_provider,
            processing_status: ProcessingStatus::Completed, // Already processed by LLM
            llm_provider: input.llm_provider,
            created_at: now,
            updated_at: now,
            inference_type: Some(input.inference_type),
            inference_confidence: Some(input.inference_confidence),
            inference_reasoning: Some(input.inference_reasoning),
            promoted_at: None,
            promotion_reason: None,
        }
    }

    /// Apply LLM processing result to this memory
    pub fn apply_processing_result(
        &mut self,
        processed_content: String,
        category: MemoryCategory,
        tags: Vec<String>,
    ) {
        self.content = processed_content;
        self.category = Some(category);
        self.tags = Some(tags);
        self.processing_status = ProcessingStatus::Completed;
        self.updated_at = Utc::now();
    }

    /// Mark LLM processing as failed (fallback to raw content)
    pub fn mark_processing_failed(&mut self) {
        self.processing_status = ProcessingStatus::Failed;
        self.updated_at = Utc::now();
    }

    /// Check if this memory is expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            Utc::now() > expires_at
        } else {
            false
        }
    }

    /// Check if this memory should transition to cooldown
    pub fn should_cooldown(&self, threshold_seconds: i64) -> bool {
        if let Some(last_hit) = self.last_hit_at {
            let elapsed = (Utc::now() - last_hit).num_seconds();
            elapsed > threshold_seconds
        } else {
            // If never hit, check against creation time
            let elapsed = (Utc::now() - self.created_at).num_seconds();
            elapsed > threshold_seconds
        }
    }

    /// Record a hit on this memory
    pub fn record_hit(&mut self) {
        self.hit_count += 1;
        self.last_hit_at = Some(Utc::now());
        self.updated_at = Utc::now();

        // If in cooldown, transition back to active
        if self.status == Status::Cooldown {
            self.status = Status::Active;
        }
    }

    /// Archive this memory (soft delete)
    pub fn archive(&mut self) {
        self.status = Status::Archived;
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_input() -> CreateMemoryInput {
        CreateMemoryInput {
            owner_id: "owner123".to_string(),
            layer: Layer::Session,
            scope_type: ScopeType::User,
            scope_id: "user123".to_string(),
            scene: "work.contract_review".to_string(),
            content: "Test memory content".to_string(),
            importance: Some(0.7),
            confidence: Some(0.9),
            ttl_seconds: Some(3600),
            event_source: Some("button_click:like".to_string()),
            event_time: Some(Utc::now()),
            embedding_provider: None,
            process_with_llm: false,
            llm_provider: None,
        }
    }

    #[test]
    fn test_valid_session_memory_creation() {
        let input = valid_input();
        let result = CreateMemoryValidation::validate(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_task_memory_creation() {
        let mut input = valid_input();
        input.layer = Layer::Task;
        let result = CreateMemoryValidation::validate(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_longterm_layer_rejection() {
        let mut input = valid_input();
        input.layer = Layer::LongTerm;
        let result = CreateMemoryValidation::validate(&input);
        assert!(matches!(result, Err(AppError::InvalidLayer)));
    }

    #[test]
    fn test_empty_scope_id_rejection() {
        let mut input = valid_input();
        input.scope_id = "".to_string();
        let result = CreateMemoryValidation::validate(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_whitespace_scope_id_rejection() {
        let mut input = valid_input();
        input.scope_id = "   ".to_string();
        let result = CreateMemoryValidation::validate(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_empty_scene_rejection() {
        let mut input = valid_input();
        input.scene = "".to_string();
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
    fn test_negative_ttl_rejection() {
        let mut input = valid_input();
        input.ttl_seconds = Some(-100);
        let result = CreateMemoryValidation::validate(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_memory_creation_with_defaults() {
        let input = CreateMemoryInput {
            owner_id: "owner123".to_string(),
            layer: Layer::Session,
            scope_type: ScopeType::User,
            scope_id: "user123".to_string(),
            scene: "test.scene".to_string(),
            content: "Test content".to_string(),
            importance: None,
            confidence: None,
            ttl_seconds: None,
            event_source: None,
            event_time: None,
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
        assert_eq!(memory.embedding_status, EmbeddingStatus::Pending);
        assert_eq!(memory.processing_status, ProcessingStatus::Skipped);
        assert!(memory.category.is_none());
        assert!(memory.tags.is_none());
        // Session layer has default TTL of 3600 seconds
        assert_eq!(memory.ttl_seconds, Some(3600));
        assert!(memory.expires_at.is_some());
        // Inference fields should be None for directly created memories
        assert!(memory.inference_type.is_none());
        assert!(memory.inference_confidence.is_none());
        assert!(memory.inference_reasoning.is_none());
        // Promotion fields
        assert!(memory.promoted_at.is_none());
        assert!(memory.promotion_reason.is_none());
    }

    #[test]
    fn test_memory_with_event_source() {
        let event_time = Utc::now();
        let input = CreateMemoryInput {
            owner_id: "owner123".to_string(),
            layer: Layer::Task,
            scope_type: ScopeType::Project,
            scope_id: "project456".to_string(),
            scene: "work.review".to_string(),
            content: "User prefers dark mode".to_string(),
            importance: Some(0.8),
            confidence: Some(0.95),
            ttl_seconds: None,
            event_source: Some("conversation:preference".to_string()),
            event_time: Some(event_time),
            embedding_provider: Some("openai".to_string()),
            process_with_llm: false,
            llm_provider: None,
        };

        let memory = Memory::new(input);
        assert_eq!(
            memory.event_source,
            Some("conversation:preference".to_string())
        );
        assert_eq!(memory.event_time, Some(event_time));
        assert_eq!(memory.embedding_provider, Some("openai".to_string()));
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
    fn test_memory_archive() {
        let input = valid_input();
        let mut memory = Memory::new(input);

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
    fn test_memory_apply_processing_result() {
        let mut input = valid_input();
        input.process_with_llm = true;

        let mut memory = Memory::new(input);
        assert_eq!(memory.processing_status, ProcessingStatus::Pending);

        memory.apply_processing_result(
            "Processed content".to_string(),
            MemoryCategory::UserPreference,
            vec!["tag1".to_string(), "tag2".to_string()],
        );

        assert_eq!(memory.content, "Processed content");
        assert_eq!(memory.category, Some(MemoryCategory::UserPreference));
        assert_eq!(
            memory.tags,
            Some(vec!["tag1".to_string(), "tag2".to_string()])
        );
        assert_eq!(memory.processing_status, ProcessingStatus::Completed);
    }

    #[test]
    fn test_memory_mark_processing_failed() {
        let mut input = valid_input();
        input.process_with_llm = true;

        let mut memory = Memory::new(input);
        memory.mark_processing_failed();

        assert_eq!(memory.processing_status, ProcessingStatus::Failed);
    }
}
