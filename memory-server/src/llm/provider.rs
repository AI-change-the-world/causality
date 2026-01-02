//! LlmProvider trait and related types
//!
//! Defines the abstract interface for LLM providers, supporting multiple
//! provider types (OpenAI, Azure OpenAI, local models like Ollama).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

/// Errors that can occur during LLM operations
#[derive(Debug, Error)]
pub enum LlmError {
    /// HTTP request failed
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    /// API returned an error response
    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },

    /// Rate limit exceeded
    #[error("Rate limit exceeded, retry after {retry_after_ms}ms")]
    RateLimitExceeded { retry_after_ms: u64 },

    /// Invalid response format
    #[error("Invalid response format: {0}")]
    InvalidResponse(String),

    /// Provider not configured
    #[error("Provider not configured: {0}")]
    NotConfigured(String),

    /// Provider is disabled
    #[error("Provider is disabled: {0}")]
    Disabled(String),

    /// All retries exhausted
    #[error("All retries exhausted after {attempts} attempts: {last_error}")]
    RetriesExhausted { attempts: u32, last_error: String },

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Content filter triggered
    #[error("Content filter triggered: {0}")]
    ContentFiltered(String),

    /// Token limit exceeded
    #[error("Token limit exceeded: max {max_tokens}, requested {requested}")]
    TokenLimitExceeded { max_tokens: u32, requested: u32 },
}

/// Type of LLM provider
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "provider_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderType {
    /// OpenAI API
    OpenAI,
    /// Azure OpenAI Service
    Azure,
    /// Local LLM (OpenAI-compatible API, e.g., Ollama)
    Local,
}

impl std::fmt::Display for LlmProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmProviderType::OpenAI => write!(f, "openai"),
            LlmProviderType::Azure => write!(f, "azure"),
            LlmProviderType::Local => write!(f, "local"),
        }
    }
}

impl std::str::FromStr for LlmProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(LlmProviderType::OpenAI),
            "azure" => Ok(LlmProviderType::Azure),
            "local" => Ok(LlmProviderType::Local),
            _ => Err(format!("Unknown LLM provider type: {}", s)),
        }
    }
}

/// Configuration for an LLM provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProviderConfig {
    /// Unique name for this provider
    pub name: String,
    /// Type of provider
    pub provider_type: LlmProviderType,
    /// API endpoint URL
    pub endpoint: String,
    /// API key (optional for local providers)
    pub api_key: Option<String>,
    /// Model name/identifier
    pub model: String,
    /// Whether this provider is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Maximum input tokens
    #[serde(default = "default_max_input_tokens")]
    pub max_input_tokens: u32,
    /// Maximum output tokens
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,
    /// Temperature for generation
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Rate limit: requests per minute
    pub rpm_limit: Option<u32>,
    /// Rate limit: tokens per minute
    pub tpm_limit: Option<u32>,
}

fn default_enabled() -> bool {
    true
}

fn default_max_input_tokens() -> u32 {
    4000
}

fn default_max_output_tokens() -> u32 {
    1000
}

fn default_temperature() -> f32 {
    0.3
}

/// Retry configuration for LLM requests
#[derive(Debug, Clone)]
pub struct LlmRetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial delay before first retry (in milliseconds)
    pub initial_delay_ms: u64,
    /// Maximum delay between retries (in milliseconds)
    pub max_delay_ms: u64,
    /// Multiplier for exponential backoff
    pub multiplier: f64,
}

impl Default for LlmRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 100,
            max_delay_ms: 5000,
            multiplier: 2.0,
        }
    }
}

impl LlmRetryConfig {
    /// Calculate delay for a given attempt number (0-indexed)
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let delay_ms = (self.initial_delay_ms as f64 * self.multiplier.powi(attempt as i32)) as u64;
        Duration::from_millis(delay_ms.min(self.max_delay_ms))
    }
}

/// Role in a chat conversation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System message (instructions)
    System,
    /// User message
    User,
    /// Assistant response
    Assistant,
}

/// A message in a chat conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Role of the message sender
    pub role: Role,
    /// Content of the message
    pub content: String,
}

impl ChatMessage {
    /// Create a system message
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    /// Create a user message
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// Create an assistant message
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// Request for chat completion
#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// Messages in the conversation
    pub messages: Vec<ChatMessage>,
    /// Optional model override (uses provider default if not specified)
    pub model: Option<String>,
    /// Optional temperature override
    pub temperature: Option<f32>,
    /// Optional max tokens override
    pub max_tokens: Option<u32>,
}

impl ChatRequest {
    /// Create a new chat request with a single user message
    pub fn new(user_message: impl Into<String>) -> Self {
        Self {
            messages: vec![ChatMessage::user(user_message)],
            model: None,
            temperature: None,
            max_tokens: None,
        }
    }

    /// Create a new chat request with system and user messages
    pub fn with_system(system: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            messages: vec![ChatMessage::system(system), ChatMessage::user(user)],
            model: None,
            temperature: None,
            max_tokens: None,
        }
    }

    /// Add a message to the conversation
    pub fn add_message(mut self, message: ChatMessage) -> Self {
        self.messages.push(message);
        self
    }

    /// Set the model
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the temperature
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set the max tokens
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }
}

