//! MemoryGuard service
//!
//! Responsible for write validation, layer constraints, update mode processing,
//! and optional LLM-driven memory processing (compression, classification, tag extraction).
//! Acts as a gatekeeper for all memory write operations.
//!
//! Extended to support event-to-memory extraction and query enhancement.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::domain::{
    CreateMemoryFromEventInput, CreateMemoryInput, CreateMemoryValidation, ExtractedMemory, Layer,
    Memory, ProcessingMode, ProcessingStatus, ScopeType, UpdateMode,
};
use crate::error::{AppError, AppResult};
use crate::repository::{AuditOperation, AuditRepository, MemoryRepository, UpdateMemoryInput};

use super::{
    ExtractFromEventRequest, MemoryProcessor, ProcessMemoryRequest, RetrievalEngine,
    RetrieveRequest, RetrieveResponse,
};

/// Request for updating a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateMemoryRequest {
    /// Update mode (append, merge, supersede)
    pub mode: UpdateMode,
    /// New content (optional)
    pub content: Option<String>,
    /// New importance value (optional)
    pub importance: Option<f32>,
    /// New confidence value (optional)
    pub confidence: Option<f32>,
    /// New TTL in seconds (optional)
    pub ttl_seconds: Option<i64>,
    /// Whether to process updated content with LLM
    #[serde(default)]
    pub process_with_llm: bool,
}

impl UpdateMemoryRequest {
    /// Validate the update request
    pub fn validate(&self) -> AppResult<()> {
        // Validate importance range if provided
        if let Some(importance) = self.importance {
            if !(0.0..=1.0).contains(&importance) {
                return Err(AppError::Validation(
                    "importance must be between 0.0 and 1.0".to_string(),
                ));
            }
        }

        // Validate confidence range if provided
        if let Some(confidence) = self.confidence {
            if !(0.0..=1.0).contains(&confidence) {
                return Err(AppError::Validation(
                    "confidence must be between 0.0 and 1.0".to_string(),
                ));
            }
        }

        // Validate TTL is positive if provided
        if let Some(ttl) = self.ttl_seconds {
            if ttl <= 0 {
                return Err(AppError::Validation(
                    "ttl_seconds must be positive".to_string(),
                ));
            }
        }

        // Validate content is not empty if provided
        if let Some(ref content) = self.content {
            if content.trim().is_empty() {
                return Err(AppError::Validation("content cannot be empty".to_string()));
            }
        }

        Ok(())
    }
}

/// Request for creating memories from an event
///
/// This struct represents a request to extract and optionally create memories
/// from raw event content. The LLM will automatically understand the event type
/// and extract relevant memories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFromEventRequest {
    /// Event content (any form: click description, conversation history, operation log, etc.)
    pub content: String,
    /// Optional context to help LLM better understand the event
    #[serde(default)]
    pub context: Option<String>,
    /// Scope type for created memories
    pub scope_type: ScopeType,
    /// Scope identifier for created memories
    pub scope_id: String,
    /// Usage scene for created memories
    pub scene: String,
    /// Processing mode (auto, assisted, manual)
    /// Defaults to "assisted" if not specified
    #[serde(default)]
    pub mode: Option<ProcessingMode>,
}

/// Result of creating memories from an event
///
/// Contains the event summary, extracted memories, and optionally the IDs
/// of created memories (only in "auto" mode).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFromEventResult {
    /// Event summary (LLM-generated understanding of the event)
    pub event_summary: String,
    /// List of extracted memories
    pub extracted_memories: Vec<ExtractedMemory>,
    /// IDs of created memories (only populated in "auto" mode)
    pub created_memory_ids: Option<Vec<Uuid>>,
}

/// MemoryGuard service for write validation and constraint enforcement
#[derive(Clone)]
pub struct MemoryGuard {
    memory_repo: MemoryRepository,
    audit_repo: AuditRepository,
    audit_enabled: bool,
    /// Optional memory processor for LLM-driven processing
    memory_processor: Option<Arc<MemoryProcessor>>,
    /// Optional retrieval engine for query enhancement
    retrieval_engine: Option<Arc<RetrievalEngine>>,
}

