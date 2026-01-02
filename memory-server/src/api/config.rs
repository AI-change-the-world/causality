//! Config API handlers
//!
//! Implements:
//! - GET /api/v1/config/providers - List all embedding providers
//! - POST /api/v1/config/providers - Create a new embedding provider
//! - PUT /api/v1/config/providers/{name} - Update an embedding provider
//! - DELETE /api/v1/config/providers/{name} - Delete an embedding provider
//! - GET /api/v1/config/llm-providers - List all LLM providers
//! - POST /api/v1/config/llm-providers - Create a new LLM provider
//! - PUT /api/v1/config/llm-providers/{name} - Update an LLM provider
//! - DELETE /api/v1/config/llm-providers/{name} - Delete an LLM provider

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::AppState;
use crate::embedding::{ProviderConfig, ProviderType, RateLimitConfig};
use crate::error::AppResult;
use crate::llm::{LlmProviderConfig, LlmProviderType};
use crate::repository::{LlmPromptConfig, UpdateLlmProviderInput};
use crate::service::{LlmProviderInfo, ProviderInfo};

// ============================================================================
// Embedding Provider Types
// ============================================================================

/// Response for listing embedding providers
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListProvidersResponse {
    /// List of all configured providers
    pub providers: Vec<ProviderInfoResponse>,
    /// Name of the default provider (if set)
    pub default_provider: Option<String>,
}

/// Provider information response
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProviderInfoResponse {
    /// Provider name
    pub name: String,
    /// Provider type (openai, azure, local)
    pub provider_type: ProviderType,
    /// Whether the provider is enabled
    pub enabled: bool,
    /// Whether this is the default provider
    pub is_default: bool,
    /// Model name
    pub model: String,
    /// Embedding dimension
    pub dimension: usize,
    /// Rate limit configuration
    pub rate_limit: Option<RateLimitConfigResponse>,
}

impl From<ProviderInfo> for ProviderInfoResponse {
    fn from(info: ProviderInfo) -> Self {
        ProviderInfoResponse {
            name: info.name,
            provider_type: info.provider_type,
            enabled: info.enabled,
            is_default: info.is_default,
            model: info.model,
            dimension: info.dimension,
            rate_limit: info.rate_limit.map(|r| RateLimitConfigResponse {
                requests_per_minute: r.requests_per_minute,
                tokens_per_minute: r.tokens_per_minute,
            }),
        }
    }
}

/// Rate limit configuration
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RateLimitConfigResponse {
    /// Maximum requests per minute
    pub requests_per_minute: Option<u32>,
    /// Maximum tokens per minute
    pub tokens_per_minute: Option<u32>,
}

impl From<RateLimitConfigResponse> for RateLimitConfig {
    fn from(r: RateLimitConfigResponse) -> Self {
        RateLimitConfig {
            requests_per_minute: r.requests_per_minute,
            tokens_per_minute: r.tokens_per_minute,
        }
    }
}

/// Request body for creating a new embedding provider
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateProviderRequest {
    /// Unique provider name
    pub name: String,
    /// Provider type (openai, azure, local)
    pub provider_type: ProviderType,
    /// API endpoint URL
    pub endpoint: String,
    /// API key (optional for local providers)
    pub api_key: Option<String>,
    /// Model name
    pub model: String,
    /// Embedding dimension
    pub dimension: usize,
    /// Set as default provider
    #[serde(default)]
    pub is_default: bool,
    /// Rate limit configuration
    pub rate_limit: Option<RateLimitConfigResponse>,
}

impl From<CreateProviderRequest> for ProviderConfig {
    fn from(req: CreateProviderRequest) -> Self {
        ProviderConfig {
            name: req.name,
            provider_type: req.provider_type,
            endpoint: req.endpoint,
            api_key: req.api_key,
            model: req.model,
            dimension: req.dimension,
            enabled: true,
            rate_limit: req.rate_limit.map(Into::into),
        }
    }
}

/// Request body for updating an embedding provider
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateProviderRequest {
    /// New endpoint URL
    pub endpoint: Option<String>,
    /// New API key
    pub api_key: Option<String>,
    /// New model name
    pub model: Option<String>,
    /// Enable/disable the provider
    pub enabled: Option<bool>,
    /// Set as default provider
    pub is_default: Option<bool>,
    /// New rate limit configuration
    pub rate_limit: Option<RateLimitConfigResponse>,
}

// ============================================================================
// LLM Provider Types
// ============================================================================

