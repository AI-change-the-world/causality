//! MemoryGuard service
//!
//! Responsible for orchestrating the Event → Memory processing flow.
//! Integrates MemoryMatcher and MemoryReconciler to handle:
//! - Creating new memories from events
//! - Reinforcing existing memories when content is consistent
//! - Creating new versions (superseding) when content conflicts
//!
//! Requirements: 2.1, 3.1, 3.2, 3.3, 3.4

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::domain::{CreateEventInput, CreateMemoryInput, CreateMemoryValidation, Event, Memory};
use crate::embedding::{EmbeddingProvider, EmbeddingRequest};
use crate::error::{AppError, AppResult};
use crate::repository::{AuditOperation, AuditRepository, EventRepository, MemoryRepository};

use super::{
    ConsistencyChecker, ExtractFromEventRequest, ExtractedMemory, MemoryMatcher, MemoryProcessor,
    MemoryReconciler, ReconcileOutcome, RetrievalEngine, RetrieveRequest, RetrieveResponse,
};

/// Result of processing an event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessEventResult {
    /// The processed event
    pub event: Event,
    /// Number of memories created
    pub memories_created: usize,
    /// Number of memories reinforced
    pub memories_reinforced: usize,
    /// Number of memories superseded (new versions created)
    pub memories_superseded: usize,
    /// IDs of created memories
    pub created_memory_ids: Vec<Uuid>,
    /// IDs of reinforced memories
    pub reinforced_memory_ids: Vec<Uuid>,
    /// IDs of superseded memories (old versions)
    pub superseded_memory_ids: Vec<Uuid>,
}

/// Request for creating memories from an event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFromEventRequest {
    /// Owner ID - the unique identifier of the memory owner
    pub owner_id: String,
    /// Event content (any form: click description, conversation history, operation log, etc.)
    pub content: String,
    /// Optional context to help LLM better understand the event
    #[serde(default)]
    pub context: Option<String>,
    /// Scope identifier for created memories (optional, null = global context)
    pub scope_id: Option<String>,
    /// Event source (e.g., "user_created", "api", "conversation")
    pub source: Option<String>,
}

/// Result of creating memories from an event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFromEventResult {
    /// The created event
    pub event: Event,
    /// Event summary (LLM-generated understanding of the event)
    pub event_summary: String,
    /// Number of memories created
    pub memories_created: usize,
    /// Number of memories reinforced
    pub memories_reinforced: usize,
    /// Number of memories superseded
    pub memories_superseded: usize,
    /// IDs of created memories
    pub created_memory_ids: Vec<Uuid>,
}

/// Request for updating a memory's mutable metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateMemoryRequest {
    /// New confidence value (optional)
    pub confidence: Option<f32>,
    /// New decay score (optional)
    pub decay_score: Option<f32>,
}

impl UpdateMemoryRequest {
    /// Validate the update request
    pub fn validate(&self) -> AppResult<()> {
        // Validate confidence range if provided
        if let Some(confidence) = self.confidence {
            if !(0.0..=1.0).contains(&confidence) {
                return Err(AppError::Validation(
                    "confidence must be between 0.0 and 1.0".to_string(),
                ));
            }
        }

        // Validate decay_score is non-negative if provided
        if let Some(decay_score) = self.decay_score {
            if decay_score < 0.0 {
                return Err(AppError::Validation(
                    "decay_score must be non-negative".to_string(),
                ));
            }
        }

        Ok(())
    }
}

/// MemoryGuard service for orchestrating Event → Memory processing
pub struct MemoryGuard<C: ConsistencyChecker> {
    memory_repo: MemoryRepository,
    event_repo: EventRepository,
    audit_repo: AuditRepository,
    audit_enabled: bool,
    /// Memory processor for LLM-driven extraction
    memory_processor: Option<Arc<MemoryProcessor>>,
    /// Memory matcher for finding similar memories
    memory_matcher: Option<MemoryMatcher>,
    /// Memory reconciler for handling match outcomes
    memory_reconciler: Option<MemoryReconciler<C>>,
    /// Embedding provider for generating embeddings
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    /// Retrieval engine for query enhancement
    retrieval_engine: Option<Arc<RetrievalEngine>>,
}

