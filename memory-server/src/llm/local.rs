//! Local LLM Provider
//!
//! Implements the LlmProvider trait for local LLM models that expose
//! an OpenAI-compatible API (e.g., Ollama, vLLM, LocalAI).

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::sleep;
use tracing::{debug, error, warn};

use super::{
    ChatMessage, ChatRequest, ChatResponse, LlmError, LlmProvider, LlmProviderConfig,
    LlmProviderType, LlmRetryConfig, Role,
};

/// OpenAI-compatible Chat Completion request body
#[derive(Debug, Serialize)]
struct LocalChatRequest {
    model: String,
    messages: Vec<LocalChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize)]
struct LocalChatMessage {
    role: String,
    content: String,
}

impl From<&ChatMessage> for LocalChatMessage {
    fn from(msg: &ChatMessage) -> Self {
        Self {
            role: match msg.role {
                Role::System => "system".to_string(),
                Role::User => "user".to_string(),
                Role::Assistant => "assistant".to_string(),
            },
            content: msg.content.clone(),
        }
    }
}

/// OpenAI-compatible Chat Completion response
#[derive(Debug, Deserialize)]
struct LocalChatResponse {
    choices: Vec<LocalChatChoice>,
    model: String,
    #[serde(default)]
    usage: Option<LocalUsage>,
}

