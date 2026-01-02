//! Repository layer for Memory Server
//!
//! Contains database access implementations for memories, audit logs, configuration,
//! and vector database operations.

mod audit_repo;
mod config_repo;
mod llm_provider_repo;
mod memory_repo;
mod qdrant_repo;

pub use audit_repo::{AuditLogEntry, AuditOperation, AuditQueryParams, AuditRepository};
pub use config_repo::{ConfigRepository, EmbeddingProviderRecord, UpdateProviderInput};
pub use llm_provider_repo::{
    LlmPromptConfig, LlmProviderRecord, LlmProviderRepository, UpdateLlmProviderInput,
};
pub use memory_repo::{
    FullTextSearchOptions, FullTextSearchResult, MemoryRepository, UpdateMemoryInput,
};
pub use qdrant_repo::{
    MultiCollectionSearch, QdrantRepository, VectorFilter, VectorPayload, VectorSearchResult,
};

// Re-export domain types that are commonly used with repositories
pub use crate::domain::CreateMemoryFromEventInput;
