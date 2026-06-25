//! Lightweight SDK-style facade for common memory operations.
//!
//! This wraps the existing ingestion and retrieval services so callers can
//! use a small surface area:
//! - remember(system, owner, scope, content)
//! - recall(system, owner, scope, query)

use uuid::Uuid;

use crate::domain::EventSource;
use crate::error::AppResult;

use super::{
    CreateFromEventRequest, EventIngestionResult, EventIngestionService, RetrievalEngine,
    RetrieveRequest, RetrieveResponse,
};

/// Thin facade over event ingestion and retrieval.
#[derive(Clone)]
pub struct MemoryFacade {
    event_ingestion_service: EventIngestionService,
    retrieval_engine: RetrievalEngine,
    default_top_k: usize,
}

impl MemoryFacade {
    /// Create a new facade with default retrieval size.
    pub fn new(
        event_ingestion_service: EventIngestionService,
        retrieval_engine: RetrievalEngine,
    ) -> Self {
        Self {
            event_ingestion_service,
            retrieval_engine,
            default_top_k: 10,
        }
    }

    /// Override the default recall result size.
    pub fn with_default_top_k(mut self, default_top_k: usize) -> Self {
        self.default_top_k = default_top_k;
        self
    }

    /// Store content as memory with the default API source.
    pub async fn remember(
        &self,
        profile_id: Uuid,
        owner_id: impl Into<String>,
        scope_id: Option<String>,
        content: impl Into<String>,
    ) -> AppResult<EventIngestionResult> {
        let request = Self::remember_request(profile_id, owner_id, scope_id, content);

        self.event_ingestion_service
            .ingest_event_sync(request)
            .await
    }

    /// Recall memories with the default retrieval configuration.
    pub async fn recall(
        &self,
        profile_id: Uuid,
        owner_id: impl Into<String>,
        scope_id: Option<String>,
        query: impl Into<String>,
    ) -> AppResult<RetrieveResponse> {
        let request = Self::recall_request(
            profile_id,
            owner_id,
            scope_id,
            query,
            Some(self.default_top_k),
        );

        self.retrieval_engine.retrieve(request, None, None).await
    }

    fn remember_request(
        profile_id: Uuid,
        owner_id: impl Into<String>,
        scope_id: Option<String>,
        content: impl Into<String>,
    ) -> CreateFromEventRequest {
        CreateFromEventRequest {
            profile_id,
            owner_id: owner_id.into(),
            content: content.into(),
            context: None,
            scope_id,
            source: Some(EventSource::Api),
        }
    }

    fn recall_request(
        profile_id: Uuid,
        owner_id: impl Into<String>,
        scope_id: Option<String>,
        query: impl Into<String>,
        top_k: Option<usize>,
    ) -> RetrieveRequest {
        RetrieveRequest {
            profile_id,
            query: query.into(),
            owner_id: owner_id.into(),
            scope_id,
            top_k,
            min_score: None,
            min_confidence: None,
            use_fulltext: Some(true),
            use_vector: Some(true),
            fulltext_weight: None,
            metadata_filter: None,
            highlight: Some(false),
            include_evidence: Some(false),
            include_history: Some(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remember_request_defaults() {
        let profile_id = Uuid::new_v4();
        let request = MemoryFacade::remember_request(
            profile_id,
            "owner-1",
            Some("scope-1".to_string()),
            "remember this",
        );

        assert_eq!(request.profile_id, profile_id);
        assert_eq!(request.owner_id, "owner-1");
        assert_eq!(request.scope_id, Some("scope-1".to_string()));
        assert_eq!(request.content, "remember this");
        assert_eq!(request.context, None);
        assert_eq!(request.source, Some(EventSource::Api));
    }

    #[test]
    fn test_recall_request_defaults() {
        let profile_id = Uuid::new_v4();
        let request = MemoryFacade::recall_request(
            profile_id,
            "owner-1",
            Some("scope-1".to_string()),
            "what should I know?",
            Some(10),
        );

        assert_eq!(request.profile_id, profile_id);
        assert_eq!(request.owner_id, "owner-1");
        assert_eq!(request.scope_id, Some("scope-1".to_string()));
        assert_eq!(request.query, "what should I know?");
        assert_eq!(request.top_k, Some(10));
        assert_eq!(request.use_fulltext, Some(true));
        assert_eq!(request.use_vector, Some(true));
        assert_eq!(request.include_evidence, Some(false));
        assert_eq!(request.include_history, Some(false));
    }
}
