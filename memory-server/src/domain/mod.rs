//! Domain layer for Memory Server
//!
//! Contains core domain types, entities, and validation logic.
//!
//! Architecture changes:
//! - Removed Layer and ScopeType (simplified to user-defined strings)
//! - Added version chain support for Memory
//! - Added LFU eviction support

mod event;
mod memory;
mod status;

pub use event::{
    CreateEventInput, CreateEventValidation, Event, EventMemoryRelation, EventMemoryRelationType,
    PromotionCriteria,
};
pub use memory::{
    CreateMemoryFromEventInput, CreateMemoryInput, CreateMemoryValidation,
    CreateSupersedingMemoryInput, Memory,
};
pub use status::{
    EmbeddingStatus, ExtractedMemory, InferenceType, MemoryCategory, ProcessingMode,
    ProcessingStatus, Status,
};
