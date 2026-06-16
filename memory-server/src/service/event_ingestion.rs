//! Event ingestion orchestration.
//!
//! Keeps HTTP handlers thin by owning the full event processing pipeline:
//! relevance check, context loading, event structuring, extraction, matching,
//! reconciliation, embedding, vector upsert, and processed-state update.

use std::sync::Arc;

use tracing::{debug, info};
use uuid::Uuid;

use super::{
    AlwaysConsistentChecker, CreateFromEventRequest, EventHandler, EventProcessingContext,
    MemoryGuard, MemoryMatcher, MemoryProcessor, ProfileService, RelevanceCheckResult,
};
use crate::domain::{CreateEventInput, CreateEventValidation, Event, Memory};
use crate::embedding::EmbeddingProvider;
use crate::error::{AppError, AppResult};
use crate::llm::LlmProvider;
use crate::repository::{QdrantRepository, StructuredEventRepository};

/// Result returned by the event ingestion pipeline.
#[derive(Debug, Clone)]
pub struct EventIngestionResult {
    pub event: Event,
    pub memories_created: usize,
    pub memories_reinforced: usize,
    pub memories_superseded: usize,
    pub created_memory_ids: Vec<Uuid>,
    pub reinforced_memory_ids: Vec<Uuid>,
    pub superseded_memory_ids: Vec<Uuid>,
    pub skipped: bool,
    pub skip_reason: Option<String>,
    pub relevance_score: Option<f32>,
}

impl EventIngestionResult {
    fn skipped(event: Event) -> Self {
        Self {
            skip_reason: event.skip_reason.clone(),
            relevance_score: event.relevance_score,
            event,
            memories_created: 0,
            memories_reinforced: 0,
            memories_superseded: 0,
            created_memory_ids: vec![],
            reinforced_memory_ids: vec![],
            superseded_memory_ids: vec![],
            skipped: true,
        }
    }
}

struct SkipDecision {
    reason: String,
    relevance_score: f32,
}

/// Application service for event-driven memory ingestion.
#[derive(Clone)]
pub struct EventIngestionService {
    memory_guard: Arc<MemoryGuard<AlwaysConsistentChecker>>,
    profile_service: ProfileService,
    llm_provider: Arc<dyn LlmProvider>,
    embedding_provider: Arc<dyn EmbeddingProvider>,
    embedding_provider_name: String,
    qdrant_repo: QdrantRepository,
    memory_matcher: Arc<MemoryMatcher>,
    structured_event_repo: StructuredEventRepository,
}

