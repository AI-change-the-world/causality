//! Service layer for Memory Server
//!
//! Contains business logic services for memory operations, retrieval, lifecycle management,
//! and configuration management.

mod config_center;
mod lifecycle_manager;
mod memory_guard;
mod memory_processor;
mod retrieval_engine;

pub use config_center::{ConfigCenter, LlmProviderInfo, ProviderInfo};
pub use lifecycle_manager::LifecycleManager;
pub use memory_guard::{
    CreateFromEventRequest, CreateFromEventResult, MemoryGuard, UpdateMemoryRequest,
};
pub use memory_processor::{
    EnhancedQuery, ExtractFromEventRequest, ExtractFromEventResult, MemoryProcessor,
    ProcessMemoryRequest, ProcessMemoryResult, ProcessingError,
};
pub use retrieval_engine::{RetrievalEngine, RetrieveRequest, RetrieveResponse, RetrievedMemory};
