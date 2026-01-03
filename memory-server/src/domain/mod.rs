//! Domain layer for Memory Server
//!
//! Contains core domain types, entities, and validation logic.

mod event;
mod layer;
mod memory;
mod scope;
mod status;

pub use event::{
    CreateEventInput, Event, EventMemoryRelation, EventMemoryRelationType, PromotionCriteria,
};
pub use layer::Layer;
pub use memory::{CreateMemoryFromEventInput, CreateMemoryInput, CreateMemoryValidation, Memory};
pub use scope::ScopeType;
pub use status::{
    EmbeddingStatus, ExtractedMemory, InferenceType, MemoryCategory, ProcessingMode,
    ProcessingStatus, Status, UpdateMode,
};