/// Response for listing LLM providers
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListLlmProvidersResponse {
    /// List of all configured LLM providers
    pub providers: Vec<LlmProviderInfoResponse>,
    /// Name of the default LLM provider (if set)
    pub default_provider: Option<String>,
}

/// LLM Provider information response
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LlmProviderInfoResponse {
    /// Provider name
    pub name: String,
    /// Provider type (openai, azure, local)
    pub provider_type: LlmProviderType,
    /// Whether the provider is enabled
    pub enabled: bool,
    /// Whether this is the default provider
    pub is_default: bool,
    /// Model name
    pub model: String,
    /// Requests per minute limit
    pub rpm_limit: Option<u32>,
    /// Tokens per minute limit
    pub tpm_limit: Option<u32>,
    /// Maximum input tokens
    pub max_input_tokens: u32,
    /// Maximum output tokens
    pub max_output_tokens: u32,
    /// Temperature for generation
    pub temperature: f32,
    /// Whether compression prompt is configured
    pub has_compression_prompt: bool,
    /// Whether classification prompt is configured
    pub has_classification_prompt: bool,
}

impl From<LlmProviderInfo> for LlmProviderInfoResponse {
    fn from(info: LlmProviderInfo) -> Self {
        LlmProviderInfoResponse {
            name: info.name,
            provider_type: info.provider_type,
            enabled: info.enabled,
            is_default: info.is_default,
            model: info.model,
            rpm_limit: info.rpm_limit,
            tpm_limit: info.tpm_limit,
            max_input_tokens: info.max_input_tokens,
            max_output_tokens: info.max_output_tokens,
            temperature: info.temperature,
            has_compression_prompt: info.has_compression_prompt,
            has_classification_prompt: info.has_classification_prompt,
        }
    }
}

/// Request body for creating a new LLM provider
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateLlmProviderRequest {
    /// Unique provider name
    pub name: String,
    /// Provider type (openai, azure, local)
    pub provider_type: LlmProviderType,
    /// API endpoint URL
    pub endpoint: String,
    /// API key (optional for local providers)
    pub api_key: Option<String>,
    /// Model name
    pub model: String,
    /// Set as default provider
    #[serde(default)]
    pub is_default: bool,
    /// Requests per minute limit
    pub rpm_limit: Option<u32>,
    /// Tokens per minute limit
    pub tpm_limit: Option<u32>,
    /// Maximum input tokens (default: 4096)
    #[serde(default = "default_max_input_tokens")]
    pub max_input_tokens: u32,
    /// Maximum output tokens (default: 1024)
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,
    /// Temperature for generation (default: 0.7)
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Compression prompt template
    pub compression_prompt: Option<String>,
    /// Classification prompt template
    pub classification_prompt: Option<String>,
}

fn default_max_input_tokens() -> u32 {
    4096
}

fn default_max_output_tokens() -> u32 {
    1024
}

fn default_temperature() -> f32 {
    0.7
}

/// Request body for updating an LLM provider
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateLlmProviderRequest {
    /// New endpoint URL
    pub endpoint: Option<String>,
    /// New API key
    pub api_key: Option<String>,
    /// New model name
    pub model: Option<String>,
    /// Enable/disable the provider
    pub enabled: Option<bool>,
    /// Set as default provider
    pub is_default: Option<bool>,
    /// Requests per minute limit
    pub rpm_limit: Option<u32>,
    /// Tokens per minute limit
    pub tpm_limit: Option<u32>,
    /// Maximum input tokens
    pub max_input_tokens: Option<u32>,
    /// Maximum output tokens
    pub max_output_tokens: Option<u32>,
    /// Temperature for generation
    pub temperature: Option<f32>,
    /// Compression prompt template
    pub compression_prompt: Option<String>,
    /// Classification prompt template
    pub classification_prompt: Option<String>,
}

// ============================================================================
// Routes
// ============================================================================

/// Create config routes
pub fn config_routes() -> Router<AppState> {
    Router::new()
        // Embedding provider routes
        .route("/providers", get(list_providers))
        .route("/providers", post(create_provider))
        .route("/providers/{name}", put(update_provider))
        .route("/providers/{name}", delete(delete_provider))
        // LLM provider routes
        .route("/llm-providers", get(list_llm_providers))
        .route("/llm-providers", post(create_llm_provider))
        .route("/llm-providers/{name}", put(update_llm_provider))
        .route("/llm-providers/{name}", delete(delete_llm_provider))
}

