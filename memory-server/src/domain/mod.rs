//! Domain layer for Memory Server
//!
//! Contains core domain types, entities, and validation logic.
//!
//! Architecture changes:
//! - Removed Layer and ScopeType (simplified to user-defined strings)
//! - Added version chain support for Memory
//! - Added LFU eviction support
//! - Added SystemProfile for system configuration

mod event;
mod memory;
mod metadata_filter;
mod profile;
mod status;

pub use event::{
    CreateEventInput, CreateEventValidation, Event, EventMemoryRelation, EventMemoryRelationType,
    EventSource, PromotionCriteria,
};
pub use memory::{
    CreateMemoryFromEventInput, CreateMemoryInput, CreateMemoryValidation,
    CreateSupersedingMemoryInput, Memory,
};
pub use metadata_filter::{
    MetadataFilter, MetadataFilterClause, MetadataFilterOperators, MetadataFilterPredicate,
};
pub use profile::{
    CreateProfileInput, ParsedProfile, ProfileValidation, SchemaStatus, SystemProfile,
    UpdateProfileInput, MAX_NAME_LENGTH,
};
pub use status::{EmbeddingStatus, InferenceType, ProcessingMode, ProcessingStatus, Status};