impl<C: ConsistencyChecker> MemoryGuard<C> {
    /// Create a new MemoryGuard service
    pub fn new(
        memory_repo: MemoryRepository,
        event_repo: EventRepository,
        audit_repo: AuditRepository,
        audit_enabled: bool,
    ) -> Self {
        Self {
            memory_repo,
            event_repo,
            audit_repo,
            audit_enabled,
            memory_processor: None,
            memory_matcher: None,
            memory_reconciler: None,
            embedding_provider: None,
            retrieval_engine: None,
        }
    }

    /// Set the memory processor
    pub fn set_processor(&mut self, processor: Arc<MemoryProcessor>) {
        self.memory_processor = Some(processor);
    }

    /// Set the memory matcher
    pub fn set_matcher(&mut self, matcher: MemoryMatcher) {
        self.memory_matcher = Some(matcher);
    }

    /// Set the memory reconciler
    pub fn set_reconciler(&mut self, reconciler: MemoryReconciler<C>) {
        self.memory_reconciler = Some(reconciler);
    }

    /// Set the embedding provider
    pub fn set_embedding_provider(&mut self, provider: Arc<dyn EmbeddingProvider>) {
        self.embedding_provider = Some(provider);
    }

    /// Set the retrieval engine
    pub fn set_retrieval_engine(&mut self, engine: Arc<RetrievalEngine>) {
        self.retrieval_engine = Some(engine);
    }

    /// Create a new memory directly (not from event)
    ///
    /// Validates the input and creates a new memory.
    pub async fn create_memory(
        &self,
        input: CreateMemoryInput,
        actor_id: Option<String>,
    ) -> AppResult<Memory> {
        debug!(
            owner_id = %input.owner_id,
            scope_id = ?input.scope_id,
            process_with_llm = input.process_with_llm,
            "Creating memory directly"
        );

        // Validate input
        CreateMemoryValidation::validate(&input)?;

        // Create the memory entity
        let memory = Memory::new(input);

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
            "Memory created successfully"
        );

