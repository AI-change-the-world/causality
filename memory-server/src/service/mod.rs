//! Service layer for Memory Server
//!
//! Contains business logic services for memory operations, retrieval, lifecycle management,
//! and configuration management.

mod decay_calculator;
mod event_handler;
mod event_ingestion;
mod eviction_manager;
mod global_promoter;
mod lifecycle_manager;
mod memory_facade;
mod memory_guard;
mod memory_matcher;
mod memory_processor;
mod memory_reconciler;
mod profile_service;
mod retrieval_engine;

pub use decay_calculator::{
    calculate_decay_score, calculate_decay_score_at_time, DecayCalculator, DecayConfig,
};
pub use event_handler::{EventAnalysis, EventHandler, RelevanceCheckResult};
pub use event_ingestion::{EventIngestionResult, EventIngestionService};
pub use eviction_manager::{EvictionConfig, EvictionManager, EvictionResult};
pub use global_promoter::{GlobalPromoter, PromotionCheck, PromotionCriteria};
pub use lifecycle_manager::LifecycleManager;
pub use memory_facade::MemoryFacade;
pub use memory_guard::{
    CreateFromEventRequest, CreateFromEventResult, EventProcessingContext, MemoryGuard,
    ProcessEventResult, UpdateMemoryRequest,
};
pub use memory_matcher::{MatchResult, MatcherConfig, MemoryMatcher};
pub use memory_processor::{
    EnhancedQuery, ExtractFromEventRequest, ExtractFromEventResult, ExtractedMemory,
    MemoryProcessor, ProcessingError,
};
pub use memory_reconciler::{
    AlwaysConflictingChecker, AlwaysConsistentChecker, ConsistencyChecker, ConsistencyResult,
    LlmConsistencyChecker, MemoryReconciler, ReconcileOutcome, ReconcilerConfig,
};
pub use profile_service::ProfileService;
pub use retrieval_engine::{
    CurrentMemoryResolution, MemoryEvidence, MemoryHistory, RetrievalEngine, RetrieveRequest,
    RetrieveResponse, RetrievedMemory,
};