impl EventIngestionService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        memory_guard: Arc<MemoryGuard<AlwaysConsistentChecker>>,
        profile_service: ProfileService,
        llm_provider: Arc<dyn LlmProvider>,
        embedding_provider: Arc<dyn EmbeddingProvider>,
        embedding_provider_name: String,
        qdrant_repo: QdrantRepository,
        memory_matcher: Arc<MemoryMatcher>,
        structured_event_repo: StructuredEventRepository,
    ) -> Self {
        Self {
            memory_guard,
            profile_service,
            llm_provider,
            embedding_provider,
            embedding_provider_name,
            qdrant_repo,
            memory_matcher,
            structured_event_repo,
        }
    }

    /// Create an unprocessed event record for asynchronous processing.
    pub async fn create_pending_event(&self, request: CreateFromEventRequest) -> AppResult<Event> {
        let input = Self::event_input_from_request(request);
        CreateEventValidation::validate(&input)?;

        let event = Event::new(input);
        self.memory_guard.event_repo().create(&event).await
    }

    /// Create and process an event synchronously.
    pub async fn ingest_event_sync(
        &self,
        request: CreateFromEventRequest,
    ) -> AppResult<EventIngestionResult> {
        debug!(
            profile_id = %request.profile_id,
            owner_id = %request.owner_id,
            scope_id = ?request.scope_id,
            "Ingesting event synchronously"
        );

        let event = self.create_pending_event(request).await?;
        self.process_existing_event(event.profile_id, event.id)
            .await
    }

    /// Process an already-created pending event. Used by async ingestion workers.
    pub async fn process_existing_event(
        &self,
        profile_id: Uuid,
        event_id: Uuid,
    ) -> AppResult<EventIngestionResult> {
        let event = self.memory_guard.event_repo().get_by_id(event_id).await?;
        if event.profile_id != profile_id {
            return Err(AppError::Validation(
                "event does not belong to the requested system profile".to_string(),
            ));
        }

        if event.is_terminal() {
            return Err(AppError::Validation(format!(
                "event has already reached terminal status: {}",
                event.processing_status
            )));
        }

        let result = self.process_loaded_event(event).await;
        if let Err(error) = &result {
            let _ = self
                .memory_guard
                .event_repo()
                .mark_failed(event_id, &error.to_string())
                .await;
        }

        result
    }

    /// Retry a failed event by resetting it to pending and processing it again.
    pub async fn retry_event(
        &self,
        profile_id: Uuid,
        event_id: Uuid,
    ) -> AppResult<EventIngestionResult> {
        let event = self
            .memory_guard
            .event_repo()
            .reset_for_retry(profile_id, event_id)
            .await?;

        let result = self.process_loaded_event(event).await;
        if let Err(error) = &result {
            let _ = self
                .memory_guard
                .event_repo()
                .mark_failed(event_id, &error.to_string())
                .await;
        }

        result
    }

    async fn process_loaded_event(&self, event: Event) -> AppResult<EventIngestionResult> {
        let event_handler = self.event_handler();
        let context_memories = self
            .load_context_memories(event.profile_id, &event.owner_id, event.scope_id.as_deref())
            .await;

        if let Some(skip) = self
            .skip_reason_if_irrelevant(
                &event_handler,
                event.profile_id,
                &event.content,
                &context_memories,
            )
            .await?
        {
            let processed_event = self
                .memory_guard
                .event_repo()
                .mark_skipped(event.id, &skip.reason, Some(skip.relevance_score))
                .await?;
            return Ok(EventIngestionResult::skipped(processed_event));
        }

        let context = self.processing_context(event_handler, context_memories);
        let result = self
            .memory_guard
            .process_event_with_context(&event, context, None)
            .await?;

        Ok(EventIngestionResult {
            event: result.event,
            memories_created: result.memories_created,
            memories_reinforced: result.memories_reinforced,
            memories_superseded: result.memories_superseded,
            created_memory_ids: result.created_memory_ids,
            reinforced_memory_ids: result.reinforced_memory_ids,
            superseded_memory_ids: result.superseded_memory_ids,
            skipped: false,
            skip_reason: None,
            relevance_score: None,
        })
    }

    fn event_handler(&self) -> EventHandler {
        EventHandler::with_profile_service(self.llm_provider.clone(), self.profile_service.clone())
    }

    async fn load_context_memories(
        &self,
        profile_id: Uuid,
        owner_id: &str,
        scope_id: Option<&str>,
    ) -> Vec<Memory> {
        self.memory_guard
            .memory_repo()
            .find_context_memories(profile_id, owner_id, scope_id, 20, 10)
            .await
            .unwrap_or_default()
    }

    async fn skip_reason_if_irrelevant(
        &self,
        event_handler: &EventHandler,
        profile_id: Uuid,
        content: &str,
        context_memories: &[Memory],
    ) -> AppResult<Option<SkipDecision>> {
        let relevance = event_handler
            .check_relevance_with_profile(profile_id, content, Some(context_memories))
            .await?;

        if relevance.is_relevant {
            return Ok(None);
        }

        let skip_reason = Self::skip_reason(&relevance);
        info!(
            profile_id = %profile_id,
            relevance_score = relevance.relevance_score,
            reason = %skip_reason,
            "Event skipped due to irrelevance"
        );

        Ok(Some(SkipDecision {
            reason: skip_reason,
            relevance_score: relevance.relevance_score,
        }))
    }

    fn processing_context(
        &self,
        event_handler: EventHandler,
        context_memories: Vec<Memory>,
    ) -> EventProcessingContext {
        EventProcessingContext::new(
            Arc::new(MemoryProcessor::new(self.llm_provider.clone())),
            Arc::new(event_handler),
            self.llm_provider.clone(),
            self.embedding_provider.clone(),
            self.embedding_provider_name.clone(),
            self.qdrant_repo.clone(),
            self.memory_matcher.clone(),
            self.structured_event_repo.clone(),
            context_memories,
        )
    }

    fn event_input_from_request(request: CreateFromEventRequest) -> CreateEventInput {
        CreateEventInput {
            profile_id: request.profile_id,
            owner_id: request.owner_id,
            scope_id: request.scope_id,
            content: request.content,
            context: request.context,
            source: request.source,
            event_time: None,
        }
    }

    fn skip_reason(relevance: &RelevanceCheckResult) -> String {
        format!(
            "事件与系统业务领域不相关。相关性分数: {:.2}, 原因: {}",
            relevance.relevance_score, relevance.reason
        )
    }
}