// ============================================================================
// Embedding Provider Handlers
// ============================================================================

/// GET /api/v1/config/providers - List all embedding providers
#[utoipa::path(
    get,
    path = "/api/v1/config/providers",
    tag = "config",
    responses(
        (status = 200, description = "List of embedding providers", body = ListProvidersResponse)
    )
)]
pub async fn list_providers(
    State(state): State<AppState>,
) -> AppResult<Json<ListProvidersResponse>> {
    let providers = state.config_center.list_providers().await?;
    let default_provider = state.config_center.get_default_provider_name().await?;

    let response = ListProvidersResponse {
        providers: providers.into_iter().map(Into::into).collect(),
        default_provider,
    };

    Ok(Json(response))
}

/// POST /api/v1/config/providers - Create a new embedding provider
#[utoipa::path(
    post,
    path = "/api/v1/config/providers",
    tag = "config",
    request_body = CreateProviderRequest,
    responses(
        (status = 201, description = "Provider created", body = ProviderInfoResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse)
    )
)]
pub async fn create_provider(
    State(state): State<AppState>,
    Json(request): Json<CreateProviderRequest>,
) -> AppResult<(StatusCode, Json<ProviderInfoResponse>)> {
    let is_default = request.is_default;
    let name = request.name.clone();
    let config: ProviderConfig = request.into();

    let provider = state.config_center.create_provider(config).await?;

    // Set as default if requested
    if is_default {
        state.config_center.set_default_provider(&name).await?;
        let updated = state.config_center.get_provider(&name).await?;
        return Ok((StatusCode::CREATED, Json(updated.into())));
    }

    Ok((StatusCode::CREATED, Json(provider.into())))
}

/// PUT /api/v1/config/providers/{name} - Update an embedding provider
#[utoipa::path(
    put,
    path = "/api/v1/config/providers/{name}",
    tag = "config",
    params(
        ("name" = String, Path, description = "Provider name")
    ),
    request_body = UpdateProviderRequest,
    responses(
        (status = 200, description = "Provider updated", body = ProviderInfoResponse),
        (status = 404, description = "Provider not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn update_provider(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<UpdateProviderRequest>,
) -> AppResult<Json<ProviderInfoResponse>> {
    let provider = state
        .config_center
        .update_provider(
            &name,
            request.endpoint,
            request.api_key,
            request.model,
            request.enabled,
            request.rate_limit.map(Into::into),
        )
        .await?;

    // Set as default if requested
    if request.is_default == Some(true) {
        state.config_center.set_default_provider(&name).await?;
        let updated = state.config_center.get_provider(&name).await?;
        return Ok(Json(updated.into()));
    }

    Ok(Json(provider.into()))
}

/// DELETE /api/v1/config/providers/{name} - Delete an embedding provider
#[utoipa::path(
    delete,
    path = "/api/v1/config/providers/{name}",
    tag = "config",
    params(
        ("name" = String, Path, description = "Provider name")
    ),
    responses(
        (status = 204, description = "Provider deleted"),
        (status = 404, description = "Provider not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn delete_provider(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> AppResult<StatusCode> {
    state.config_center.delete_provider(&name).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// LLM Provider Handlers
// ============================================================================

/// GET /api/v1/config/llm-providers - List all LLM providers
#[utoipa::path(
    get,
    path = "/api/v1/config/llm-providers",
    tag = "config",
    responses(
        (status = 200, description = "List of LLM providers", body = ListLlmProvidersResponse)
    )
)]
pub async fn list_llm_providers(
    State(state): State<AppState>,
) -> AppResult<Json<ListLlmProvidersResponse>> {
    let providers = state.config_center.list_llm_providers().await?;
    let default_provider = state.config_center.get_default_llm_provider_name().await?;

    let response = ListLlmProvidersResponse {
        providers: providers.into_iter().map(Into::into).collect(),
        default_provider,
    };

    Ok(Json(response))
}

/// POST /api/v1/config/llm-providers - Create a new LLM provider
#[utoipa::path(
    post,
    path = "/api/v1/config/llm-providers",
    tag = "config",
    request_body = CreateLlmProviderRequest,
    responses(
        (status = 201, description = "LLM provider created", body = LlmProviderInfoResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse)
    )
)]
pub async fn create_llm_provider(
    State(state): State<AppState>,
    Json(request): Json<CreateLlmProviderRequest>,
) -> AppResult<(StatusCode, Json<LlmProviderInfoResponse>)> {
    let is_default = request.is_default;
    let name = request.name.clone();

    let config = LlmProviderConfig {
        name: request.name,
        provider_type: request.provider_type,
        endpoint: request.endpoint,
        api_key: request.api_key,
        model: request.model,
        enabled: true,
        rpm_limit: request.rpm_limit,
        tpm_limit: request.tpm_limit,
        max_input_tokens: request.max_input_tokens,
        max_output_tokens: request.max_output_tokens,
        temperature: request.temperature,
    };

    let prompts = if request.compression_prompt.is_some() || request.classification_prompt.is_some()
    {
        Some(LlmPromptConfig {
            compression_prompt: request.compression_prompt,
            classification_prompt: request.classification_prompt,
        })
    } else {
        None
    };

    let provider = state
        .config_center
        .create_llm_provider(config, prompts)
        .await?;

    // Set as default if requested
    if is_default {
        state.config_center.set_default_llm_provider(&name).await?;
        let updated = state.config_center.get_llm_provider(&name).await?;
        return Ok((StatusCode::CREATED, Json(updated.into())));
    }

    Ok((StatusCode::CREATED, Json(provider.into())))
}

