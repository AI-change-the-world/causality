//! EmbeddingProvider trait and related types
//!
//! Defines the abstract interface for embedding providers, supporting multiple
//! provider types (Openai, Azure Openai, local models).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use utoipa::ToSchema;

/// Errors that can occur during embedding operations
#[derive(Debug, Error)]
pub enum EmbeddingError {
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

    /// Dimension mismatch
    #[error("Embedding dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

/// Type of embedding provider
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, ToSchema,
)]
#[sqlx(type_name = "provider_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    /// Openai API
    Openai,
    /// Azure Openai Service
    Azure,
    /// Local embedding model (Openai-compatible API)
    #[default]
    Local,
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderType::Openai => write!(f, "openai"),
            ProviderType::Azure => write!(f, "azure"),
            ProviderType::Local => write!(f, "local"),
        }
    }
}

impl std::str::FromStr for ProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(ProviderType::Openai),
            "azure" => Ok(ProviderType::Azure),
            "local" => Ok(ProviderType::Local),
            _ => Err(format!("Unknown provider type: {}", s)),
        }
    }
}

/// Configuration for an embedding provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Unique name for this provider
    pub name: String,
    /// Type of provider
    pub provider_type: ProviderType,
    /// API endpoint URL
    pub endpoint: String,
    /// API key (optional for local providers)
    pub api_key: Option<String>,
    /// Model name/identifier
    pub model: String,
    /// Expected embedding dimension
    pub dimension: usize,
    /// Whether this provider is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Retry configuration for embedding requests
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial delay before first retry (in milliseconds)
    pub initial_delay_ms: u64,
    /// Maximum delay between retries (in milliseconds)
    pub max_delay_ms: u64,
    /// Multiplier for exponential backoff
    pub multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 100,
            max_delay_ms: 5000,
            multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Calculate delay for a given attempt number (0-indexed)
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let delay_ms = (self.initial_delay_ms as f64 * self.multiplier.powi(attempt as i32)) as u64;
        Duration::from_millis(delay_ms.min(self.max_delay_ms))
    }
}

/// Request to generate embeddings
#[derive(Debug, Clone)]
pub struct EmbeddingRequest {
    /// Text to embed
    pub text: String,
    /// Optional model override (uses provider default if not specified)
    pub model: Option<String>,
}

impl EmbeddingRequest {
    /// Create a new embedding request
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            model: None,
        }
    }

    /// Create a new embedding request with a specific model
    pub fn with_model(text: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            model: Some(model.into()),
        }
    }
}

/// Response from embedding generation
#[derive(Debug, Clone)]
pub struct EmbeddingResponse {
    /// The embedding vector
    pub embedding: Vec<f32>,
    /// Model used for generation
    pub model: String,
    /// Token count (if available)
    pub token_count: Option<u32>,
}

impl EmbeddingResponse {
    /// Get the dimension of the embedding
    pub fn dimension(&self) -> usize {
        self.embedding.len()
    }
}

/// Trait for embedding providers
///
/// Implementations must be thread-safe and support concurrent requests.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Get the provider name
    fn name(&self) -> &str;

    /// Get the provider type
    fn provider_type(&self) -> ProviderType;

    /// Get the expected embedding dimension
    fn dimension(&self) -> usize;

    /// Check if the provider is enabled
    fn is_enabled(&self) -> bool;

    /// Generate an embedding for the given text
    ///
    /// This method should handle retries internally according to the provider's
    /// retry configuration.
    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, EmbeddingError>;

    /// Generate embeddings for multiple texts in a batch
    ///
    /// Default implementation calls `embed` for each text sequentially.
    /// Providers may override this for more efficient batch processing.
    async fn embed_batch(
        &self,
        requests: Vec<EmbeddingRequest>,
    ) -> Result<Vec<EmbeddingResponse>, EmbeddingError> {
        let mut responses = Vec::with_capacity(requests.len());
        for request in requests {
            responses.push(self.embed(request).await?);
        }
        Ok(responses)
    }

    /// Validate that the embedding dimension matches the expected dimension
    fn validate_dimension(&self, embedding: &[f32]) -> Result<(), EmbeddingError> {
        if embedding.len() != self.dimension() {
            return Err(EmbeddingError::DimensionMismatch {
                expected: self.dimension(),
                actual: embedding.len(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_type_display() {
        assert_eq!(ProviderType::Openai.to_string(), "openai");
        assert_eq!(ProviderType::Azure.to_string(), "azure");
        assert_eq!(ProviderType::Local.to_string(), "local");
    }

    #[test]
    fn test_provider_type_from_str() {
        assert_eq!(
            "openai".parse::<ProviderType>().unwrap(),
            ProviderType::Openai
        );
        assert_eq!(
            "OPENAI".parse::<ProviderType>().unwrap(),
            ProviderType::Openai
        );
        assert_eq!(
            "azure".parse::<ProviderType>().unwrap(),
            ProviderType::Azure
        );
        assert_eq!(
            "local".parse::<ProviderType>().unwrap(),
            ProviderType::Local
        );
        assert!("unknown".parse::<ProviderType>().is_err());
    }

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_delay_ms, 100);
        assert_eq!(config.max_delay_ms, 5000);
        assert!((config.multiplier - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_retry_config_delay_calculation() {
        let config = RetryConfig::default();

        // First retry: 100ms
        assert_eq!(config.delay_for_attempt(0), Duration::from_millis(100));
        // Second retry: 200ms
        assert_eq!(config.delay_for_attempt(1), Duration::from_millis(200));
        // Third retry: 400ms
        assert_eq!(config.delay_for_attempt(2), Duration::from_millis(400));
        // Fourth retry: 800ms
        assert_eq!(config.delay_for_attempt(3), Duration::from_millis(800));
    }

    #[test]
    fn test_retry_config_max_delay() {
        let config = RetryConfig {
            max_retries: 10,
            initial_delay_ms: 1000,
            max_delay_ms: 5000,
            multiplier: 2.0,
        };

        // Should be capped at max_delay_ms
        assert_eq!(config.delay_for_attempt(5), Duration::from_millis(5000));
        assert_eq!(config.delay_for_attempt(10), Duration::from_millis(5000));
    }

    #[test]
    fn test_embedding_request_new() {
        let request = EmbeddingRequest::new("test text");
        assert_eq!(request.text, "test text");
        assert!(request.model.is_none());
    }

    #[test]
    fn test_embedding_request_with_model() {
        let request = EmbeddingRequest::with_model("test text", "text-embedding-3-small");
        assert_eq!(request.text, "test text");
        assert_eq!(request.model, Some("text-embedding-3-small".to_string()));
    }

    #[test]
    fn test_embedding_response_dimension() {
        let response = EmbeddingResponse {
            embedding: vec![0.1, 0.2, 0.3, 0.4],
            model: "test-model".to_string(),
            token_count: Some(5),
        };
        assert_eq!(response.dimension(), 4);
    }

    #[test]
    fn test_provider_config_serialization() {
        let config = ProviderConfig {
            name: "openai".to_string(),
            provider_type: ProviderType::Openai,
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: Some("sk-test".to_string()),
            model: "text-embedding-3-small".to_string(),
            dimension: 1536,
            enabled: true,
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ProviderConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.name, config.name);
        assert_eq!(deserialized.provider_type, config.provider_type);
        assert_eq!(deserialized.dimension, config.dimension);
    }
}