#[derive(Debug, Deserialize)]
struct LocalChatChoice {
    message: LocalChatResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalChatResponseMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

/// Local LLM Provider
///
/// Implements chat completion using a local model server that exposes
/// an OpenAI-compatible API. This is useful for models served via
/// Ollama, vLLM, LocalAI, or text-generation-inference.
pub struct LocalLlmProvider {
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
    /// Default temperature
    temperature: f32,
    /// Default max tokens
    max_tokens: u32,
    /// Whether the provider is enabled
    enabled: AtomicBool,
    /// Retry configuration
    retry_config: LlmRetryConfig,
}

impl LocalLlmProvider {
    /// Create a new local LLM provider from configuration
    pub fn new(config: LlmProviderConfig) -> Result<Self, LlmError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300)) // Longer timeout for local models
            .build()
            .map_err(LlmError::HttpError)?;

        Ok(Self {
            name: config.name,
            client,
            endpoint: config.endpoint,
            api_key: config.api_key,
            model: config.model,
            temperature: config.temperature,
            max_tokens: config.max_output_tokens,
            enabled: AtomicBool::new(config.enabled),
            retry_config: LlmRetryConfig::default(),
        })
    }

    /// Create a new local LLM provider with custom retry configuration
    pub fn with_retry_config(mut self, retry_config: LlmRetryConfig) -> Self {
        self.retry_config = retry_config;
        self
    }

    /// Set the enabled state
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    /// Build the chat completions API URL
    fn chat_url(&self) -> String {
        let base = self.endpoint.trim_end_matches('/');
        // Support both /chat/completions and /v1/chat/completions endpoints
        if base.ends_with("/v1") {
            format!("{}/chat/completions", base)
        } else {
            format!("{}/v1/chat/completions", base)
        }
    }

    /// Make a single chat request (without retry)
    async fn make_request(&self, request: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let model = request.model.as_deref().unwrap_or(&self.model);
        let temperature = request.temperature.unwrap_or(self.temperature);
        let max_tokens = request.max_tokens.unwrap_or(self.max_tokens);

        let messages: Vec<LocalChatMessage> = request.messages.iter().map(|m| m.into()).collect();

        let request_body = LocalChatRequest {
            model: model.to_string(),
            messages,
            temperature: Some(temperature),
            max_tokens: Some(max_tokens),
            stream: Some(false),
        };

        debug!(
            provider = %self.name,
            model = %model,
            message_count = request.messages.len(),
            "Making local LLM chat request"
        );

        let mut request_builder = self
            .client
            .post(&self.chat_url())
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
            let api_response: LocalChatResponse = response
                .json()
                .await
                .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

            let choice =
                api_response.choices.into_iter().next().ok_or_else(|| {
                    LlmError::InvalidResponse("No choices in response".to_string())
                })?;

            let content = choice.message.content.unwrap_or_default();

            Ok(ChatResponse {
                content,
                model: api_response.model,
                prompt_tokens: api_response.usage.as_ref().and_then(|u| u.prompt_tokens),
                completion_tokens: api_response
                    .usage
                    .as_ref()
                    .and_then(|u| u.completion_tokens),
                total_tokens: api_response.usage.as_ref().and_then(|u| u.total_tokens),
                finish_reason: choice.finish_reason,
            })
        } else if status.as_u16() == 429 {
            // Rate limit exceeded
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1);

            Err(LlmError::RateLimitExceeded {
                retry_after_ms: retry_after * 1000,
            })
        } else {
            // Try to get error message from response
            let error_text = response.text().await.unwrap_or_default();

            Err(LlmError::ApiError {
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
impl LlmProvider for LocalLlmProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn provider_type(&self) -> LlmProviderType {
        LlmProviderType::Local
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
        if !self.is_enabled() {
            return Err(LlmError::Disabled(self.name.clone()));
        }

        let mut last_error = String::new();

        for attempt in 0..=self.retry_config.max_retries {
            match self.make_request(&request).await {
                Ok(response) => {
                    debug!(
                        provider = %self.name,
                        attempt = attempt,
                        total_tokens = ?response.total_tokens,
                        "Local LLM chat request succeeded"
                    );
                    return Ok(response);
                }
                Err(e) => {
                    last_error = e.to_string();

                    // Check if we should retry
                    let should_retry = match &e {
                        LlmError::RateLimitExceeded { .. } => true,
                        LlmError::HttpError(_) => true,
                        LlmError::ApiError { status, .. } => {
                            // Retry on 5xx errors and connection errors
                            *status >= 500
                        }
                        _ => false,
                    };

                    if should_retry && attempt < self.retry_config.max_retries {
                        let delay = match &e {
                            LlmError::RateLimitExceeded { retry_after_ms } => {
                                std::time::Duration::from_millis(*retry_after_ms)
                            }
                            _ => self.retry_config.delay_for_attempt(attempt),
                        };

                        warn!(
                            provider = %self.name,
                            attempt = attempt,
                            delay_ms = delay.as_millis(),
                            error = %e,
                            "Local LLM chat request failed, retrying"
                        );

                        sleep(delay).await;
                    } else {
                        error!(
                            provider = %self.name,
                            attempt = attempt,
                            error = %e,
                            "Local LLM chat request failed, not retrying"
                        );

                        if attempt >= self.retry_config.max_retries {
                            return Err(LlmError::RetriesExhausted {
                                attempts: attempt + 1,
                                last_error,
                            });
                        }
                        return Err(e);
                    }
                }
            }
        }

        Err(LlmError::RetriesExhausted {
            attempts: self.retry_config.max_retries + 1,
            last_error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> LlmProviderConfig {
        LlmProviderConfig {
            name: "test-local".to_string(),
            provider_type: LlmProviderType::Local,
            endpoint: "http://localhost:11434".to_string(),
            api_key: None,
            model: "llama3.2".to_string(),
            enabled: true,
            max_input_tokens: 4000,
            max_output_tokens: 1000,
            temperature: 0.3,
            rpm_limit: None,
            tpm_limit: None,
        }
    }

    #[test]
    fn test_local_llm_provider_creation() {
        let config = test_config();
        let provider = LocalLlmProvider::new(config).unwrap();

        assert_eq!(provider.name(), "test-local");
        assert_eq!(provider.provider_type(), LlmProviderType::Local);
        assert_eq!(provider.model(), "llama3.2");
        assert!(provider.is_enabled());
    }

    #[test]
    fn test_local_llm_provider_no_api_key_required() {
        let config = test_config();
        let result = LocalLlmProvider::new(config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_local_llm_provider_with_api_key() {
        let mut config = test_config();
        config.api_key = Some("local-api-key".to_string());

        let provider = LocalLlmProvider::new(config).unwrap();
        assert!(provider.api_key.is_some());
    }

    #[test]
    fn test_local_llm_provider_enable_disable() {
        let config = test_config();
        let provider = LocalLlmProvider::new(config).unwrap();

        assert!(provider.is_enabled());
        provider.set_enabled(false);
        assert!(!provider.is_enabled());
        provider.set_enabled(true);
        assert!(provider.is_enabled());
    }

    #[test]
    fn test_chat_url_without_v1() {
        let config = test_config();
        let provider = LocalLlmProvider::new(config).unwrap();

        assert_eq!(
            provider.chat_url(),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn test_chat_url_with_v1() {
        let mut config = test_config();
        config.endpoint = "http://localhost:11434/v1".to_string();
        let provider = LocalLlmProvider::new(config).unwrap();

        assert_eq!(
            provider.chat_url(),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn test_chat_url_trailing_slash() {
        let mut config = test_config();
        config.endpoint = "http://localhost:11434/".to_string();
        let provider = LocalLlmProvider::new(config).unwrap();

        assert_eq!(
            provider.chat_url(),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn test_custom_retry_config() {
        let config = test_config();
        let retry_config = LlmRetryConfig {
            max_retries: 5,
            initial_delay_ms: 200,
            max_delay_ms: 10000,
            multiplier: 3.0,
        };

        let provider = LocalLlmProvider::new(config)
            .unwrap()
            .with_retry_config(retry_config);

        assert_eq!(provider.retry_config.max_retries, 5);
        assert_eq!(provider.retry_config.initial_delay_ms, 200);
    }

    #[tokio::test]
    async fn test_disabled_provider_returns_error() {
        let mut config = test_config();
        config.enabled = false;
        let provider = LocalLlmProvider::new(config).unwrap();

        let request = ChatRequest::new("Hello");
        let result = provider.chat(request).await;

        assert!(matches!(result, Err(LlmError::Disabled(_))));
    }
}
