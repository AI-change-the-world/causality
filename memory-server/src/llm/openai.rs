//! Openai LLM Provider
//!
//! Implements the LlmProvider trait for Openai's Chat Completion API.
//! Uses async-openai library with streaming to avoid timeouts.

use async_openai::{config::OpenAIConfig, error::OpenAIError, Client};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::sleep;
use tracing::{debug, error, warn};

use super::{
    ChatRequest, ChatResponse, LlmError, LlmProvider, LlmProviderConfig, LlmProviderType,
    LlmRetryConfig, ResponseFormat, Role,
};

type JsonStream = Pin<Box<dyn futures::Stream<Item = Result<Value, OpenAIError>> + Send>>;

/// Openai LLM Provider
///
/// Implements chat completion using Openai's API with streaming and retry logic.
pub struct OpenAILlmProvider {
    /// Provider name
    name: String,
    /// async-openai client
    client: Client<OpenAIConfig>,
    /// Model name
    model: String,
    /// Default temperature
    temperature: f32,
    /// Whether the provider is enabled
    enabled: AtomicBool,
    /// Retry configuration
    retry_config: LlmRetryConfig,
}

impl OpenAILlmProvider {
    /// Create a new Openai LLM provider from configuration
    pub fn new(config: LlmProviderConfig) -> Result<Self, LlmError> {
        let api_key = config.api_key.ok_or_else(|| {
            LlmError::NotConfigured("API key is required for Openai LLM provider".to_string())
        })?;

        // Build Openai config with custom endpoint if provided
        let openai_config = OpenAIConfig::new()
            .with_api_key(&api_key)
            .with_api_base(&config.endpoint);

        let client = Client::with_config(openai_config);

        Ok(Self {
            name: config.name,
            client,
            model: config.model,
            temperature: 0.7, // Default temperature for LLM processing
            enabled: AtomicBool::new(config.enabled),
            retry_config: LlmRetryConfig::default(),
        })
    }

    /// Create a new Openai LLM provider with custom retry configuration
    pub fn with_retry_config(mut self, retry_config: LlmRetryConfig) -> Self {
        self.retry_config = retry_config;
        self
    }

    /// Set the enabled state
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    fn role_name(role: Role) -> &'static str {
        match role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    fn build_chat_payload(&self, request: &ChatRequest, model: &str, temperature: f32) -> Value {
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|message| {
                json!({
                    "role": Self::role_name(message.role),
                    "content": message.content,
                })
            })
            .collect();

        let mut payload = json!({
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "stream": true,
            "enable_thinking": false,
        });

        if let Some(max_tokens) = request.max_tokens {
            payload["max_tokens"] = json!(max_tokens);
        }

        if request.response_format == ResponseFormat::JsonObject {
            payload["response_format"] = json!({ "type": "json_object" });
        }

