//! LLM module for Memory Server
//!
//! Provides abstraction for LLM providers (OpenAI, Azure, local models like Ollama)
//! with support for multiple providers, retry logic, and rate limiting.
//! Used for memory processing: compression, classification, and tag extraction.

mod local;
mod openai;
mod provider;

pub use local::LocalLlmProvider;
pub use openai::OpenAILlmProvider;
pub use provider::{
    ChatMessage, ChatRequest, ChatResponse, LlmError, LlmProvider, LlmProviderConfig,
    LlmProviderType, LlmRetryConfig, ResponseFormat, Role,
};
