//! Domain layer for Memory Server
//!
//! Contains core domain types, entities, and validation logic.

mod layer;
mod memory;
mod scope;
mod status;

pub use layer::Layer;
pub use memory::{CreateMemoryInput, CreateMemoryValidation, Memory};
pub use scope::ScopeType;
pub use status::{EmbeddingStatus, MemoryCategory, ProcessingStatus, Status, UpdateMode};
