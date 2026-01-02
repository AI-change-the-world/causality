//! Embedding module for Memory Server
//!
//! Provides abstraction for embedding providers (OpenAI, Azure, local models)
//! with support for multiple providers, retry logic, and rate limiting.

mod local;
mod openai;
mod provider;

pub use local::LocalProvider;
pub use openai::OpenAIProvider;
pub use provider::{
    EmbeddingError, EmbeddingProvider, EmbeddingRequest, EmbeddingResponse, ProviderConfig,
    ProviderType, RetryConfig,
};
