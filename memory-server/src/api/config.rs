//! Config API handlers
//!
//! Implements:
//! - GET /api/v1/config/providers - List all embedding providers
//! - POST /api/v1/config/providers - Create a new embedding provider
//! - PUT /api/v1/config/providers/{name} - Update an embedding provider
//! - PUT /api/v1/config/default-provider - Set the default provider

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::AppState;
use crate::embedding::{ProviderConfig, ProviderType, RateLimitConfig};
use crate::error::AppResult;
use crate::service::ProviderInfo;

/// Response for listing providers
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

/// Rate limit configuration response
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

/// Request body for creating a new provider
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

/// Request body for updating a provider
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
    /// New rate limit configuration
    pub rate_limit: Option<RateLimitConfigResponse>,
}

/// Request body for setting the default provider
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SetDefaultProviderRequest {
    /// Name of the provider to set as default
    pub provider_name: String,
}

/// Create config routes
pub fn config_routes() -> Router<AppState> {
    Router::new()
        .route("/providers", get(list_providers))
        .route("/providers", post(create_provider))
        .route("/providers/{name}", put(update_provider))
        .route("/default-provider", put(set_default_provider))
}

/// GET /api/v1/config/providers - List all embedding providers
#[utoipa::path(
    get,
    path = "/api/v1/config/providers",
    tag = "config",
    responses(
        (status = 200, description = "List of providers", body = ListProvidersResponse)
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
    let config: ProviderConfig = request.into();

    let provider = state.config_center.create_provider(config).await?;

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

    Ok(Json(provider.into()))
}

/// PUT /api/v1/config/default-provider - Set the default provider
#[utoipa::path(
    put,
    path = "/api/v1/config/default-provider",
    tag = "config",
    request_body = SetDefaultProviderRequest,
    responses(
        (status = 200, description = "Default provider set", body = ProviderInfoResponse),
        (status = 400, description = "Provider disabled", body = crate::error::ErrorResponse),
        (status = 404, description = "Provider not found", body = crate::error::ErrorResponse)
    )
)]
pub async fn set_default_provider(
    State(state): State<AppState>,
    Json(request): Json<SetDefaultProviderRequest>,
) -> AppResult<Json<ProviderInfoResponse>> {
    let provider = state
        .config_center
        .set_default_provider(&request.provider_name)
        .await?;

    Ok(Json(provider.into()))
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
