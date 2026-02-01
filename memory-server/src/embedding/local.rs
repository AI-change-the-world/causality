//! Local Embedding Provider
//!
//! Implements the EmbeddingProvider trait for local embedding models
//! that expose an Openai-compatible API (e.g., BGE, sentence-transformers).

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

/// Openai-compatible embedding request body
#[derive(Debug, Serialize)]
struct LocalEmbeddingRequest {
    input: String,
    model: String,
}

/// Openai-compatible embedding response
#[derive(Debug, Deserialize)]
struct LocalEmbeddingResponse {
    data: Vec<LocalEmbeddingData>,
    model: String,
    #[serde(default)]
    usage: Option<LocalUsage>,
}

#[derive(Debug, Deserialize)]
struct LocalEmbeddingData {
    embedding: Vec<f32>,
    #[allow(dead_code)]
    index: usize,
}

#[derive(Debug, Deserialize)]
struct LocalUsage {
    #[allow(dead_code)]
    prompt_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

/// Local Embedding Provider
///
/// Implements embedding generation using a local model server that exposes
/// an Openai-compatible API. This is useful for models like BGE, sentence-transformers,
/// or any other model served via frameworks like FastAPI, vLLM, or text-embeddings-inference.
pub struct LocalProvider {
    /// Provider name
    name: String,
    /// HTTP client
    client: Client,
    /// API endpoint
    endpoint: String,
    /// Optional API key (some local servers may require authentication)
    api_key: Option<String>,
    /// Model name
    model: String,
    /// Expected embedding dimension
    dimension: usize,
    /// Whether the provider is enabled
    enabled: AtomicBool,
    /// Retry configuration
    retry_config: RetryConfig,
}

impl LocalProvider {
    /// Create a new local provider from configuration
    pub fn new(config: ProviderConfig) -> Result<Self, EmbeddingError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60)) // Longer timeout for local models
            .build()
            .map_err(EmbeddingError::HttpError)?;

        Ok(Self {
            name: config.name,
            client,
            endpoint: config.endpoint,
            api_key: config.api_key,
            model: config.model,
            dimension: config.dimension,
            enabled: AtomicBool::new(config.enabled),
            retry_config: RetryConfig::default(),
        })
    }

    /// Create a new local provider with custom retry configuration
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
        // Support both /embeddings and /v1/embeddings endpoints
        if base.ends_with("/v1") {
            format!("{}/embeddings", base)
        } else {
            format!("{}/v1/embeddings", base)
        }
    }

    /// Make a single embedding request (without retry)
    async fn make_request(
        &self,
        text: &str,
        model: &str,
    ) -> Result<EmbeddingResponse, EmbeddingError> {
        let request_body = LocalEmbeddingRequest {
            input: text.to_string(),
            model: model.to_string(),
        };

        debug!(
            provider = %self.name,
            model = %model,
            text_len = text.len(),
            "Making local embedding request"
        );

        let mut request_builder = self
            .client
            .post(&self.embeddings_url())
            .header("Content-Type", "application/json")
            .json(&request_body);

        // Add authorization header if API key is provided
        if let Some(ref api_key) = self.api_key {
            request_builder =
                request_builder.header("Authorization", format!("Bearer {}", api_key));
        }

        let response = request_builder.send().await?;
        let status = response.status();

        if status.is_success() {
            let api_response: LocalEmbeddingResponse = response
                .json()
                .await
                .map_err(|e| EmbeddingError::InvalidResponse(e.to_string()))?;

            let embedding_data = api_response.data.into_iter().next().ok_or_else(|| {
                EmbeddingError::InvalidResponse("No embedding data in response".to_string())
            })?;

            let token_count = api_response.usage.and_then(|u| u.total_tokens);

            let response = EmbeddingResponse {
                embedding: embedding_data.embedding,
                model: api_response.model,
                token_count,
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
                .unwrap_or(1);

            Err(EmbeddingError::RateLimitExceeded {
                retry_after_ms: retry_after * 1000,
            })
        } else {
            // Try to get error message from response
            let error_text = response.text().await.unwrap_or_default();

            Err(EmbeddingError::ApiError {
                status: status.as_u16(),
                message: if error_text.is_empty() {
                    format!("HTTP {}", status)
                } else {
                    error_text
                },
            })
        }
    }
}

#[async_trait]
impl EmbeddingProvider for LocalProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::Local
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
                        "Local embedding request succeeded"
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
                            // Retry on 5xx errors and connection errors
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
                            "Local embedding request failed, retrying"
                        );

                        sleep(delay).await;
                    } else {
                        error!(
                            provider = %self.name,
                            attempt = attempt,
                            error = %e,
                            "Local embedding request failed, not retrying"
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
            name: "test-local".to_string(),
            provider_type: ProviderType::Local,
            endpoint: "http://localhost:8000".to_string(),
            api_key: None,
            model: "bge-large-zh-v1.5".to_string(),
            dimension: 1024,
            enabled: true,
        }
    }

    #[test]
    fn test_local_provider_creation() {
        let config = test_config();
        let provider = LocalProvider::new(config).unwrap();

        assert_eq!(provider.name(), "test-local");
        assert_eq!(provider.provider_type(), ProviderType::Local);
        assert_eq!(provider.dimension(), 1024);
        assert!(provider.is_enabled());
    }

    #[test]
    fn test_local_provider_no_api_key_required() {
        let config = test_config();
        let result = LocalProvider::new(config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_local_provider_with_api_key() {
        let mut config = test_config();
        config.api_key = Some("local-api-key".to_string());

        let provider = LocalProvider::new(config).unwrap();
        assert!(provider.api_key.is_some());
    }

    #[test]
    fn test_local_provider_enable_disable() {
        let config = test_config();
        let provider = LocalProvider::new(config).unwrap();

        assert!(provider.is_enabled());
        provider.set_enabled(false);
        assert!(!provider.is_enabled());
        provider.set_enabled(true);
        assert!(provider.is_enabled());
    }

    #[test]
    fn test_embeddings_url_without_v1() {
        let config = test_config();
        let provider = LocalProvider::new(config).unwrap();

        assert_eq!(
            provider.embeddings_url(),
            "http://localhost:8000/v1/embeddings"
        );
    }

    #[test]
    fn test_embeddings_url_with_v1() {
        let mut config = test_config();
        config.endpoint = "http://localhost:8000/v1".to_string();
        let provider = LocalProvider::new(config).unwrap();

        assert_eq!(
            provider.embeddings_url(),
            "http://localhost:8000/v1/embeddings"
        );
    }

    #[test]
    fn test_embeddings_url_trailing_slash() {
        let mut config = test_config();
        config.endpoint = "http://localhost:8000/".to_string();
        let provider = LocalProvider::new(config).unwrap();

        assert_eq!(
            provider.embeddings_url(),
            "http://localhost:8000/v1/embeddings"
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

        let provider = LocalProvider::new(config)
            .unwrap()
            .with_retry_config(retry_config);

        assert_eq!(provider.retry_config.max_retries, 5);
        assert_eq!(provider.retry_config.initial_delay_ms, 200);
    }

    #[tokio::test]
    async fn test_disabled_provider_returns_error() {
        let mut config = test_config();
        config.enabled = false;
        let provider = LocalProvider::new(config).unwrap();

        let request = EmbeddingRequest::new("test text");
        let result = provider.embed(request).await;

        assert!(matches!(result, Err(EmbeddingError::Disabled(_))));
    }
}
