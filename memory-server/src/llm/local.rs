//! Local LLM Provider
//!
//! Implements the LlmProvider trait for local LLM models that expose
//! an Openai-compatible API (e.g., Ollama, vLLM, LocalAI).

use crate::llm::OpenAILlmProvider;

/// Local LLM Provider
///
/// Implements chat completion using a local model server that exposes
/// an Openai-compatible API. This is useful for models served via
/// Ollama, vLLM, LocalAI, or text-generation-inference.
pub type LocalLlmProvider = OpenAILlmProvider;