/// Response from chat completion
#[derive(Debug, Clone)]
pub struct ChatResponse {
    /// The generated content
    pub content: String,
    /// Model used for generation
    pub model: String,
    /// Prompt tokens used (if available)
    pub prompt_tokens: Option<u32>,
    /// Completion tokens used (if available)
    pub completion_tokens: Option<u32>,
    /// Total tokens used (if available)
    pub total_tokens: Option<u32>,
    /// Finish reason
    pub finish_reason: Option<String>,
}

/// Trait for LLM providers
///
/// Implementations must be thread-safe and support concurrent requests.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Get the provider name
    fn name(&self) -> &str;

    /// Get the provider type
    fn provider_type(&self) -> LlmProviderType;

    /// Get the default model
    fn model(&self) -> &str;

    /// Check if the provider is enabled
    fn is_enabled(&self) -> bool;

    /// Generate a chat completion
    ///
    /// This method should handle retries internally according to the provider's
    /// retry configuration.
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_provider_type_display() {
        assert_eq!(LlmProviderType::OpenAI.to_string(), "openai");
        assert_eq!(LlmProviderType::Azure.to_string(), "azure");
        assert_eq!(LlmProviderType::Local.to_string(), "local");
    }

    #[test]
    fn test_llm_provider_type_from_str() {
        assert_eq!(
            "openai".parse::<LlmProviderType>().unwrap(),
            LlmProviderType::OpenAI
        );
        assert_eq!(
            "OPENAI".parse::<LlmProviderType>().unwrap(),
            LlmProviderType::OpenAI
        );
        assert_eq!(
            "azure".parse::<LlmProviderType>().unwrap(),
            LlmProviderType::Azure
        );
        assert_eq!(
            "local".parse::<LlmProviderType>().unwrap(),
            LlmProviderType::Local
        );
        assert!("unknown".parse::<LlmProviderType>().is_err());
    }

    #[test]
    fn test_retry_config_default() {
        let config = LlmRetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_delay_ms, 100);
        assert_eq!(config.max_delay_ms, 5000);
        assert!((config.multiplier - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_retry_config_delay_calculation() {
        let config = LlmRetryConfig::default();

        // First retry: 100ms
        assert_eq!(config.delay_for_attempt(0), Duration::from_millis(100));
        // Second retry: 200ms
        assert_eq!(config.delay_for_attempt(1), Duration::from_millis(200));
        // Third retry: 400ms
        assert_eq!(config.delay_for_attempt(2), Duration::from_millis(400));
    }

    #[test]
    fn test_retry_config_max_delay() {
        let config = LlmRetryConfig {
            max_retries: 10,
            initial_delay_ms: 1000,
            max_delay_ms: 5000,
            multiplier: 2.0,
        };

        // Should be capped at max_delay_ms
        assert_eq!(config.delay_for_attempt(5), Duration::from_millis(5000));
    }

    #[test]
    fn test_chat_message_creation() {
        let system = ChatMessage::system("You are a helpful assistant");
        assert_eq!(system.role, Role::System);
        assert_eq!(system.content, "You are a helpful assistant");

        let user = ChatMessage::user("Hello");
        assert_eq!(user.role, Role::User);
        assert_eq!(user.content, "Hello");

        let assistant = ChatMessage::assistant("Hi there!");
        assert_eq!(assistant.role, Role::Assistant);
        assert_eq!(assistant.content, "Hi there!");
    }

    #[test]
    fn test_chat_request_new() {
        let request = ChatRequest::new("Hello");
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, Role::User);
        assert_eq!(request.messages[0].content, "Hello");
        assert!(request.model.is_none());
    }

    #[test]
    fn test_chat_request_with_system() {
        let request = ChatRequest::with_system("You are helpful", "Hello");
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, Role::System);
        assert_eq!(request.messages[1].role, Role::User);
    }

    #[test]
    fn test_chat_request_builder() {
        let request = ChatRequest::new("Hello")
            .with_model("gpt-4")
            .with_temperature(0.7)
            .with_max_tokens(500);

        assert_eq!(request.model, Some("gpt-4".to_string()));
        assert_eq!(request.temperature, Some(0.7));
        assert_eq!(request.max_tokens, Some(500));
    }

    #[test]
    fn test_llm_provider_config_serialization() {
        let config = LlmProviderConfig {
            name: "openai".to_string(),
            provider_type: LlmProviderType::OpenAI,
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: Some("sk-test".to_string()),
            model: "gpt-4o-mini".to_string(),
            enabled: true,
            max_input_tokens: 4000,
            max_output_tokens: 1000,
            temperature: 0.3,
            rpm_limit: Some(500),
            tpm_limit: Some(100000),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: LlmProviderConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.name, config.name);
        assert_eq!(deserialized.provider_type, config.provider_type);
        assert_eq!(deserialized.model, config.model);
    }
}