impl MemoryGuard {
    /// Create a new MemoryGuard service
    pub fn new(
        memory_repo: MemoryRepository,
        audit_repo: AuditRepository,
        audit_enabled: bool,
    ) -> Self {
        Self {
            memory_repo,
            audit_repo,
            audit_enabled,
            memory_processor: None,
            retrieval_engine: None,
        }
    }

    /// Create a new MemoryGuard service with LLM processing support
    pub fn with_processor(
        memory_repo: MemoryRepository,
        audit_repo: AuditRepository,
        audit_enabled: bool,
        memory_processor: Arc<MemoryProcessor>,
    ) -> Self {
        Self {
            memory_repo,
            audit_repo,
            audit_enabled,
            memory_processor: Some(memory_processor),
            retrieval_engine: None,
        }
    }

    /// Set the memory processor
    pub fn set_processor(&mut self, processor: Arc<MemoryProcessor>) {
        self.memory_processor = Some(processor);
    }

    /// Set the retrieval engine
    pub fn set_retrieval_engine(&mut self, engine: Arc<RetrievalEngine>) {
        self.retrieval_engine = Some(engine);
    }

    /// Create a new memory with validation
    ///
    /// Validates the input and enforces layer constraints:
    /// - Long-term memories cannot be created directly
    /// - All required fields must be valid
    ///
    /// If `process_with_llm` is true and a MemoryProcessor is configured,
    /// the content will be processed (compressed, classified, tags extracted).
    /// On processing failure, falls back to storing raw content.
    pub async fn create_memory(
        &self,
        input: CreateMemoryInput,
        actor_id: Option<String>,
    ) -> AppResult<Memory> {
        debug!(
            layer = %input.layer,
            scope_type = %input.scope_type,
            scope_id = %input.scope_id,
            scene = %input.scene,
            process_with_llm = input.process_with_llm,
            "Creating memory"
        );

        // Validate input (includes layer constraint check)
        CreateMemoryValidation::validate(&input)?;

        // Create the memory entity
        let mut memory = Memory::new(input.clone());

        // Process with LLM if requested and processor is available
        if input.process_with_llm {
            if let Some(ref processor) = self.memory_processor {
                let process_request = ProcessMemoryRequest::new(&input.content);

                match processor.process(process_request).await {
                    Ok(result) => {
                        debug!(
                            memory_id = %memory.id,
                            category = %result.category,
                            tag_count = result.tags.len(),
                            "LLM processing completed"
                        );
                        memory.apply_processing_result(
                            result.processed_content,
                            result.category,
                            result.tags,
                        );
                    }
                    Err(e) => {
                        warn!(
                            memory_id = %memory.id,
                            error = %e,
                            "LLM processing failed, falling back to raw content"
                        );
                        memory.mark_processing_failed();
                    }
                }
            } else {
                warn!(
                    memory_id = %memory.id,
                    "LLM processing requested but no processor configured, skipping"
                );
                memory.processing_status = ProcessingStatus::Skipped;
            }
        }

        // Persist to database
        let created = self.memory_repo.create(&memory).await?;

        // Create audit log entry
        if self.audit_enabled {
            let new_value = serde_json::to_value(&created).ok();
            self.audit_repo
                .create(
                    created.id,
                    AuditOperation::Create,
                    actor_id,
                    None,
                    new_value,
                    None,
                )
                .await?;
        }

        info!(
            memory_id = %created.id,
            layer = %created.layer,
            processing_status = %created.processing_status,
            "Memory created successfully"
        );

        Ok(created)
    }

    /// Get a memory by ID
    pub async fn get_memory(&self, id: Uuid) -> AppResult<Memory> {
        self.memory_repo.get_by_id(id).await
    }

