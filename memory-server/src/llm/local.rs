//! Local LLM Provider
//!
//! Implements the LlmProvider trait for local LLM models that expose
//! an OpenAI-compatible API (e.g., Ollama, vLLM, LocalAI).

use crate::llm::OpenAILlmProvider;

/// Local LLM Provider
///
/// Implements chat completion using a local model server that exposes
/// an OpenAI-compatible API. This is useful for models served via
/// Ollama, vLLM, LocalAI, or text-generation-inference.
pub type LocalLlmProvider = OpenAILlmProvider;
