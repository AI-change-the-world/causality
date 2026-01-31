//! Repository layer for Memory Server
//!
//! Contains database access implementations for memories, audit logs, configuration,
//! and vector database operations.

mod audit_repo;
mod config_repo;
mod event_repo;
mod llm_provider_repo;
mod memory_repo;
mod profile_repo;
mod qdrant_repo;
mod structured_event_repo;

pub use audit_repo::{AuditLogEntry, AuditOperation, AuditQueryParams, AuditRepository};
pub use config_repo::{ConfigRepository, EmbeddingProviderRecord, UpdateProviderInput};
pub use event_repo::EventRepository;
pub use llm_provider_repo::{LlmProviderRecord, LlmProviderRepository, UpdateLlmProviderInput};
pub use memory_repo::{FullTextSearchOptions, FullTextSearchResult, MemoryRepository};
pub use profile_repo::ProfileRepository;
pub use qdrant_repo::{
    MultiCollectionSearch, QdrantRepository, VectorFilter, VectorPayload, VectorSearchResult,
};
pub use structured_event_repo::StructuredEventRepository;

// Re-export domain types that are commonly used with repositories
pub use crate::domain::CreateMemoryFromEventInput;
