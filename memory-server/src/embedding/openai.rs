//! OpenAI Embedding Provider
//!
//! Implements the EmbeddingProvider trait for OpenAI's embedding API.
//! Supports retry logic with exponential backoff.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::sleep;
use tracing::{debug, error, warn};

use super::{
    EmbeddingError, EmbeddingProvider, EmbeddingRequest, EmbeddingResponse, ProviderConfig,
    ProviderType, RetryConfig,
};

/// OpenAI embedding API request body
#[derive(Debug, Serialize)]
struct OpenAIEmbeddingRequest {
    input: String,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding_format: Option<String>,
}

/// OpenAI embedding API response
#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingResponse {
    data: Vec<OpenAIEmbeddingData>,
    model: String,
    usage: OpenAIUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingData {
    embedding: Vec<f32>,
    #[allow(dead_code)]
    index: usize,
}

#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    #[allow(dead_code)]
    prompt_tokens: u32,
    total_tokens: u32,
}

/// OpenAI API error response
#[derive(Debug, Deserialize)]
struct OpenAIErrorResponse {
    error: OpenAIError,
}

#[derive(Debug, Deserialize)]
struct OpenAIError {
    message: String,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    error_type: Option<String>,
    #[allow(dead_code)]
    code: Option<String>,
}

/// OpenAI Embedding Provider
///
/// Implements embedding generation using OpenAI's API with retry logic.
pub struct OpenAIProvider {
    /// Provider name
    name: String,
    /// HTTP client
    client: Client,
    /// API endpoint
    endpoint: String,
    /// API key
    api_key: String,
    /// Model name
    model: String,
    /// Expected embedding dimension
    dimension: usize,
    /// Whether the provider is enabled
    enabled: AtomicBool,
    /// Retry configuration
    retry_config: RetryConfig,
}

impl OpenAIProvider {
    /// Create a new OpenAI provider from configuration
    pub fn new(config: ProviderConfig) -> Result<Self, EmbeddingError> {
        let api_key = config.api_key.ok_or_else(|| {
            EmbeddingError::NotConfigured("API key is required for OpenAI provider".to_string())
        })?;

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(EmbeddingError::HttpError)?;

        Ok(Self {
            name: config.name,
            client,
            endpoint: config.endpoint,
            api_key,
            model: config.model,
            dimension: config.dimension,
            enabled: AtomicBool::new(config.enabled),
            retry_config: RetryConfig::default(),
        })
    }

    /// Create a new OpenAI provider with custom retry configuration
    pub fn with_retry_config(mut self, retry_config: RetryConfig) -> Self {
        self.retry_config = retry_config;
        self
    }

    /// Set the enabled state
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    /// Build the embeddings API URL
    fn embeddings_url(&self) -> String {
        let base = self.endpoint.trim_end_matches('/');
        format!("{}/embeddings", base)
    }

