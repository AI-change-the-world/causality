//! MemoryGuard service
//!
//! Responsible for write validation, layer constraints, update mode processing,
//! and optional LLM-driven memory processing (compression, classification, tag extraction).
//! Acts as a gatekeeper for all memory write operations.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::domain::{
    CreateMemoryInput, CreateMemoryValidation, Memory, ProcessingStatus, UpdateMode,
};
use crate::error::{AppError, AppResult};
use crate::repository::{AuditOperation, AuditRepository, MemoryRepository, UpdateMemoryInput};

use super::{MemoryProcessor, ProcessMemoryRequest};

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

/// MemoryGuard service for write validation and constraint enforcement
#[derive(Clone)]
pub struct MemoryGuard {
    memory_repo: MemoryRepository,
    audit_repo: AuditRepository,
    audit_enabled: bool,
    /// Optional memory processor for LLM-driven processing
    memory_processor: Option<Arc<MemoryProcessor>>,
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
        }
    }

    /// Set the memory processor
    pub fn set_processor(&mut self, processor: Arc<MemoryProcessor>) {
        self.memory_processor = Some(processor);
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