        Ok(created)
    }

    /// Get a memory by ID
    pub async fn get_memory(&self, id: Uuid) -> AppResult<Memory> {
        self.memory_repo.get_by_id(id).await
    }

    /// Update a memory's mutable metadata
    pub async fn update_memory(
        &self,
        id: Uuid,
        request: UpdateMemoryRequest,
        actor_id: Option<String>,
    ) -> AppResult<Memory> {
        debug!(
            memory_id = %id,
            "Updating memory metadata"
        );

        // Validate update request
        request.validate()?;

        // Get existing memory for audit
        let old_memory = self.memory_repo.get_by_id(id).await?;
        let old_value = serde_json::to_value(&old_memory).ok();

        // Perform update
        let updated = self
            .memory_repo
            .update_metadata(id, request.confidence, request.decay_score)
            .await?;

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
                    Some("Metadata update".to_string()),
                )
                .await?;
        }

        info!(
            memory_id = %id,
            "Memory metadata updated successfully"
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

    /// Process an event and create/reinforce/supersede memories
    ///
    /// This is the main entry point for the Event → Memory processing flow.
    /// It:
    /// 1. Stores the event
    /// 2. Extracts potential memories using LLM
    /// 3. For each extracted memory:
    ///    a. Generates embedding
    ///    b. Finds similar existing memories
    ///    c. Reconciles (create new, reinforce, or supersede)
    ///
    /// Requirements: 2.1, 3.1, 3.2, 3.3, 3.4
    pub async fn process_event(
        &self,
        event: &Event,
        actor_id: Option<String>,
    ) -> AppResult<ProcessEventResult> {
        debug!(
            event_id = %event.id,
            owner_id = %event.owner_id,
            scope_id = ?event.scope_id,
            "Processing event"
        );

        // Ensure we have required components
        let processor = self
            .memory_processor
            .as_ref()
            .ok_or_else(|| AppError::Internal("No memory processor configured".to_string()))?;

        let matcher = self
            .memory_matcher
            .as_ref()
            .ok_or_else(|| AppError::Internal("No memory matcher configured".to_string()))?;

        let reconciler = self
            .memory_reconciler
            .as_ref()
            .ok_or_else(|| AppError::Internal("No memory reconciler configured".to_string()))?;

        let embedding_provider = self
            .embedding_provider
            .as_ref()
            .ok_or_else(|| AppError::Internal("No embedding provider configured".to_string()))?;

        // Extract memories from the event using LLM
        let extract_request = ExtractFromEventRequest {
            content: event.content.clone(),
            context: event.context.clone(),
        };

        let extract_result = processor
            .extract_from_event(extract_request)
            .await
            .map_err(|e| AppError::Internal(format!("Memory extraction failed: {}", e)))?;

        // Track results
        let mut created_memory_ids = Vec::new();
        let mut reinforced_memory_ids = Vec::new();
        let mut superseded_memory_ids = Vec::new();

        // Process each extracted memory
        for extracted in &extract_result.extracted_memories {
            // Convert to our ExtractedMemory type
            let extracted_memory = ExtractedMemory {
                content: extracted.content.clone(),
                category: extracted.category.clone(),
                tags: extracted.tags.clone(),
                importance: extracted.importance,
                confidence: extracted.confidence,
                inference_type: extracted.inference_type,
                inference_confidence: extracted.confidence,
                inference_reasoning: extracted.reasoning.clone(),
            };

            // Generate embedding for the extracted content
            let embedding_request = EmbeddingRequest::new(&extracted.content);
            let embedding_response = embedding_provider
                .embed(embedding_request)
                .await
                .map_err(|e| AppError::Internal(format!("Embedding generation failed: {}", e)))?;

            // Find similar existing memories
            let matches = matcher
                .find_similar(
                    &event.owner_id,
                    event.scope_id.as_deref(),
                    &embedding_response.embedding,
                    embedding_provider.name(),
                )
                .await?;

            // Reconcile: decide whether to create, reinforce, or supersede
            let outcome = reconciler
                .reconcile(event, &extracted_memory, matches)
                .await?;

            // Track the outcome
            match outcome {
                ReconcileOutcome::CreateNew { memory, .. } => {
                    debug!(
                        memory_id = %memory.id,
                        event_id = %event.id,
                        "Created new memory from event"
                    );

                    // Create audit log
                    if self.audit_enabled {
                        let new_value = serde_json::to_value(&memory).ok();
                        self.audit_repo
                            .create(
                                memory.id,
                                AuditOperation::Create,
                                actor_id.clone(),
                                None,
                                new_value,
                                Some(format!("Created from event {}", event.id)),
                            )
                            .await?;
                    }

                    created_memory_ids.push(memory.id);
                }
                ReconcileOutcome::Reinforce {
                    memory_id,
                    confidence_delta,
                    ..
                } => {
                    debug!(
                        memory_id = %memory_id,
                        event_id = %event.id,
                        confidence_delta = confidence_delta,
                        "Reinforced existing memory"
                    );

                    // Create audit log
                    if self.audit_enabled {
                        self.audit_repo
                            .create(
                                memory_id,
                                AuditOperation::Update,
                                actor_id.clone(),
                                None,
                                None,
                                Some(format!(
                                    "Reinforced by event {}, confidence +{}",
                                    event.id, confidence_delta
                                )),
                            )
                            .await?;
                    }

                    reinforced_memory_ids.push(memory_id);
                }
                ReconcileOutcome::Supersede {
                    new_memory,
                    superseded_id,
                    ..
                } => {
                    debug!(
                        new_memory_id = %new_memory.id,
                        superseded_id = %superseded_id,
                        event_id = %event.id,
                        "Created superseding memory version"
                    );

                    // Create audit logs
                    if self.audit_enabled {
                        // Log for new memory
                        let new_value = serde_json::to_value(&new_memory).ok();
                        self.audit_repo
                            .create(
                                new_memory.id,
                                AuditOperation::Create,
                                actor_id.clone(),
                                None,
                                new_value,
                                Some(format!(
                                    "Superseded memory {} from event {}",
                                    superseded_id, event.id
                                )),
                            )
                            .await?;

                        // Log for superseded memory
                        self.audit_repo
                            .create(
                                superseded_id,
                                AuditOperation::Update,
                                actor_id.clone(),
                                None,
                                None,
                                Some(format!(
                                    "Superseded by memory {} from event {}",
                                    new_memory.id, event.id
                                )),
                            )
                            .await?;
                    }

                    created_memory_ids.push(new_memory.id);
                    superseded_memory_ids.push(superseded_id);
                }
            }
        }

        // Mark event as processed
        let processed_event = self
            .event_repo
            .mark_processed(event.id, &extract_result.event_summary)
            .await?;

        info!(
            event_id = %event.id,
            memories_created = created_memory_ids.len(),
            memories_reinforced = reinforced_memory_ids.len(),
            memories_superseded = superseded_memory_ids.len(),
            "Event processing completed"
        );

        Ok(ProcessEventResult {
            event: processed_event,
            memories_created: created_memory_ids.len(),
            memories_reinforced: reinforced_memory_ids.len(),
            memories_superseded: superseded_memory_ids.len(),
            created_memory_ids,
            reinforced_memory_ids,
            superseded_memory_ids,
        })
    }

    /// Create memories from an event (convenience method)
    ///
    /// Creates an event from the request and processes it.
    pub async fn create_from_event(
        &self,
        request: CreateFromEventRequest,
        actor_id: Option<String>,
    ) -> AppResult<CreateFromEventResult> {
        debug!(
            owner_id = %request.owner_id,
            scope_id = ?request.scope_id,
            content_len = request.content.len(),
            "Creating memories from event"
        );

        // Create the event
        let event_input = CreateEventInput {
            owner_id: request.owner_id,
            scope_id: request.scope_id,
            content: request.content,
            context: request.context,
            source: request.source,
            event_time: None,
        };

        let event = Event::new(event_input);

        // Store the event
        let stored_event = self.event_repo.create(&event).await?;

        // Process the event
        let result = self.process_event(&stored_event, actor_id).await?;

        let event_summary = result.event.summary.clone().unwrap_or_default();

        Ok(CreateFromEventResult {
            event: result.event,
            event_summary,
            memories_created: result.memories_created,
            memories_reinforced: result.memories_reinforced,
            memories_superseded: result.memories_superseded,
            created_memory_ids: result.created_memory_ids,
        })
    }

    /// Retrieve memories with optional query enhancement
    ///
    /// If `enhance` is true and a MemoryProcessor is configured, the query
    /// will be expanded with semantic synonyms before retrieval.
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
            confidence: Some(0.8),
            decay_score: Some(0.5),
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn test_update_request_validation_invalid_confidence() {
        let request = UpdateMemoryRequest {
            confidence: Some(1.5),
            decay_score: None,
        };
        assert!(matches!(request.validate(), Err(AppError::Validation(_))));
    }

    #[test]
    fn test_update_request_validation_negative_confidence() {
        let request = UpdateMemoryRequest {
            confidence: Some(-0.1),
            decay_score: None,
        };
        assert!(matches!(request.validate(), Err(AppError::Validation(_))));
    }

    #[test]
    fn test_update_request_validation_negative_decay_score() {
        let request = UpdateMemoryRequest {
            confidence: None,
            decay_score: Some(-0.5),
        };
        assert!(matches!(request.validate(), Err(AppError::Validation(_))));
    }

    #[test]
    fn test_update_request_validation_empty() {
        let request = UpdateMemoryRequest {
            confidence: None,
            decay_score: None,
        };
        assert!(request.validate().is_ok());
    }
}
