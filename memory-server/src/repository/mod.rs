//! Repository layer for Memory Server
//!
//! Contains database access implementations for memories, audit logs, and configuration.

mod audit_repo;
mod config_repo;
mod llm_provider_repo;
mod memory_repo;

pub use audit_repo::{AuditLogEntry, AuditOperation, AuditQueryParams, AuditRepository};
pub use config_repo::{ConfigRepository, EmbeddingProviderRecord, UpdateProviderInput};
pub use llm_provider_repo::{
    LlmPromptConfig, LlmProviderRecord, LlmProviderRepository, UpdateLlmProviderInput,
};
pub use memory_repo::{
    FullTextSearchOptions, FullTextSearchResult, MemoryRepository, UpdateMemoryInput,
};
