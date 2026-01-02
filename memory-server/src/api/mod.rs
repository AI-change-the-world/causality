//! API layer for Memory Server
//!
//! Contains HTTP handlers for all API endpoints using Axum.

mod audit;
mod config;
mod event;
pub mod health;
mod memory;
mod retrieval;

pub use audit::audit_routes;
pub use config::config_routes;
pub use event::event_routes;
pub use health::health_routes;
pub use memory::memory_routes;
pub use retrieval::retrieval_routes;

use axum::Router;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::service::{ConfigCenter, LifecycleManager, MemoryGuard, RetrievalEngine};

/// Application state shared across all handlers
#[derive(Clone)]
pub struct AppState {
    pub memory_guard: MemoryGuard,
    pub retrieval_engine: RetrievalEngine,
    pub config_center: ConfigCenter,
    pub lifecycle_manager: LifecycleManager,
}

/// OpenAPI documentation
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Memory Server API",
        version = "0.1.0",
        description = "A context memory governance service for Agent/LLM applications.\n\nProvides structured memory storage, retrieval, lifecycle management, and optional LLM-powered memory processing.",
        license(name = "MIT"),
    ),
    servers(
        (url = "/", description = "Local server")
    ),
    tags(
        (name = "memories", description = "Memory CRUD operations"),
        (name = "retrieval", description = "Memory retrieval and search"),
        (name = "config", description = "Provider configuration management"),
        (name = "audit", description = "Audit log queries"),
        (name = "health", description = "Health check and metrics")
    ),
    paths(
        memory::create_memory,
        memory::get_memory,
        memory::update_memory,
        memory::delete_memory,
        event::create_from_event,
        retrieval::retrieve_memories,
        config::list_providers,
        config::create_provider,
        config::update_provider,
        config::delete_provider,
        config::list_llm_providers,
        config::create_llm_provider,
        config::update_llm_provider,
        config::delete_llm_provider,
        audit::query_audit_logs,
        health::health_check,
        health::metrics,
    ),
    components(schemas(
        // Memory types
        memory::CreateMemoryApiRequest,
        memory::CreateMemoryResponse,
        memory::GetMemoryResponse,
        memory::UpdateMemoryApiRequest,
        // Event processing types
        event::CreateFromEventApiRequest,
        event::CreateFromEventApiResponse,
        event::ExtractedMemoryResponse,
        // Retrieval types
        retrieval::RetrieveApiRequest,
        retrieval::RetrieveApiResponse,
        retrieval::RetrievedMemoryResponse,
        // Config types - Embedding providers
        config::ListProvidersResponse,
        config::ProviderInfoResponse,
        config::CreateProviderRequest,
        config::UpdateProviderRequest,
        config::RateLimitConfigResponse,
        // Config types - LLM providers
        config::ListLlmProvidersResponse,
        config::LlmProviderInfoResponse,
        config::CreateLlmProviderRequest,
        config::UpdateLlmProviderRequest,
        // Audit types
        audit::AuditQueryRequest,
        audit::AuditQueryResponse,
        audit::AuditLogEntryResponse,
        // Health types
        health::HealthResponse,
        health::ComponentHealth,
        health::HealthStatus,
        // Domain types
        crate::domain::Layer,
        crate::domain::ScopeType,
        crate::domain::Status,
        crate::domain::UpdateMode,
        crate::domain::EmbeddingStatus,
        crate::domain::ProcessingStatus,
        crate::domain::MemoryCategory,
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
        .nest("/api/v1/memories", memory_routes())
        .nest("/api/v1/memories", retrieval_routes())
        .nest("/api/v1/memories", event_routes())
        .nest("/api/v1/config", config_routes())
        .nest("/api/v1/audit", audit_routes())
        .merge(health_routes())
        .with_state(state)
}