/// PUT /api/v1/config/llm-providers/{name} - Update an LLM provider
#[utoipa::path(
    put,
    path = "/api/v1/config/llm-providers/{name}",
    tag = "config",
    params(
        ("name" = String, Path, description = "LLM provider name")
    ),
    request_body = UpdateLlmProviderRequest,
    responses(
        (status = 200, description = "LLM provider updated", body = LlmProviderInfoResponse),
        (status = 404, description = "Provider not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn update_llm_provider(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<UpdateLlmProviderRequest>,
) -> AppResult<Json<LlmProviderInfoResponse>> {
    let update = UpdateLlmProviderInput {
        endpoint: request.endpoint,
        api_key: request.api_key,
        model: request.model,
        enabled: request.enabled,
        rpm_limit: request.rpm_limit,
        tpm_limit: request.tpm_limit,
        max_input_tokens: request.max_input_tokens,
        max_output_tokens: request.max_output_tokens,
        temperature: request.temperature,
        compression_prompt: request.compression_prompt,
        classification_prompt: request.classification_prompt,
    };

    let provider = state
        .config_center
        .update_llm_provider(&name, update)
        .await?;

    // Set as default if requested
    if request.is_default == Some(true) {
        state.config_center.set_default_llm_provider(&name).await?;
        let updated = state.config_center.get_llm_provider(&name).await?;
        return Ok(Json(updated.into()));
    }

    Ok(Json(provider.into()))
}

/// DELETE /api/v1/config/llm-providers/{name} - Delete an LLM provider
#[utoipa::path(
    delete,
    path = "/api/v1/config/llm-providers/{name}",
    tag = "config",
    params(
        ("name" = String, Path, description = "LLM provider name")
    ),
    responses(
        (status = 204, description = "LLM provider deleted"),
        (status = 404, description = "Provider not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn delete_llm_provider(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> AppResult<StatusCode> {
    state.config_center.delete_llm_provider(&name).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_provider_request_conversion() {
        let request = CreateProviderRequest {
            name: "test-provider".to_string(),
            provider_type: ProviderType::OpenAI,
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: Some("sk-test".to_string()),
            model: "text-embedding-3-small".to_string(),
            dimension: 1536,
            is_default: false,
            rate_limit: Some(RateLimitConfigResponse {
                requests_per_minute: Some(500),
                tokens_per_minute: Some(1000000),
            }),
        };

        let config: ProviderConfig = request.clone().into();

        assert_eq!(config.name, request.name);
        assert_eq!(config.provider_type, request.provider_type);
        assert_eq!(config.endpoint, request.endpoint);
        assert_eq!(config.api_key, request.api_key);
        assert_eq!(config.model, request.model);
        assert_eq!(config.dimension, request.dimension);
        assert!(config.enabled);
        assert!(config.rate_limit.is_some());
    }

    #[test]
    fn test_rate_limit_config_conversion() {
        let response = RateLimitConfigResponse {
            requests_per_minute: Some(100),
            tokens_per_minute: Some(50000),
        };

        let config: RateLimitConfig = response.clone().into();

        assert_eq!(config.requests_per_minute, response.requests_per_minute);
        assert_eq!(config.tokens_per_minute, response.tokens_per_minute);
    }
}
