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
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::domain::{CreateEventInput, CreateMemoryInput, CreateMemoryValidation, Event, Memory};
use crate::embedding::{EmbeddingProvider, EmbeddingRequest};
use crate::error::{AppError, AppResult};
use crate::repository::{
    AuditOperation, AuditRepository, EventRepository, MemoryRepository, QdrantRepository,
    VectorFilter,
};

use super::{
    ConsistencyChecker, ExtractFromEventRequest, ExtractedMemory, MatchResult, MemoryProcessor,
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

/// Context for event processing containing all required providers
///
/// This struct holds all the dynamically created providers needed for
/// processing an event. The providers are created based on:
/// - User-specified LLM provider in the API request
/// - User-specified embedding provider for new memories
/// - Embedding providers determined by querying existing memories
pub struct EventProcessingContext {
    /// Memory processor for LLM-driven extraction
    pub memory_processor: Arc<MemoryProcessor>,
    /// Embedding provider for new memories (from request)
    pub embedding_provider: Arc<dyn EmbeddingProvider>,
    /// Name of the embedding provider for new memories
    pub embedding_provider_name: String,
    /// Qdrant repository for vector operations
    pub qdrant_repo: QdrantRepository,
    /// Map of embedding provider name -> provider instance (for existing memories)
    /// This is populated based on what providers are used by existing memories
    pub embedding_providers: HashMap<String, Arc<dyn EmbeddingProvider>>,
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
    /// Memory reconciler for handling match outcomes
    memory_reconciler: MemoryReconciler<C>,
    /// Memory processor for LLM-driven extraction (optional, for query enhancement)
    memory_processor: Option<Arc<MemoryProcessor>>,
    /// Retrieval engine for query enhancement
    retrieval_engine: Option<Arc<RetrievalEngine>>,
}

impl MemoryGuard<super::AlwaysConsistentChecker> {
    /// Create a new MemoryGuard service with default AlwaysConsistentChecker
    /// This is a convenience method for basic operations
    pub fn new_basic(
        memory_repo: MemoryRepository,
        event_repo: EventRepository,
        audit_repo: AuditRepository,
        audit_enabled: bool,
    ) -> Self {
        // Create a default reconciler with AlwaysConsistentChecker
        let reconciler = MemoryReconciler::with_defaults(
            memory_repo.clone(),
            event_repo.clone(),
            super::AlwaysConsistentChecker,
        );
        MemoryGuard {
            memory_repo,
            event_repo,
            audit_repo,
            audit_enabled,
            memory_reconciler: reconciler,
            memory_processor: None,
            retrieval_engine: None,
        }
    }
}

impl<C: ConsistencyChecker> MemoryGuard<C> {
    /// Create a new MemoryGuard service with a reconciler
    pub fn new(
        memory_repo: MemoryRepository,
        event_repo: EventRepository,
        audit_repo: AuditRepository,
        audit_enabled: bool,
        memory_reconciler: MemoryReconciler<C>,
    ) -> Self {
        Self {
            memory_repo,
            event_repo,
            audit_repo,
            audit_enabled,
            memory_reconciler,
            memory_processor: None,
            retrieval_engine: None,
        }
    }

    /// Set the memory processor (for query enhancement)
    pub fn set_processor(&mut self, processor: Arc<MemoryProcessor>) {
        self.memory_processor = Some(processor);
    }

    /// Set the retrieval engine
    pub fn set_retrieval_engine(&mut self, engine: Arc<RetrievalEngine>) {
        self.retrieval_engine = Some(engine);
    }

    /// Get the memory repository
    pub fn memory_repo(&self) -> &MemoryRepository {
        &self.memory_repo
    }

    /// Get the event repository
    pub fn event_repo(&self) -> &EventRepository {
        &self.event_repo
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
    ///    a. Queries existing memories grouped by embedding_provider
    ///    b. For each provider group, generates embedding and searches for matches
    ///    c. If match found in any group → reinforce/supersede
    ///    d. If no match in any group → create new memory with default embedding_provider
    ///
    /// The context parameter provides all dynamically created providers.
    ///
    /// Requirements: 2.1, 3.1, 3.2, 3.3, 3.4
    pub async fn process_event_with_context(
        &self,
        event: &Event,
        context: EventProcessingContext,
        actor_id: Option<String>,
    ) -> AppResult<ProcessEventResult> {
        debug!(
            event_id = %event.id,
            owner_id = %event.owner_id,
            scope_id = ?event.scope_id,
            "Processing event with context"
        );

        // Extract memories from the event using LLM
        let extract_request = ExtractFromEventRequest {
            content: event.content.clone(),
            context: event.context.clone(),
        };

        let extract_result = context
            .memory_processor
            .extract_from_event(extract_request)
            .await
            .map_err(|e| AppError::Internal(format!("Memory extraction failed: {}", e)))?;

        debug!(
            extracted_memory = extract_result.event_summary,
            "Extracted memories from event"
        );
        info!(
            extracted_memory_count = extract_result.extracted_memories.len(),
            "Extracted memories from event successfully"
        );

        // Query existing memories grouped by embedding_provider
        let memories_by_provider = self
            .memory_repo
            .find_by_owner_scope_grouped_by_provider(
                &event.owner_id,
                event.scope_id.as_deref(),
                true, // include_global
            )
            .await?;

        debug!(
            provider_count = memories_by_provider.len(),
            "Found existing memories grouped by provider"
        );

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

            // Try to find matches across all provider groups
            let mut all_matches: Vec<MatchResult> = Vec::new();

            for (provider_name, memories) in &memories_by_provider {
                // Get the embedding provider for this group
                let embedding_provider = match context.embedding_providers.get(provider_name) {
                    Some(provider) => provider.clone(),
                    None => {
                        warn!(
                            provider_name = %provider_name,
                            "Embedding provider not found in context, skipping group"
                        );
                        continue;
                    }
                };

                // Generate embedding for the extracted content using this provider
                let embedding_request = EmbeddingRequest::new(&extracted.content);
                let embedding_response = match embedding_provider.embed(embedding_request).await {
                    Ok(response) => response,
                    Err(e) => {
                        warn!(
                            provider_name = %provider_name,
                            error = %e,
                            "Failed to generate embedding, skipping group"
                        );
                        continue;
                    }
                };

                // Search for similar vectors in Qdrant for this provider's collection
                let filter = VectorFilter {
                    scope_id: event.scope_id.clone(),
                    statuses: Some(vec!["active".to_string()]),
                    ..Default::default()
                };

                let vector_results = context
                    .qdrant_repo
                    .search(
                        provider_name,
                        embedding_response.embedding.clone(),
                        10, // max candidates
                        Some(filter),
                    )
                    .await?;

                // Build score map from vector search results
                let score_map: std::collections::HashMap<Uuid, f32> = vector_results
                    .iter()
                    .map(|r| (r.memory_id, r.score))
                    .collect();

                // Filter memories that match our criteria
                for memory in memories {
                    if let Some(&score) = score_map.get(&memory.id) {
                        // Check similarity threshold (0.85)
                        if score >= 0.85
                            && memory.is_current_version
                            && memory.status == crate::domain::Status::Active
                        {
                            all_matches.push(MatchResult {
                                memory: memory.clone(),
                                similarity_score: score,
                            });
                        }
                    }
                }
            }

            // Sort all matches by similarity score (highest first)
            all_matches.sort_by(|a, b| {
                b.similarity_score
                    .partial_cmp(&a.similarity_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            // Reconcile: decide whether to create, reinforce, or supersede
            let outcome = self
                .memory_reconciler
                .reconcile(event, &extracted_memory, all_matches)
                .await?;

            // Track the outcome
            match outcome {
                ReconcileOutcome::CreateNew { mut memory, .. } => {
                    // For new memories, we need to generate embedding and store in Qdrant
                    // Set the embedding provider name
                    memory = self
                        .memory_repo
                        .get_by_id(memory.id)
                        .await
                        .unwrap_or(memory);

                    // Generate embedding with the user-specified provider
                    let embedding_request = EmbeddingRequest::new(&memory.content);
                    let embedding_response = context
                        .embedding_provider
                        .embed(embedding_request)
                        .await
                        .map_err(|e| {
                            AppError::Internal(format!("Embedding generation failed: {}", e))
                        })?;

                    // Store in Qdrant
                    let payload = crate::repository::VectorPayload {
                        memory_id: memory.id,
                        scope_id: memory.scope_id.clone(),
                        category: memory.category.clone(),
                        status: memory.status.to_string(),
                        is_global: memory.is_global,
                    };

                    context
                        .qdrant_repo
                        .upsert_vector(
                            &context.embedding_provider_name,
                            memory.id,
                            embedding_response.embedding,
                            payload,
                        )
                        .await?;

                    // Update memory with embedding status and provider in database
                    memory = self
                        .memory_repo
                        .update_embedding_status_and_provider(
                            memory.id,
                            crate::domain::EmbeddingStatus::Completed,
                            Some(&context.embedding_provider_name),
                        )
                        .await?;

                    debug!(
                        memory_id = %memory.id,
                        event_id = %event.id,
                        provider = %context.embedding_provider_name,
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
                    // For superseding memories, generate embedding with the same provider as the old memory
                    // or use default if not available
                    let old_memory = self.memory_repo.get_by_id(superseded_id).await?;
                    let provider_name = old_memory
                        .embedding_provider
                        .as_ref()
                        .unwrap_or(&context.embedding_provider_name);

                    let embedding_provider = context
                        .embedding_providers
                        .get(provider_name)
                        .cloned()
                        .unwrap_or_else(|| context.embedding_provider.clone());

                    let embedding_request = EmbeddingRequest::new(&new_memory.content);
                    let embedding_response = embedding_provider
                        .embed(embedding_request)
                        .await
                        .map_err(|e| {
                            AppError::Internal(format!("Embedding generation failed: {}", e))
                        })?;

                    // Store in Qdrant
                    let payload = crate::repository::VectorPayload {
                        memory_id: new_memory.id,
                        scope_id: new_memory.scope_id.clone(),
                        category: new_memory.category.clone(),
                        status: new_memory.status.to_string(),
                        is_global: new_memory.is_global,
                    };

                    context
                        .qdrant_repo
                        .upsert_vector(
                            provider_name,
                            new_memory.id,
                            embedding_response.embedding,
                            payload,
                        )
                        .await?;

                    // Update new memory with embedding status and provider in database
                    let _updated_memory = self
                        .memory_repo
                        .update_embedding_status_and_provider(
                            new_memory.id,
                            crate::domain::EmbeddingStatus::Completed,
                            Some(provider_name),
                        )
                        .await?;

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

    /// Process an event and create/reinforce/supersede memories (legacy method)
    ///
    /// This method is kept for backward compatibility but requires the MemoryGuard
    /// to have a configured memory_processor. For new code, use process_event_with_context.
    ///
    /// Requirements: 2.1, 3.1, 3.2, 3.3, 3.4
    pub async fn process_event(
        &self,
        _event: &Event,
        _memory_processor: Option<Arc<MemoryProcessor>>,
        _actor_id: Option<String>,
    ) -> AppResult<ProcessEventResult> {
        // This legacy method cannot work without a full context
        // Return an error indicating the new method should be used
        Err(AppError::Internal(
            "process_event requires EventProcessingContext. Use process_event_with_context instead, \
             or ensure the API layer builds the context properly.".to_string()
        ))
    }

    /// Create memories from an event with full context
    ///
    /// Creates an event from the request and processes it using the provided context.
    pub async fn create_from_event_with_context(
        &self,
        request: CreateFromEventRequest,
        context: EventProcessingContext,
        actor_id: Option<String>,
    ) -> AppResult<CreateFromEventResult> {
        debug!(
            owner_id = %request.owner_id,
            scope_id = ?request.scope_id,
            content_len = request.content.len(),
            "Creating memories from event with context"
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

        // Process the event with the provided context
        let result = self
            .process_event_with_context(&stored_event, context, actor_id)
            .await?;

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

    /// Create memories from an event (legacy method)
    ///
    /// Creates an event from the request and processes it.
    /// This method is kept for backward compatibility but requires proper context setup.
    pub async fn create_from_event(
        &self,
        _request: CreateFromEventRequest,
        _memory_processor: Option<Arc<MemoryProcessor>>,
        _actor_id: Option<String>,
    ) -> AppResult<CreateFromEventResult> {
        // This legacy method cannot work without a full context
        Err(AppError::Internal(
            "create_from_event requires EventProcessingContext. Use create_from_event_with_context instead.".to_string()
        ))
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
