//! Service layer for Memory Server
//!
//! Contains business logic services for memory operations, retrieval, lifecycle management,
//! and configuration management.

mod config_center;
mod decay_calculator;
mod eviction_manager;
mod global_promoter;
mod lifecycle_manager;
mod memory_guard;
mod memory_matcher;
mod memory_processor;
mod memory_reconciler;
mod retrieval_engine;

pub use config_center::{ConfigCenter, LlmProviderInfo, ProviderInfo};
pub use decay_calculator::{
    calculate_decay_score, calculate_decay_score_at_time, DecayCalculator, DecayConfig,
};
pub use eviction_manager::{EvictionConfig, EvictionManager, EvictionResult};
pub use global_promoter::{GlobalPromoter, PromotionCheck, PromotionCriteria};
pub use lifecycle_manager::LifecycleManager;
pub use memory_guard::{
    CreateFromEventRequest, CreateFromEventResult, EventProcessingContext, MemoryGuard,
    ProcessEventResult, UpdateMemoryRequest,
};
pub use memory_matcher::{MatchResult, MatcherConfig, MemoryMatcher};
pub use memory_processor::{
    EnhancedQuery, ExtractFromEventRequest, ExtractFromEventResult, MemoryProcessor,
    ProcessMemoryRequest, ProcessMemoryResult, ProcessingError,
};
pub use memory_reconciler::{
    AlwaysConflictingChecker, AlwaysConsistentChecker, ConsistencyChecker, ConsistencyResult,
    ExtractedMemory, MemoryReconciler, ReconcileOutcome, ReconcilerConfig,
};
pub use retrieval_engine::{
    CategoryQuery, MemoryEvidence, MemoryHistory, RetrievalEngine, RetrieveRequest,
    RetrieveResponse, RetrievedMemory,
};