    /// Update a memory with validation
    ///
    /// Validates the update request and processes according to update mode:
    /// - Append: Appends new content to existing content
    /// - Merge: Merges new content with existing content (with separator)
    /// - Supersede: Replaces existing content with new content
    ///
    /// If `process_with_llm` is true and content is being updated,
    /// the combined content will be re-processed through LLM.
    pub async fn update_memory(
        &self,
        id: Uuid,
        request: UpdateMemoryRequest,
        actor_id: Option<String>,
    ) -> AppResult<Memory> {
        debug!(
            memory_id = %id,
            mode = %request.mode,
            process_with_llm = request.process_with_llm,
            "Updating memory"
        );

        // Validate update request
        request.validate()?;

        // Get existing memory for audit
        let old_memory = self.memory_repo.get_by_id(id).await?;
        let old_value = serde_json::to_value(&old_memory).ok();

        // Convert to repository input
        let repo_input = UpdateMemoryInput {
            mode: request.mode,
            content: request.content.clone(),
            importance: request.importance,
            confidence: request.confidence,
            ttl_seconds: request.ttl_seconds,
        };

        // Perform update
        let mut updated = self.memory_repo.update(id, &repo_input).await?;

        // Process with LLM if requested and content was updated
        if request.process_with_llm && request.content.is_some() {
            if let Some(ref processor) = self.memory_processor {
                let process_request = ProcessMemoryRequest::new(&updated.content);

                match processor.process(process_request).await {
                    Ok(result) => {
                        debug!(
                            memory_id = %id,
                            category = %result.category,
                            tag_count = result.tags.len(),
                            "LLM processing completed for update"
                        );
                        updated.apply_processing_result(
                            result.processed_content,
                            result.category,
                            result.tags,
                        );
                        // Update the memory in database with processing results
                        // Note: This is a simplified approach; in production you might want
                        // a separate method to update processing results
                    }
                    Err(e) => {
                        warn!(
                            memory_id = %id,
                            error = %e,
                            "LLM processing failed for update, keeping raw content"
                        );
                        updated.mark_processing_failed();
                    }
                }
            } else {
                warn!(
                    memory_id = %id,
                    "LLM processing requested but no processor configured"
                );
            }
        }

        // Create audit log entry
        if self.audit_enabled {
            let new_value = serde_json::to_value(&updated).ok();
            self.audit_repo
                .create(
                    id,
                    AuditOperation::Update,
                    actor_id,
                    old_value,
                    new_value,
                    Some(format!("Update mode: {}", request.mode)),
                )
                .await?;
        }

        info!(
            memory_id = %id,
            mode = %request.mode,
            "Memory updated successfully"
        );

        Ok(updated)
    }

    /// Delete (archive) a memory
    ///
    /// Performs soft delete by setting status to 'archived'.
    /// Content is preserved for audit purposes.
    pub async fn delete_memory(&self, id: Uuid, actor_id: Option<String>) -> AppResult<Memory> {
        debug!(memory_id = %id, "Deleting memory");

        // Get existing memory for audit
        let old_memory = self.memory_repo.get_by_id(id).await?;
        let old_value = serde_json::to_value(&old_memory).ok();

        // Perform soft delete
        let deleted = self.memory_repo.delete(id).await?;

        // Create audit log entry
        if self.audit_enabled {
            let new_value = serde_json::to_value(&deleted).ok();
            self.audit_repo
                .create(
                    id,
                    AuditOperation::Delete,
                    actor_id,
                    old_value,
                    new_value,
                    Some("Soft delete - archived".to_string()),
                )
                .await?;
        }

        info!(memory_id = %id, "Memory deleted (archived) successfully");

        Ok(deleted)
    }

    /// Validate a scope type string
    ///
    /// Returns an error if the scope type is invalid.
    pub fn validate_scope_type(scope_type: &str) -> AppResult<()> {
        use crate::domain::ScopeType;

        if ScopeType::from_str(scope_type).is_none() {
            return Err(AppError::InvalidScopeType(scope_type.to_string()));
        }
        Ok(())
    }

    /// Validate an update mode string
    ///
    /// Returns an error if the update mode is invalid.
    pub fn validate_update_mode(mode: &str) -> AppResult<UpdateMode> {
        match mode.to_lowercase().as_str() {
            "append" => Ok(UpdateMode::Append),
            "merge" => Ok(UpdateMode::Merge),
            "supersede" => Ok(UpdateMode::Supersede),
            _ => Err(AppError::InvalidUpdateMode(mode.to_string())),
        }
    }

