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
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::domain::{
    CreateEventInput, CreateMemoryInput, CreateMemoryValidation, EmbeddingStatus, Event,
    EventSource, Memory, SystemProfile,
};
use crate::embedding::{EmbeddingProvider, EmbeddingRequest};
use crate::error::{AppError, AppResult};
use crate::repository::{
    AuditOperation, AuditRepository, EventRepository, MemoryRepository, QdrantRepository,
};

use super::{
    ConsistencyChecker, ExtractFromEventRequest, MemoryMatcher, MemoryProcessor, MemoryReconciler,
    ReconcileOutcome, RetrievalEngine, RetrieveRequest, RetrieveResponse,
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
/// This struct holds the globally configured providers and repositories needed
/// by the event-driven memory processing pipeline.
pub struct EventProcessingContext {
    /// Active business system profile
    pub profile: SystemProfile,
    /// Memory processor for LLM-driven extraction
    pub memory_processor: Arc<MemoryProcessor>,
    /// Event handler for structuring events into six-element format
    pub event_handler: Option<Arc<super::EventHandler>>,
    /// LLM provider for consistency checking
    pub llm_provider: Arc<dyn crate::llm::LlmProvider>,
    /// Embedding provider for new memories
    pub embedding_provider: Arc<dyn EmbeddingProvider>,
    /// Name of the embedding provider for new memories
    pub embedding_provider_name: String,
    /// Qdrant repository for vector operations
    pub qdrant_repo: QdrantRepository,
    /// Matcher for finding existing memories during event reconciliation
    pub memory_matcher: Arc<MemoryMatcher>,
    /// Context memories for better relevance judgment and memory updates
    /// These are same-scope memories + global memories fetched before processing
    pub context_memories: Vec<crate::domain::Memory>,
}

impl EventProcessingContext {
    pub fn new(
        profile: SystemProfile,
        memory_processor: Arc<MemoryProcessor>,
        event_handler: Arc<super::EventHandler>,
        llm_provider: Arc<dyn crate::llm::LlmProvider>,
        embedding_provider: Arc<dyn EmbeddingProvider>,
        embedding_provider_name: String,
        qdrant_repo: QdrantRepository,
        memory_matcher: Arc<MemoryMatcher>,
        context_memories: Vec<crate::domain::Memory>,
    ) -> Self {
        Self {
            profile,
            memory_processor,
            event_handler: Some(event_handler),
            llm_provider,
            embedding_provider,
            embedding_provider_name,
            qdrant_repo,
            memory_matcher,
            context_memories,
        }
    }
}

/// Request for creating memories from an event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFromEventRequest {
    /// System profile ID - which business system this event belongs to
    pub profile_id: Uuid,
    /// Owner ID - the unique identifier of the memory owner
    pub owner_id: String,
    /// Event content (any form: click description, conversation history, operation log, etc.)
    pub content: String,
    /// Optional context to help LLM better understand the event
    #[serde(default)]
    pub context: Option<String>,
    /// Scope identifier for created memories (optional, null = global context)
    pub scope_id: Option<String>,
    /// Event source. Recommended values: conversation, user_action, system_event, manual, api.
    pub source: Option<EventSource>,
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
    /// IDs of reinforced memories
    pub reinforced_memory_ids: Vec<Uuid>,
    /// IDs of superseded memories (old versions)
    pub superseded_memory_ids: Vec<Uuid>,
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
        let reconciler =
            MemoryReconciler::with_defaults(memory_repo.clone(), super::AlwaysConsistentChecker);
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
                    created.profile_id,
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
                    updated.profile_id,
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
                    deleted.profile_id,
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
    /// 1. Structures the event into six-element format (if EventHandler is available)
    /// 2. Extracts potential memories using LLM
    /// 3. For each extracted memory:
    ///    a. Queries existing memories grouped by embedding_provider
    ///    b. For each provider group, generates embedding and searches for matches
    ///    c. If match found in any group → reinforce/supersede
    ///    d. If no match in any group → create new memory with default embedding_provider
    ///
    /// The context parameter provides globally configured providers.
    ///
    /// Requirements: 2.1, 3.1, 3.2, 3.3, 3.4, 6.9-6.10, 8.1-8.4
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

        // Step 1: Analyze the event into a profile-aware payload (if EventHandler is available)
        let event_analysis = if let Some(ref event_handler) = context.event_handler {
            match event_handler.structure_event(event).await {
                Ok(analysis) => {
                    debug!(
                        event_id = %event.id,
                        has_analysis_payload = analysis.payload.is_object(),
                        analysis_schema_version = ?analysis.schema_version,
                        "Event analyzed successfully"
                    );
                    Some(analysis)
                }
                Err(e) => {
                    warn!(
                        event_id = %event.id,
                        error = %e,
                        "Failed to analyze event, continuing with raw content"
                    );
                    None
                }
            }
        } else {
            debug!(
                event_id = %event.id,
                "No EventHandler available, using raw event content"
            );
            None
        };

        // Step 2: Build a schema-aware extraction request
        let extract_request = if let Some(ref analysis) = event_analysis {
            let enhanced_context =
                Self::build_enhanced_context(event, &analysis.payload, &context.context_memories);
            ExtractFromEventRequest::with_context(
                event.content.clone(),
                enhanced_context,
                context.profile.extraction_prompt.clone(),
                context.profile.metadata_schema.clone(),
                context.profile.schema_generation_prompt.clone(),
                analysis
                    .schema_version
                    .unwrap_or(context.profile.schema_version),
            )
        } else {
            let mut request = ExtractFromEventRequest::new(
                event.content.clone(),
                context.profile.extraction_prompt.clone(),
                context.profile.metadata_schema.clone(),
                context.profile.schema_generation_prompt.clone(),
                context.profile.schema_version,
            );
            request.context = Some(Self::build_raw_extraction_context(
                event,
                &context.context_memories,
            ));
            request
        };

        // Step 3: Extract memories from the event using LLM
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

        let mut working_context_memories = context.context_memories.clone();

        // Track results
        let mut created_memory_ids = Vec::new();
        let mut reinforced_memory_ids = Vec::new();
        let mut superseded_memory_ids = Vec::new();

        // Process each extracted memory
        for extracted in &extract_result.extracted_memories {
            let embedding_request = EmbeddingRequest::new(&extracted.content);
            let embedding_response = context
                .embedding_provider
                .embed(embedding_request)
                .await
                .map_err(|e| AppError::Internal(format!("Embedding generation failed: {}", e)))?;

            let vector_matches = context
                .memory_matcher
                .find_similar(
                    event.profile_id,
                    &event.owner_id,
                    event.scope_id.as_deref(),
                    &embedding_response.embedding,
                    &context.embedding_provider_name,
                )
                .await?;
            let matches = Self::merge_schema_aware_matches(
                vector_matches,
                extracted,
                &working_context_memories,
                &context.profile.metadata_schema,
            );

            debug!(
                match_count = matches.len(),
                "Found memory matches for extracted memory"
            );

            // Reconcile: decide whether to create, reinforce, or supersede
            // Use LLM-based consistency checking with the context's LLM provider
            let outcome = self
                .memory_reconciler
                .reconcile_with_llm(event, extracted, matches, context.llm_provider.clone())
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

                    memory = self.upsert_event_memory_vector(&context, memory).await?;

                    debug!(
                        memory_id = %memory.id,
                        event_id = %event.id,
                        provider = %context.embedding_provider_name,
                        "Created new memory from event"
                    );

                    // Create audit log
                    let new_value = serde_json::to_value(&memory).ok();
                    self.create_event_audit_log(
                        memory.profile_id,
                        memory.id,
                        AuditOperation::Create,
                        actor_id.clone(),
                        None,
                        new_value,
                        Some(format!("Created from event {}", event.id)),
                    )
                    .await;

                    Self::upsert_working_context_memory(&mut working_context_memories, &memory);
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
                    self.create_event_audit_log(
                        event.profile_id,
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
                    .await;

                    reinforced_memory_ids.push(memory_id);
                }
                ReconcileOutcome::Supersede {
                    new_memory,
                    superseded_id,
                    ..
                } => {
                    let _updated_memory = self
                        .upsert_event_memory_vector(&context, new_memory.clone())
                        .await?;

                    if let Err(e) = context
                        .qdrant_repo
                        .delete_vector(&context.embedding_provider_name, superseded_id)
                        .await
                    {
                        warn!(
                            memory_id = %superseded_id,
                            provider = %context.embedding_provider_name,
                            error = %e,
                            "Failed to delete superseded memory vector"
                        );
                    }

                    debug!(
                        new_memory_id = %new_memory.id,
                        superseded_id = %superseded_id,
                        event_id = %event.id,
                        "Created superseding memory version"
                    );

                    // Create audit logs
                    let new_value = serde_json::to_value(&new_memory).ok();
                    self.create_event_audit_log(
                        new_memory.profile_id,
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
                    .await;

                    self.create_event_audit_log(
                        event.profile_id,
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
                    .await;

                    Self::replace_superseded_context_memory(
                        &mut working_context_memories,
                        superseded_id,
                        &new_memory,
                    );
                    created_memory_ids.push(new_memory.id);
                    superseded_memory_ids.push(superseded_id);
                }
            }
        }

        // Mark event as processed
        let processed_event = self
            .event_repo
            .mark_processed(
                event.id,
                &extract_result.event_summary,
                event_analysis.as_ref().map(|analysis| &analysis.payload),
                event_analysis
                    .as_ref()
                    .and_then(|analysis| analysis.schema_version),
            )
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

    /// Process an event and create/reinforce/supersede memories.
    ///
    /// Use `process_event_with_context` so provider/repository dependencies are explicit.
    ///
    /// Requirements: 2.1, 3.1, 3.2, 3.3, 3.4
    pub async fn process_event(
        &self,
        _event: &Event,
        _memory_processor: Option<Arc<MemoryProcessor>>,
        _actor_id: Option<String>,
    ) -> AppResult<ProcessEventResult> {
        // This method cannot work without a full context.
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
            profile_id: request.profile_id,
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
            reinforced_memory_ids: result.reinforced_memory_ids,
            superseded_memory_ids: result.superseded_memory_ids,
        })
    }

    /// Create memories from an event.
    ///
    /// Use `create_from_event_with_context` so provider/repository dependencies are explicit.
    pub async fn create_from_event(
        &self,
        _request: CreateFromEventRequest,
        _memory_processor: Option<Arc<MemoryProcessor>>,
        _actor_id: Option<String>,
    ) -> AppResult<CreateFromEventResult> {
        // This method cannot work without a full context.
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

    /// Build enhanced context from event analysis payload for memory extraction
    ///
    /// Combines the original event context with analysis payload
    /// to provide richer context for memory extraction.
    fn build_enhanced_context(
        event: &Event,
        analysis_payload: &Value,
        context_memories: &[Memory],
    ) -> String {
        let mut context_parts = Vec::new();

        // Add original context if present
        if let Some(ref ctx) = event.context {
            context_parts.push(format!("原始上下文: {}", ctx));
        }

        context_parts.push("事件分析结果(JSON):".to_string());
        context_parts.push(analysis_payload.to_string());
        Self::push_context_memories(&mut context_parts, context_memories);

        context_parts.join("\n")
    }

    fn build_raw_extraction_context(event: &Event, context_memories: &[Memory]) -> String {
        let mut context_parts = Vec::new();

        if let Some(ref ctx) = event.context {
            context_parts.push(format!("原始上下文: {}", ctx));
        }

        Self::push_context_memories(&mut context_parts, context_memories);
        context_parts.join("\n")
    }

    fn push_context_memories(context_parts: &mut Vec<String>, context_memories: &[Memory]) {
        if context_memories.is_empty() {
            return;
        }

        context_parts
            .push("用户历史记忆（用于判断是否需要强化、放宽、推翻或更新旧记忆）:".to_string());
        for (index, memory) in context_memories.iter().take(30).enumerate() {
            context_parts.push(format!(
                "{}. id={} version={} current={} metadata={} content={}",
                index + 1,
                memory.id,
                memory.version_number,
                memory.is_current_version,
                memory.metadata,
                memory.content
            ));
        }
    }

    fn merge_schema_aware_matches(
        vector_matches: Vec<super::MatchResult>,
        extracted: &super::ExtractedMemory,
        context_memories: &[Memory],
        metadata_schema: &Value,
    ) -> Vec<super::MatchResult> {
        let schema_match_fields = Self::stable_schema_match_fields(metadata_schema);
        if schema_match_fields.is_empty() {
            return vector_matches;
        }

        let mut matches_by_id: HashMap<Uuid, super::MatchResult> = HashMap::new();
        for item in vector_matches {
            matches_by_id.insert(item.memory.id, item);
        }

        for memory in context_memories {
            if !memory.is_current_version || memory.status != crate::domain::Status::Active {
                continue;
            }

            let Some(schema_score) = Self::schema_metadata_match_score(
                &extracted.metadata,
                &memory.metadata,
                &schema_match_fields,
            ) else {
                continue;
            };

            matches_by_id
                .entry(memory.id)
                .and_modify(|item| {
                    item.similarity_score = item.similarity_score.max(schema_score);
                })
                .or_insert_with(|| super::MatchResult {
                    memory: memory.clone(),
                    similarity_score: schema_score,
                });
        }

        let mut matches: Vec<super::MatchResult> = matches_by_id.into_values().collect();
        matches.sort_by(|a, b| {
            b.similarity_score
                .partial_cmp(&a.similarity_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        matches.truncate(20);
        matches
    }

    fn upsert_working_context_memory(context_memories: &mut Vec<Memory>, memory: &Memory) {
        if let Some(existing) = context_memories.iter_mut().find(|item| item.id == memory.id) {
            *existing = memory.clone();
        } else {
            context_memories.push(memory.clone());
        }
    }

    fn replace_superseded_context_memory(
        context_memories: &mut Vec<Memory>,
        superseded_id: Uuid,
        new_memory: &Memory,
    ) {
        if let Some(existing) = context_memories.iter_mut().find(|item| item.id == superseded_id) {
            existing.mark_superseded(new_memory.id);
        }

        Self::upsert_working_context_memory(context_memories, new_memory);
    }

    fn stable_schema_match_fields(metadata_schema: &Value) -> Vec<String> {
        let mut fields = BTreeSet::new();
        let volatile_fields = [
            "sentiment",
            "polarity",
            "status",
            "state",
            "decision_stage",
            "constraint_flag",
        ];

        if let Some(entity_types) = metadata_schema
            .get("entity_types")
            .and_then(|value| value.as_object())
        {
            for entity in entity_types.values() {
                for key in [
                    "lineage_group_fields",
                    "candidate_match_fields",
                    "conflict_fields",
                ] {
                    if let Some(items) = entity.get(key).and_then(|value| value.as_array()) {
                        for item in items {
                            if let Some(field) = item.as_str() {
                                if !volatile_fields.contains(&field) {
                                    fields.insert(field.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(filterable_fields) = metadata_schema
            .get("filterable_fields")
            .and_then(|value| value.as_array())
        {
            for item in filterable_fields {
                if let Some(field) = item.as_str() {
                    if matches!(
                        field,
                        "subject"
                            | "topic"
                            | "object"
                            | "entity_type"
                            | "preference_category"
                            | "category"
                            | "memory_type"
                    ) {
                        fields.insert(field.to_string());
                    }
                }
            }
        }

        fields.into_iter().collect()
    }

    fn schema_metadata_match_score(
        new_metadata: &Value,
        existing_metadata: &Value,
        fields: &[String],
    ) -> Option<f32> {
        let mut matched_fields = 0usize;
        let mut matched_non_type_fields = 0usize;

        for field in fields {
            let Some(new_value) = Self::lookup_metadata_value(new_metadata, field) else {
                continue;
            };
            let Some(existing_value) = Self::lookup_metadata_value(existing_metadata, field) else {
                continue;
            };
            if !Self::metadata_values_overlap(new_value, existing_value) {
                continue;
            }

            matched_fields += 1;
            if field != "memory_type" && field != "entity_type" {
                matched_non_type_fields += 1;
            }
        }

        if matched_non_type_fields == 0 {
            return None;
        }

        Some((0.88 + matched_fields as f32 * 0.03).min(0.97))
    }

    fn lookup_metadata_value<'a>(metadata: &'a Value, field: &str) -> Option<&'a Value> {
        if let Some(value) = metadata.as_object()?.get(field) {
            return Some(value);
        }

        let mut current = metadata;
        for part in field.split('.') {
            current = current.get(part)?;
        }
        Some(current)
    }

    fn metadata_values_overlap(left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Array(left_items), Value::Array(right_items)) => left_items
                .iter()
                .any(|left| right_items.iter().any(|right| left == right)),
            (Value::Array(items), value) | (value, Value::Array(items)) => {
                items.iter().any(|item| item == value)
            }
            _ => left == right,
        }
    }

    async fn upsert_event_memory_vector(
        &self,
        context: &EventProcessingContext,
        mut memory: Memory,
    ) -> AppResult<Memory> {
        let embedding_request = EmbeddingRequest::new(&memory.content);

        let embedding_response = match context.embedding_provider.embed(embedding_request).await {
            Ok(response) => response,
            Err(e) => {
                warn!(
                    memory_id = %memory.id,
                    error = %e,
                    "Embedding generation failed after memory DB write"
                );
                return self
                    .memory_repo
                    .update_embedding_status_and_provider(memory.id, EmbeddingStatus::Failed, None)
                    .await;
            }
        };

        let payload = crate::repository::VectorPayload {
            profile_id: memory.profile_id,
            owner_id: memory.owner_id.clone(),
            memory_id: memory.id,
            scope_id: memory.scope_id.clone(),
            status: memory.status.to_string(),
            is_global: memory.is_global,
        };

        if let Err(e) = context
            .qdrant_repo
            .upsert_vector(
                &context.embedding_provider_name,
                memory.id,
                embedding_response.embedding,
                payload,
            )
            .await
        {
            warn!(
                memory_id = %memory.id,
                provider = %context.embedding_provider_name,
                error = %e,
                "Qdrant upsert failed after memory DB write"
            );
            return self
                .memory_repo
                .update_embedding_status_and_provider(memory.id, EmbeddingStatus::Failed, None)
                .await;
        }

        memory = self
            .memory_repo
            .update_embedding_status_and_provider(
                memory.id,
                EmbeddingStatus::Completed,
                Some(&context.embedding_provider_name),
            )
            .await?;

        Ok(memory)
    }

    async fn create_event_audit_log(
        &self,
        profile_id: Uuid,
        memory_id: Uuid,
        operation: AuditOperation,
        actor_id: Option<String>,
        old_value: Option<serde_json::Value>,
        new_value: Option<serde_json::Value>,
        reason: Option<String>,
    ) {
        if !self.audit_enabled {
            return;
        }

        if let Err(e) = self
            .audit_repo
            .create(
                profile_id, memory_id, operation, actor_id, old_value, new_value, reason,
            )
            .await
        {
            warn!(
                profile_id = %profile_id,
                memory_id = %memory_id,
                operation = %operation,
                error = %e,
                "Failed to write event memory audit log"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::AlwaysConsistentChecker;
    use serde_json::json;

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

    #[test]
    fn test_schema_metadata_match_scores_domain_group_fields() {
        let metadata_schema = json!({
            "entity_types": {
                "housing_preference": {
                    "candidate_match_fields": ["preference_category", "sentiment"],
                    "lineage_group_fields": ["preference_category", "decision_stage"],
                    "conflict_fields": ["preference_category", "sentiment"]
                }
            },
            "filterable_fields": ["memory_type", "preference_category", "sentiment"]
        });
        let fields =
            MemoryGuard::<AlwaysConsistentChecker>::stable_schema_match_fields(&metadata_schema);

        let score = MemoryGuard::<AlwaysConsistentChecker>::schema_metadata_match_score(
            &json!({
                "memory_type": "housing_preference",
                "preference_category": "budget",
                "sentiment": "positive"
            }),
            &json!({
                "memory_type": "housing_preference",
                "preference_category": "budget",
                "sentiment": "negative"
            }),
            &fields,
        );

        assert!(score.is_some());
    }

    #[test]
    fn test_schema_metadata_match_ignores_type_only_match() {
        let fields = vec!["memory_type".to_string(), "preference_category".to_string()];

        let score = MemoryGuard::<AlwaysConsistentChecker>::schema_metadata_match_score(
            &json!({
                "memory_type": "housing_preference",
                "preference_category": "budget"
            }),
            &json!({
                "memory_type": "housing_preference",
                "preference_category": "style"
            }),
            &fields,
        );

        assert!(score.is_none());
    }

    #[test]
    fn test_replace_superseded_context_memory_marks_old_and_adds_new() {
        let profile_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let old_memory = Memory::new_from_event(crate::domain::CreateMemoryFromEventInput {
            profile_id,
            owner_id: "user-1".to_string(),
            scope_id: Some("scope-1".to_string()),
            content: "用户觉得300万的房子有点贵".to_string(),
            metadata: json!({
                "memory_type": "housing_preference",
                "preference_category": "budget"
            }),
            schema_version: 1,
            importance: 0.8,
            confidence: 0.9,
            source_event_id: event_id,
            embedding_provider: None,
            llm_provider: None,
            inference_type: crate::domain::InferenceType::Preference,
            inference_confidence: 0.9,
            inference_reasoning: "预算偏好".to_string(),
        });

        let new_memory = Memory::new_superseding(crate::domain::CreateSupersedingMemoryInput {
            old_memory_id: old_memory.id,
            root_memory_id: old_memory.root_memory_id.unwrap_or(old_memory.id),
            version_number: old_memory.version_number + 1,
            profile_id,
            owner_id: "user-1".to_string(),
            scope_id: Some("scope-1".to_string()),
            content: "用户税后预算能力提高，愿意为了居住体验考虑更高预算".to_string(),
            metadata: json!({
                "memory_type": "housing_preference",
                "preference_category": "budget"
            }),
            schema_version: 1,
            importance: 0.9,
            confidence: 0.95,
            is_global: false,
            source_event_id: event_id,
            embedding_provider: None,
            llm_provider: None,
            inference_type: crate::domain::InferenceType::Preference,
            inference_confidence: 0.95,
            inference_reasoning: "预算变化".to_string(),
        });

        let mut context_memories = vec![old_memory.clone()];
        MemoryGuard::<AlwaysConsistentChecker>::replace_superseded_context_memory(
            &mut context_memories,
            old_memory.id,
            &new_memory,
        );

        let old_entry = context_memories
            .iter()
            .find(|memory| memory.id == old_memory.id)
            .expect("old memory should remain for lineage");
        assert!(!old_entry.is_current_version);
        assert_eq!(old_entry.status, crate::domain::Status::Superseded);
        assert_eq!(old_entry.superseded_by, Some(new_memory.id));

        let new_entry = context_memories
            .iter()
            .find(|memory| memory.id == new_memory.id)
            .expect("new memory should be added");
        assert!(new_entry.is_current_version);
        assert_eq!(new_entry.status, crate::domain::Status::Active);
        assert_eq!(new_entry.supersedes, Some(old_memory.id));
    }
}