    /// Make a single embedding request (without retry)
    async fn make_request(
        &self,
        text: &str,
        model: &str,
    ) -> Result<EmbeddingResponse, EmbeddingError> {
        let request_body = OpenAIEmbeddingRequest {
            input: text.to_string(),
            model: model.to_string(),
            encoding_format: Some("float".to_string()),
        };

        debug!(
            provider = %self.name,
            model = %model,
            text_len = text.len(),
            "Making OpenAI embedding request"
        );

        let response = self
            .client
            .post(&self.embeddings_url())
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let status = response.status();

        if status.is_success() {
            let api_response: OpenAIEmbeddingResponse = response
                .json()
                .await
                .map_err(|e| EmbeddingError::InvalidResponse(e.to_string()))?;

            let embedding_data = api_response.data.into_iter().next().ok_or_else(|| {
                EmbeddingError::InvalidResponse("No embedding data in response".to_string())
            })?;

            let response = EmbeddingResponse {
                embedding: embedding_data.embedding,
                model: api_response.model,
                token_count: Some(api_response.usage.total_tokens),
            };

            // Validate dimension
            self.validate_dimension(&response.embedding)?;

            Ok(response)
        } else if status.as_u16() == 429 {
            // Rate limit exceeded
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1000);

            Err(EmbeddingError::RateLimitExceeded {
                retry_after_ms: retry_after * 1000,
            })
        } else {
            // Try to parse error response
            let error_text = response.text().await.unwrap_or_default();
            let error_message = serde_json::from_str::<OpenAIErrorResponse>(&error_text)
                .map(|e| e.error.message)
                .unwrap_or(error_text);

            Err(EmbeddingError::ApiError {
                status: status.as_u16(),
                message: error_message,
            })
        }
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAIProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::OpenAI
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, EmbeddingError> {
        if !self.is_enabled() {
            return Err(EmbeddingError::Disabled(self.name.clone()));
        }

        let model = request.model.as_deref().unwrap_or(&self.model);
        let mut last_error = String::new();

        for attempt in 0..=self.retry_config.max_retries {
            match self.make_request(&request.text, model).await {
                Ok(response) => {
                    debug!(
                        provider = %self.name,
                        attempt = attempt,
                        dimension = response.dimension(),
                        "Embedding request succeeded"
                    );
                    return Ok(response);
                }
                Err(e) => {
                    last_error = e.to_string();

                    // Check if we should retry
                    let should_retry = match &e {
                        EmbeddingError::RateLimitExceeded { .. } => true,
                        EmbeddingError::HttpError(_) => true,
                        EmbeddingError::ApiError { status, .. } => {
                            // Retry on 5xx errors
                            *status >= 500
                        }
                        _ => false,
                    };

                    if should_retry && attempt < self.retry_config.max_retries {
                        let delay = match &e {
                            EmbeddingError::RateLimitExceeded { retry_after_ms } => {
                                std::time::Duration::from_millis(*retry_after_ms)
                            }
                            _ => self.retry_config.delay_for_attempt(attempt),
                        };

                        warn!(
                            provider = %self.name,
                            attempt = attempt,
                            delay_ms = delay.as_millis(),
                            error = %e,
                            "Embedding request failed, retrying"
                        );

                        sleep(delay).await;
                    } else {
                        error!(
                            provider = %self.name,
                            attempt = attempt,
                            error = %e,
                            "Embedding request failed, not retrying"
                        );

                        if attempt >= self.retry_config.max_retries {
                            return Err(EmbeddingError::RetriesExhausted {
                                attempts: attempt + 1,
                                last_error,
                            });
                        }
                        return Err(e);
                    }
                }
            }
        }

        Err(EmbeddingError::RetriesExhausted {
            attempts: self.retry_config.max_retries + 1,
            last_error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ProviderConfig {
        ProviderConfig {
            name: "test-openai".to_string(),
            provider_type: ProviderType::OpenAI,
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: Some("sk-test-key".to_string()),
            model: "text-embedding-3-small".to_string(),
            dimension: 1536,
            enabled: true,
        }
    }

    #[test]
    fn test_openai_provider_creation() {
        let config = test_config();
        let provider = OpenAIProvider::new(config).unwrap();

        assert_eq!(provider.name(), "test-openai");
        assert_eq!(provider.provider_type(), ProviderType::OpenAI);
        assert_eq!(provider.dimension(), 1536);
        assert!(provider.is_enabled());
    }

    #[test]
    fn test_openai_provider_requires_api_key() {
        let mut config = test_config();
        config.api_key = None;

        let result = OpenAIProvider::new(config);
        assert!(matches!(result, Err(EmbeddingError::NotConfigured(_))));
    }

    #[test]
    fn test_openai_provider_enable_disable() {
        let config = test_config();
        let provider = OpenAIProvider::new(config).unwrap();

        assert!(provider.is_enabled());
        provider.set_enabled(false);
        assert!(!provider.is_enabled());
        provider.set_enabled(true);
        assert!(provider.is_enabled());
    }

    #[test]
    fn test_embeddings_url() {
        let config = test_config();
        let provider = OpenAIProvider::new(config).unwrap();

        assert_eq!(
            provider.embeddings_url(),
            "https://api.openai.com/v1/embeddings"
        );
    }

    #[test]
    fn test_embeddings_url_trailing_slash() {
        let mut config = test_config();
        config.endpoint = "https://api.openai.com/v1/".to_string();
        let provider = OpenAIProvider::new(config).unwrap();

        assert_eq!(
            provider.embeddings_url(),
            "https://api.openai.com/v1/embeddings"
        );
    }

    #[test]
    fn test_custom_retry_config() {
        let config = test_config();
        let retry_config = RetryConfig {
            max_retries: 5,
            initial_delay_ms: 200,
            max_delay_ms: 10000,
            multiplier: 3.0,
        };

        let provider = OpenAIProvider::new(config)
            .unwrap()
            .with_retry_config(retry_config);

        assert_eq!(provider.retry_config.max_retries, 5);
        assert_eq!(provider.retry_config.initial_delay_ms, 200);
    }

    #[tokio::test]
    async fn test_disabled_provider_returns_error() {
        let mut config = test_config();
        config.enabled = false;
        let provider = OpenAIProvider::new(config).unwrap();

        let request = EmbeddingRequest::new("test text");
        let result = provider.embed(request).await;

        assert!(matches!(result, Err(EmbeddingError::Disabled(_))));
    }
}