    /// Create memories from an event
    ///
    /// Extracts memories from raw event content using LLM processing.
    /// The behavior depends on the processing mode:
    /// - Auto: Extract and create memories without confirmation
    /// - Assisted: Return proposed memories for user approval (default)
    /// - Manual: Only summarize events without extraction
    ///
    /// # Arguments
    /// * `request` - The event extraction request containing content and metadata
    /// * `actor_id` - Optional actor ID for audit logging
    ///
    /// # Returns
    /// * `CreateFromEventResult` containing event summary, extracted memories,
    ///   and optionally created memory IDs (in auto mode)
    pub async fn create_from_event(
        &self,
        request: CreateFromEventRequest,
        actor_id: Option<String>,
    ) -> AppResult<CreateFromEventResult> {
        let mode = request.mode.unwrap_or_default();

        debug!(
            content_len = request.content.len(),
            scope_type = %request.scope_type,
            scope_id = %request.scope_id,
            scene = %request.scene,
            mode = %mode,
            "Creating memories from event"
        );

        // Ensure we have a memory processor
        let processor = self
            .memory_processor
            .as_ref()
            .ok_or_else(|| AppError::Internal("No memory processor configured".to_string()))?;

        // Handle manual mode - only summarize, no extraction
        if mode == ProcessingMode::Manual {
            let summary = if processor.needs_summary(&request.content) {
                processor
                    .summarize_conversation(&request.content)
                    .await
                    .map_err(|e| AppError::Internal(e.to_string()))?
            } else {
                request.content.clone()
            };

            return Ok(CreateFromEventResult {
                event_summary: summary,
                extracted_memories: Vec::new(),
                created_memory_ids: None,
            });
        }

        // Extract memories from the event
        let extract_request = ExtractFromEventRequest {
            content: request.content.clone(),
            context: request.context.clone(),
        };

        let extract_result = processor
            .extract_from_event(extract_request)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;

        // In assisted mode, just return the extracted memories without creating them
        if mode == ProcessingMode::Assisted {
            return Ok(CreateFromEventResult {
                event_summary: extract_result.event_summary,
                extracted_memories: extract_result.extracted_memories,
                created_memory_ids: None,
            });
        }

        // Auto mode: create memories from extracted results
        let mut created_ids = Vec::new();

        for extracted in &extract_result.extracted_memories {
            let memory_input = CreateMemoryFromEventInput {
                layer: Layer::Session, // Default to session layer
                scope_type: request.scope_type,
                scope_id: request.scope_id.clone(),
                scene: request.scene.clone(),
                content: extracted.content.clone(),
                raw_content: Some(request.content.clone()),
                category: Some(extracted.category),
                tags: Some(extracted.tags.clone()),
                importance: extracted.importance,
                confidence: extracted.confidence,
                ttl_seconds: None,
                event_source: None,
                event_time: None,
                embedding_provider: None,
                llm_provider: None,
                inference_type: extracted.inference_type,
                inference_confidence: extracted.confidence,
                inference_reasoning: extracted.reasoning.clone(),
            };

            let memory = Memory::new_from_event(memory_input);
            let created = self.memory_repo.create(&memory).await?;

            // Create audit log entry
            if self.audit_enabled {
                let new_value = serde_json::to_value(&created).ok();
                self.audit_repo
                    .create(
                        created.id,
                        AuditOperation::Create,
                        actor_id.clone(),
                        None,
                        new_value,
                        Some("Created from event extraction".to_string()),
                    )
                    .await?;
            }

            created_ids.push(created.id);
        }

        info!(
            memory_count = created_ids.len(),
            mode = %mode,
            "Memories created from event"
        );

        Ok(CreateFromEventResult {
            event_summary: extract_result.event_summary,
            extracted_memories: extract_result.extracted_memories,
            created_memory_ids: Some(created_ids),
        })
    }

