//! API layer for Memory Server
//!
//! Contains HTTP handlers for all API endpoints using Axum.

mod admin;
mod audit;
mod config;
mod event;
pub mod health;
mod memory;
mod profile;
mod retrieval;

pub use admin::admin_routes;
pub use audit::audit_routes;
pub use config::config_routes;
pub use event::event_async_routes;
pub use event::event_routes;
pub use health::health_routes;
pub use memory::memory_routes;
pub use profile::profile_routes;
pub use retrieval::retrieval_routes;

use axum::Router;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use std::sync::Arc;

use crate::embedding::EmbeddingProvider;
use crate::llm::LlmProvider;
use crate::repository::QdrantRepository;
use crate::service::AlwaysConsistentChecker;
use crate::service::{
    EventIngestionService, LifecycleManager, MemoryGuard, ProfileService, RetrievalEngine,
};

/// Application state shared across all handlers
/// Uses AlwaysConsistentChecker as the default consistency checker
#[derive(Clone)]
pub struct AppState {
    pub memory_guard: Arc<MemoryGuard<AlwaysConsistentChecker>>,
    pub retrieval_engine: RetrievalEngine,
    pub lifecycle_manager: LifecycleManager,
    pub event_ingestion_service: EventIngestionService,
    pub profile_service: ProfileService,
    /// Global LLM provider instance (from config.yaml)
    pub llm_provider: Arc<dyn LlmProvider>,
    /// Global Embedding provider instance (from config.yaml)
    pub embedding_provider: Arc<dyn EmbeddingProvider>,
    /// Embedding provider name (for Qdrant collection)
    pub embedding_provider_name: String,
    /// Qdrant repository for vector operations
    pub qdrant_repo: QdrantRepository,
}

/// OpenAPI documentation
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Memory Server API",
        version = "0.3.0",
        description = "A context memory governance service for Agent/LLM applications.\n\nProvides structured memory storage, retrieval, lifecycle management, and optional LLM-powered memory processing.",
        license(name = "MIT"),
    ),
    servers(
        (url = "/", description = "Local server")
    ),
    tags(
        (name = "memories", description = "Memory CRUD operations"),
        (name = "events", description = "Event creation and processing"),
        (name = "retrieval", description = "Memory retrieval and search"),
        (name = "admin", description = "Administrative operations"),
        (name = "audit", description = "Audit log queries"),
        (name = "health", description = "Health check and metrics"),
        (name = "system", description = "System profile management")
    ),
    paths(
        memory::create_memory,
        memory::get_memory,
        memory::update_memory,
        memory::delete_memory,
        memory::get_memory_history,
        memory::resolve_current_memory,
        memory::promote_memory,
        event::create_event,
        event::create_event_async,
        event::get_event,
        event::retry_event,
        retrieval::retrieve_memories,
        retrieval::auto_retrieve_memories,
        admin::run_eviction,
        admin::update_decay_scores,
        admin::rebuild_embeddings,
        audit::query_audit_logs,
        health::health_check,
        health::metrics,
        profile::create_profile,
        profile::list_profiles,
        profile::get_profile_by_id,
        profile::update_profile_by_id,
        profile::confirm_profile_schema,
    ),
    components(schemas(
        // Memory types
        memory::CreateMemoryApiRequest,
        memory::CreateMemoryResponse,
        memory::GetMemoryResponse,
        memory::UpdateMemoryApiRequest,
        memory::MemoryHistoryResponse,
        memory::MemoryVersionResponse,
        memory::ResolveCurrentMemoryResponse,
        memory::PromoteMemoryRequest,
        memory::PromoteMemoryResponse,
        // Event processing types
        event::CreateEventApiRequest,
        event::CreateEventApiResponse,
        event::CreateEventAsyncResponse,
        event::GetEventApiResponse,
        event::RelatedMemoryResponse,
        crate::domain::EventSource,
        // Retrieval types
        retrieval::RetrieveApiRequest,
        retrieval::RetrieveOptions,
        retrieval::RetrieveApiResponse,
        retrieval::RetrievedMemoryResponse,
        retrieval::AutoRetrieveApiRequest,
        retrieval::AutoRetrieveApiResponse,
        retrieval::AutoRetrieveRecord,
        // Admin types
        admin::EvictionRequest,
        admin::EvictionResponse,
        admin::DecayUpdateRequest,
        admin::DecayConfigRequest,
        admin::DecayUpdateResponse,
        admin::DecayConfigResponse,
        admin::RebuildEmbeddingsRequest,
        admin::RebuildEmbeddingsResponse,
        admin::RebuildEmbeddingFailure,
        // Audit types
        audit::AuditQueryRequest,
        audit::AuditQueryResponse,
        audit::AuditLogEntryResponse,
        // Health types
        health::HealthResponse,
        health::ComponentHealth,
        health::HealthStatus,
        // Profile types
        profile::InitializeProfileRequest,
        profile::ProfileResponse,
        profile::ConfirmProfileSchemaResponse,
        profile::UpdateProfileRequest,
        // Domain types
        crate::domain::Status,
        crate::domain::EmbeddingStatus,
        crate::domain::ProcessingStatus,
        crate::domain::SchemaStatus,
        crate::domain::InferenceType,
        crate::domain::ProcessingMode,
        crate::embedding::ProviderType,
        crate::llm::LlmProviderType,
        // Error types
        crate::error::ErrorResponse,
        crate::error::ErrorCode,
    ))
)]
pub struct ApiDoc;

/// Create the main API router with all routes
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .nest("/api/v1/systems", profile_routes())
        .nest("/api/v1/systems/{profile_id}/memories", memory_routes())
        .nest("/api/v1/systems/{profile_id}/memories", retrieval_routes())
        .nest("/api/v1/systems/{profile_id}/events", event_routes())
        .nest(
            "/api/v1/systems/{profile_id}/events-async",
            event_async_routes(),
        )
        .nest("/api/v1/systems/{profile_id}/admin", admin_routes())
        .nest("/api/v1/systems/{profile_id}/audit", audit_routes())
        .nest("/api/v1/config", config_routes())
        .merge(health_routes())
        .with_state(state)
}