        payload
    }

    /// Make a single chat request with streaming (without retry)
    async fn make_request(&self, request: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let model = request.model.as_deref().unwrap_or(&self.model);
        let temperature = request.temperature.unwrap_or(self.temperature);

        debug!(
            provider = %self.name,
            model = %model,
            message_count = request.messages.len(),
            streaming = true,
            json_mode = matches!(request.response_format, ResponseFormat::JsonObject),
            "Making Openai chat request with streaming"
        );

        let payload = self.build_chat_payload(request, model, temperature);
        let mut stream: JsonStream = self
            .client
            .chat()
            .create_stream_byot(payload)
            .await
            .map_err(Self::map_openai_error)?;

        // Collect streamed chunks
        let mut content = String::new();
        let mut model_used = model.to_string();
        let mut finish_reason = None;

        while let Some(result) = stream.next().await {
            match result {
                Ok(response) => {
                    if let Some(response_model) = response["model"].as_str() {
                        if !response_model.is_empty() {
                            model_used = response_model.to_string();
                        }
                    }

                    if let Some(choices) = response["choices"].as_array() {
                        for choice in choices {
                            if let Some(delta_content) = choice["delta"]["content"].as_str() {
                                content.push_str(delta_content);
                            }
                            if let Some(reason) = choice["finish_reason"].as_str() {
                                finish_reason = Some(reason.to_string());
                            }
                        }
                    }
                }
                Err(e) => {
                    return Err(Self::map_openai_error(e));
                }
            }
        }

        debug!(
            provider = %self.name,
            content_len = content.len(),
            "Streaming completed"
        );

        Ok(ChatResponse {
            content,
            model: model_used,
            prompt_tokens: None, // Not available in streaming
            completion_tokens: None,
            total_tokens: None,
            finish_reason,
        })
    }

    /// Map async-openai error to LlmError
    fn map_openai_error(e: OpenAIError) -> LlmError {
        match e {
            OpenAIError::ApiError(api_err) => {
                // Check for rate limit
                if api_err
                    .api_error
                    .message
                    .to_lowercase()
                    .contains("rate limit")
                {
                    LlmError::RateLimitExceeded {
                        retry_after_ms: 1000,
                    }
                } else {
                    LlmError::ApiError {
                        status: api_err.status_code.as_u16(),
                        message: api_err.api_error.message,
                    }
                }
            }
            OpenAIError::StreamError(msg) => {
                LlmError::InvalidResponse(format!("Stream error: {}", msg))
            }
            _ => LlmError::InvalidResponse(e.to_string()),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAILlmProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn provider_type(&self) -> LlmProviderType {
        LlmProviderType::Openai
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
                        "Chat request succeeded"
                    );
                    return Ok(response);
                }
                Err(e) => {
                    last_error = e.to_string();

                    // Check if we should retry
                    let should_retry = match &e {
                        LlmError::RateLimitExceeded { .. } => true,
                        LlmError::HttpError(_) => true,
                        LlmError::ApiError { status, .. } => *status >= 500,
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
                            "Chat request failed, retrying"
                        );

                        sleep(delay).await;
                    } else {
                        error!(
                            provider = %self.name,
                            attempt = attempt,
                            error = %e,
                            "Chat request failed, not retrying"
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
            name: "test-openai".to_string(),
            provider_type: LlmProviderType::Openai,
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: Some("sk-test-key".to_string()),
            model: "gpt-4o-mini".to_string(),
            enabled: true,
        }
    }

    #[test]
    fn test_openai_llm_provider_creation() {
        let config = test_config();
        let provider = OpenAILlmProvider::new(config).unwrap();

        assert_eq!(provider.name(), "test-openai");
        assert_eq!(provider.provider_type(), LlmProviderType::Openai);
        assert_eq!(provider.model(), "gpt-4o-mini");
        assert!(provider.is_enabled());
    }

    #[test]
    fn test_openai_llm_provider_requires_api_key() {
        let mut config = test_config();
        config.api_key = None;

        let result = OpenAILlmProvider::new(config);
        assert!(matches!(result, Err(LlmError::NotConfigured(_))));
    }

    #[test]
    fn test_openai_llm_provider_enable_disable() {
        let config = test_config();
        let provider = OpenAILlmProvider::new(config).unwrap();

        assert!(provider.is_enabled());
        provider.set_enabled(false);
        assert!(!provider.is_enabled());
        provider.set_enabled(true);
        assert!(provider.is_enabled());
    }

    #[test]
    fn test_build_chat_payload_disables_thinking() {
        let config = test_config();
        let provider = OpenAILlmProvider::new(config).unwrap();
        let request = ChatRequest::with_system("system prompt", "user prompt");

        let payload = provider.build_chat_payload(&request, "qwen3-max", 0.1);

        assert_eq!(payload["model"], "qwen3-max");
        assert_eq!(payload["stream"], true);
        assert_eq!(payload["enable_thinking"], false);
        assert_eq!(payload["messages"][0]["role"], "system");
        assert_eq!(payload["messages"][1]["role"], "user");
    }

    #[test]
    fn test_build_chat_payload_json_response_format() {
        let config = test_config();
        let provider = OpenAILlmProvider::new(config).unwrap();
        let request = ChatRequest::new("return json")
            .with_response_format(ResponseFormat::JsonObject)
            .with_max_tokens(128);

        let payload = provider.build_chat_payload(&request, "qwen3-max", 0.1);

        assert_eq!(payload["response_format"]["type"], "json_object");
        assert_eq!(payload["max_tokens"], 128);
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

        let provider = OpenAILlmProvider::new(config)
            .unwrap()
            .with_retry_config(retry_config);

        assert_eq!(provider.retry_config.max_retries, 5);
        assert_eq!(provider.retry_config.initial_delay_ms, 200);
    }

    #[tokio::test]
    async fn test_disabled_provider_returns_error() {
        let mut config = test_config();
        config.enabled = false;
        let provider = OpenAILlmProvider::new(config).unwrap();

        let request = ChatRequest::new("Hello");
        let result = provider.chat(request).await;

        assert!(matches!(result, Err(LlmError::Disabled(_))));
    }
}