    /// Retrieve memories with optional query enhancement
    ///
    /// If `enhance` is true and a MemoryProcessor is configured, the query
    /// will be expanded with semantic synonyms before retrieval.
    /// If `enhance` is false, the original query is used directly.
    ///
    /// # Arguments
    /// * `request` - The retrieval request
    /// * `enhance` - Whether to enhance the query with semantic expansion
    /// * `similarities` - Optional vector similarities from Qdrant
    /// * `actor_id` - Optional actor ID for audit logging
    ///
    /// # Returns
    /// * `RetrieveResponse` containing matched memories
    pub async fn retrieve_with_enhancement(
        &self,
        mut request: RetrieveRequest,
        enhance: bool,
        similarities: Option<Vec<(Uuid, f32)>>,
        actor_id: Option<String>,
    ) -> AppResult<RetrieveResponse> {
        debug!(
            query = %request.query,
            enhance = enhance,
            "Retrieving memories with enhancement option"
        );

        // Enhance query if requested and processor is available
        if enhance {
            if let Some(ref processor) = self.memory_processor {
                match processor.enhance_query(&request.query, None).await {
                    Ok(enhanced) => {
                        debug!(
                            original = %request.query,
                            enhanced = %enhanced.enhanced_query,
                            "Query enhanced"
                        );
                        request.query = enhanced.enhanced_query;
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            "Query enhancement failed, using original query"
                        );
                        // Continue with original query on enhancement failure
                    }
                }
            } else {
                warn!("Query enhancement requested but no processor configured");
            }
        }

        // Ensure we have a retrieval engine
        let engine = self
            .retrieval_engine
            .as_ref()
            .ok_or_else(|| AppError::Internal("No retrieval engine configured".to_string()))?;

        // Perform retrieval
        engine.retrieve(request, similarities, actor_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_request_validation_valid() {
        let request = UpdateMemoryRequest {
            mode: UpdateMode::Append,
            content: Some("New content".to_string()),
            importance: Some(0.8),
            confidence: Some(0.9),
            ttl_seconds: Some(3600),
            process_with_llm: false,
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn test_update_request_validation_invalid_importance() {
        let request = UpdateMemoryRequest {
            mode: UpdateMode::Append,
            content: None,
            importance: Some(1.5),
            confidence: None,
            ttl_seconds: None,
            process_with_llm: false,
        };
        assert!(matches!(request.validate(), Err(AppError::Validation(_))));
    }

    #[test]
    fn test_update_request_validation_invalid_confidence() {
        let request = UpdateMemoryRequest {
            mode: UpdateMode::Merge,
            content: None,
            importance: None,
            confidence: Some(-0.1),
            ttl_seconds: None,
            process_with_llm: false,
        };
        assert!(matches!(request.validate(), Err(AppError::Validation(_))));
    }

    #[test]
    fn test_update_request_validation_invalid_ttl() {
        let request = UpdateMemoryRequest {
            mode: UpdateMode::Supersede,
            content: None,
            importance: None,
            confidence: None,
            ttl_seconds: Some(-100),
            process_with_llm: false,
        };
        assert!(matches!(request.validate(), Err(AppError::Validation(_))));
    }

    #[test]
    fn test_update_request_validation_empty_content() {
        let request = UpdateMemoryRequest {
            mode: UpdateMode::Append,
            content: Some("   ".to_string()),
            importance: None,
            confidence: None,
            ttl_seconds: None,
            process_with_llm: false,
        };
        assert!(matches!(request.validate(), Err(AppError::Validation(_))));
    }

    #[test]
    fn test_validate_scope_type_valid() {
        assert!(MemoryGuard::validate_scope_type("user").is_ok());
        assert!(MemoryGuard::validate_scope_type("org").is_ok());
        assert!(MemoryGuard::validate_scope_type("project").is_ok());
        assert!(MemoryGuard::validate_scope_type("task").is_ok());
        assert!(MemoryGuard::validate_scope_type("session").is_ok());
    }

    #[test]
    fn test_validate_scope_type_invalid() {
        let result = MemoryGuard::validate_scope_type("invalid");
        assert!(matches!(result, Err(AppError::InvalidScopeType(_))));
    }

    #[test]
    fn test_validate_update_mode_valid() {
        assert_eq!(
            MemoryGuard::validate_update_mode("append").unwrap(),
            UpdateMode::Append
        );
        assert_eq!(
            MemoryGuard::validate_update_mode("MERGE").unwrap(),
            UpdateMode::Merge
        );
        assert_eq!(
            MemoryGuard::validate_update_mode("Supersede").unwrap(),
            UpdateMode::Supersede
        );
    }

    #[test]
    fn test_validate_update_mode_invalid() {
        let result = MemoryGuard::validate_update_mode("invalid");
        assert!(matches!(result, Err(AppError::InvalidUpdateMode(_))));
    }
}
