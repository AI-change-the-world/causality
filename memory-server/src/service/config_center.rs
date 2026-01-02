//! ConfigCenter service
//!
//! Responsible for embedding and LLM provider management:
//! - Provider CRUD operations
//! - Rate limiting
//! - Hot configuration updates
//! - Default provider management

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::embedding::{ProviderConfig, ProviderType, RateLimitConfig};
use crate::error::{AppError, AppResult};
use crate::llm::{LlmProviderConfig, LlmProviderType};
use crate::repository::{
    ConfigRepository, EmbeddingProviderRecord, LlmPromptConfig, LlmProviderRecord,
    LlmProviderRepository, QdrantRepository, UpdateLlmProviderInput, UpdateProviderInput,
};

/// Provider information for API responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    /// Provider name
    pub name: String,
    /// Provider type
    pub provider_type: ProviderType,
    /// Whether the provider is enabled
    pub enabled: bool,
    /// Model name
    pub model: String,
    /// Embedding dimension
    pub dimension: usize,
    /// Rate limit configuration
    pub rate_limit: Option<RateLimitConfig>,
    /// Whether this is the default provider
    pub is_default: bool,
}

impl From<EmbeddingProviderRecord> for ProviderInfo {
    fn from(record: EmbeddingProviderRecord) -> Self {
        let rate_limit = if record.rpm_limit.is_some() || record.tpm_limit.is_some() {
            Some(RateLimitConfig {
                requests_per_minute: record.rpm_limit.map(|v| v as u32),
                tokens_per_minute: record.tpm_limit.map(|v| v as u32),
            })
        } else {
            None
        };

        ProviderInfo {
            name: record.name,
            provider_type: record.provider_type,
            enabled: record.enabled,
            model: record.model,
            dimension: record.dimension as usize,
            rate_limit,
            is_default: record.is_default,
        }
    }
}

/// LLM Provider information for API responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProviderInfo {
    /// Provider name
    pub name: String,
    /// Provider type
    pub provider_type: LlmProviderType,
    /// Whether the provider is enabled
    pub enabled: bool,
    /// Model name
    pub model: String,
    /// Whether this is the default provider
    pub is_default: bool,
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

impl From<LlmProviderRecord> for LlmProviderInfo {
    fn from(record: LlmProviderRecord) -> Self {
        LlmProviderInfo {
            name: record.name,
            provider_type: record.provider_type,
            enabled: record.enabled,
            model: record.model,
            is_default: record.is_default,
            rpm_limit: record.rpm_limit.map(|v| v as u32),
            tpm_limit: record.tpm_limit.map(|v| v as u32),
            max_input_tokens: record.max_input_tokens as u32,
            max_output_tokens: record.max_output_tokens as u32,
            temperature: record.temperature,
            has_compression_prompt: record.compression_prompt.is_some(),
            has_classification_prompt: record.classification_prompt.is_some(),
        }
    }
}

/// Rate limiter state for a provider
#[derive(Debug, Clone)]
struct RateLimiterState {
    /// Requests made in current window
    requests_in_window: u32,
    /// Tokens used in current window
    tokens_in_window: u32,
    /// Window start time
    window_start: std::time::Instant,
    /// Rate limit config
    config: Option<RateLimitConfig>,
}

impl RateLimiterState {
    fn new(config: Option<RateLimitConfig>) -> Self {
        Self {
            requests_in_window: 0,
            tokens_in_window: 0,
            window_start: std::time::Instant::now(),
            config,
        }
    }

    /// Check if a request can be made (and record it if so)
    fn try_acquire(&mut self, tokens: u32) -> Result<(), u64> {
        // Reset window if minute has passed
        if self.window_start.elapsed().as_secs() >= 60 {
            self.requests_in_window = 0;
            self.tokens_in_window = 0;
            self.window_start = std::time::Instant::now();
        }

        if let Some(ref config) = self.config {
            // Check RPM limit
            if let Some(rpm) = config.requests_per_minute {
                if self.requests_in_window >= rpm {
                    let wait_ms = (60 - self.window_start.elapsed().as_secs()) * 1000;
                    return Err(wait_ms.max(1));
                }
            }

            // Check TPM limit
            if let Some(tpm) = config.tokens_per_minute {
                if self.tokens_in_window + tokens > tpm {
                    let wait_ms = (60 - self.window_start.elapsed().as_secs()) * 1000;
                    return Err(wait_ms.max(1));
                }
            }
        }

        // Record the request
        self.requests_in_window += 1;
        self.tokens_in_window += tokens;

        Ok(())
    }
}

/// ConfigCenter service for provider management
#[derive(Clone)]
pub struct ConfigCenter {
    config_repo: ConfigRepository,
    llm_repo: Option<LlmProviderRepository>,
    qdrant_repo: Option<QdrantRepository>,
    /// In-memory rate limiters per embedding provider
    rate_limiters: Arc<RwLock<HashMap<String, RateLimiterState>>>,
    /// Cached embedding provider configs for hot access
    provider_cache: Arc<RwLock<HashMap<String, ProviderConfig>>>,
    /// In-memory rate limiters per LLM provider
    llm_rate_limiters: Arc<RwLock<HashMap<String, RateLimiterState>>>,
    /// Cached LLM provider configs for hot access
    llm_provider_cache: Arc<RwLock<HashMap<String, LlmProviderConfig>>>,
}

impl ConfigCenter {
    /// Create a new ConfigCenter service
    pub fn new(config_repo: ConfigRepository) -> Self {
        Self {
            config_repo,
            llm_repo: None,
            qdrant_repo: None,
            rate_limiters: Arc::new(RwLock::new(HashMap::new())),
            provider_cache: Arc::new(RwLock::new(HashMap::new())),
            llm_rate_limiters: Arc::new(RwLock::new(HashMap::new())),
            llm_provider_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new ConfigCenter service with LLM provider support
    pub fn with_llm_repo(config_repo: ConfigRepository, llm_repo: LlmProviderRepository) -> Self {
        Self {
            config_repo,
            llm_repo: Some(llm_repo),
            qdrant_repo: None,
            rate_limiters: Arc::new(RwLock::new(HashMap::new())),
            provider_cache: Arc::new(RwLock::new(HashMap::new())),
            llm_rate_limiters: Arc::new(RwLock::new(HashMap::new())),
            llm_provider_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new ConfigCenter service with full support (LLM + Qdrant)
    pub fn with_full_support(
        config_repo: ConfigRepository,
        llm_repo: LlmProviderRepository,
        qdrant_repo: QdrantRepository,
    ) -> Self {
        Self {
            config_repo,
            llm_repo: Some(llm_repo),
            qdrant_repo: Some(qdrant_repo),
            rate_limiters: Arc::new(RwLock::new(HashMap::new())),
            provider_cache: Arc::new(RwLock::new(HashMap::new())),
            llm_rate_limiters: Arc::new(RwLock::new(HashMap::new())),
            llm_provider_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the Qdrant repository (if configured)
    pub fn qdrant_repo(&self) -> Option<&QdrantRepository> {
        self.qdrant_repo.as_ref()
    }

    /// Initialize the config center by loading providers from database
    pub async fn initialize(&self) -> AppResult<()> {
        debug!("Initializing ConfigCenter");

        // Load embedding providers
        let providers = self.config_repo.list_providers().await?;

        let mut cache = self.provider_cache.write().await;
        let mut limiters = self.rate_limiters.write().await;

        for record in &providers {
            let config = record.to_provider_config();
            let rate_limit = config.rate_limit.clone();

            cache.insert(config.name.clone(), config);
            limiters.insert(record.name.clone(), RateLimiterState::new(rate_limit));
        }

        info!(
            embedding_provider_count = cache.len(),
            "Embedding providers loaded"
        );

        // Ensure Qdrant collections exist for all providers
        if let Some(ref qdrant) = self.qdrant_repo {
            for record in &providers {
                if !qdrant.collection_exists(&record.name).await {
                    if let Err(e) = qdrant
                        .create_collection(&record.name, record.dimension as usize)
                        .await
                    {
                        warn!(
                            provider_name = %record.name,
                            dimension = record.dimension,
                            error = %e,
                            "Failed to create Qdrant collection during initialization"
                        );
                    } else {
                        info!(
                            provider_name = %record.name,
                            dimension = record.dimension,
                            "Created Qdrant collection during initialization"
                        );
                    }
                }
            }
        }

        // Load LLM providers if repository is configured
        if let Some(ref llm_repo) = self.llm_repo {
            let llm_providers = llm_repo.list_providers().await?;

            let mut llm_cache = self.llm_provider_cache.write().await;
            let mut llm_limiters = self.llm_rate_limiters.write().await;

            for record in llm_providers {
                let config = record.to_provider_config();
                let rate_limit = if config.rpm_limit.is_some() || config.tpm_limit.is_some() {
                    Some(RateLimitConfig {
                        requests_per_minute: config.rpm_limit,
                        tokens_per_minute: config.tpm_limit,
                    })
                } else {
                    None
                };

                llm_cache.insert(config.name.clone(), config);
                llm_limiters.insert(record.name.clone(), RateLimiterState::new(rate_limit));
            }

            info!(llm_provider_count = llm_cache.len(), "LLM providers loaded");
        }

        info!("ConfigCenter initialized");

        Ok(())
    }

    /// List all configured providers
    pub async fn list_providers(&self) -> AppResult<Vec<ProviderInfo>> {
        let records = self.config_repo.list_providers().await?;
        Ok(records.into_iter().map(Into::into).collect())
    }

    /// Get a provider by name
    pub async fn get_provider(&self, name: &str) -> AppResult<ProviderInfo> {
        let record = self.config_repo.get_provider(name).await?;
        Ok(record.into())
    }

    /// Get the default provider
    pub async fn get_default_provider(&self) -> AppResult<ProviderInfo> {
        let record = self.config_repo.get_default_provider().await?;
        Ok(record.into())
    }

    /// Get the default provider name
    pub async fn get_default_provider_name(&self) -> AppResult<Option<String>> {
        match self.config_repo.get_default_provider().await {
            Ok(record) => Ok(Some(record.name)),
            Err(AppError::NoDefaultProvider) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Create a new provider
    pub async fn create_provider(&self, config: ProviderConfig) -> AppResult<ProviderInfo> {
        debug!(provider_name = %config.name, "Creating provider");

        let record = self.config_repo.create_provider(&config).await?;

        // Create Qdrant collection for this provider
        if let Some(ref qdrant) = self.qdrant_repo {
            if let Err(e) = qdrant
                .create_collection(&config.name, config.dimension)
                .await
            {
                // Log warning but don't fail - collection might already exist
                warn!(
                    provider_name = %config.name,
                    dimension = config.dimension,
                    error = %e,
                    "Failed to create Qdrant collection (may already exist)"
                );
            } else {
                info!(
                    provider_name = %config.name,
                    dimension = config.dimension,
                    "Created Qdrant collection for provider"
                );
            }
        }

        // Update cache
        {
            let mut cache = self.provider_cache.write().await;
            cache.insert(config.name.clone(), config.clone());
        }

        // Initialize rate limiter
        {
            let mut limiters = self.rate_limiters.write().await;
            limiters.insert(
                config.name.clone(),
                RateLimiterState::new(config.rate_limit),
            );
        }

        info!(provider_name = %record.name, "Provider created");

        Ok(record.into())
    }

    /// Update an existing provider
    pub async fn update_provider(
        &self,
        name: &str,
        endpoint: Option<String>,
        api_key: Option<String>,
        model: Option<String>,
        enabled: Option<bool>,
        rate_limit: Option<RateLimitConfig>,
    ) -> AppResult<ProviderInfo> {
        debug!(provider_name = %name, "Updating provider");

        let update = UpdateProviderInput {
            endpoint,
            api_key,
            model,
            enabled,
            rate_limit,
        };

        let record = self.config_repo.update_provider(name, &update).await?;
        let config = record.to_provider_config();

        // Update cache
        {
            let mut cache = self.provider_cache.write().await;
            cache.insert(name.to_string(), config.clone());
        }

        // Update rate limiter if rate limit changed
        if update.rate_limit.is_some() {
            let mut limiters = self.rate_limiters.write().await;
            limiters.insert(name.to_string(), RateLimiterState::new(config.rate_limit));
        }

        info!(provider_name = %name, "Provider updated");

        Ok(record.into())
    }

    /// Check if any enabled provider exists
    pub async fn has_enabled_provider(&self) -> AppResult<bool> {
        self.config_repo.has_enabled_provider().await
    }

    /// Enable or disable a provider
    pub async fn set_provider_enabled(&self, name: &str, enabled: bool) -> AppResult<ProviderInfo> {
        debug!(provider_name = %name, enabled = enabled, "Setting provider enabled state");

        let record = self.config_repo.set_provider_enabled(name, enabled).await?;

        // Update cache
        {
            let mut cache = self.provider_cache.write().await;
            if let Some(config) = cache.get_mut(name) {
                config.enabled = enabled;
            }
        }

        info!(provider_name = %name, enabled = enabled, "Provider enabled state updated");

        Ok(record.into())
    }

    /// Set the default provider
    pub async fn set_default_provider(&self, name: &str) -> AppResult<ProviderInfo> {
        debug!(provider_name = %name, "Setting default provider");

        let record = self.config_repo.set_default_provider(name).await?;

        info!(provider_name = %name, "Default provider set");

        Ok(record.into())
    }

    /// Delete a provider
    pub async fn delete_provider(&self, name: &str) -> AppResult<()> {
        debug!(provider_name = %name, "Deleting provider");

        self.config_repo.delete_provider(name).await?;

        // Delete Qdrant collection for this provider
        if let Some(ref qdrant) = self.qdrant_repo {
            if let Err(e) = qdrant.delete_collection(name).await {
                warn!(
                    provider_name = %name,
                    error = %e,
                    "Failed to delete Qdrant collection"
                );
            } else {
                info!(provider_name = %name, "Deleted Qdrant collection for provider");
            }
        }

        // Remove from cache
        {
            let mut cache = self.provider_cache.write().await;
            cache.remove(name);
        }

        // Remove rate limiter
        {
            let mut limiters = self.rate_limiters.write().await;
            limiters.remove(name);
        }

        info!(provider_name = %name, "Provider deleted");

        Ok(())
    }

    /// Check rate limit for a provider
    ///
    /// Returns Ok(()) if request can proceed, or Err with wait time in ms.
    pub async fn check_rate_limit(&self, provider_name: &str, tokens: u32) -> AppResult<()> {
        let mut limiters = self.rate_limiters.write().await;

        if let Some(limiter) = limiters.get_mut(provider_name) {
            match limiter.try_acquire(tokens) {
                Ok(()) => Ok(()),
                Err(wait_ms) => {
                    warn!(
                        provider_name = %provider_name,
                        wait_ms = wait_ms,
                        "Rate limit exceeded"
                    );
                    Err(AppError::RateLimitExceeded(provider_name.to_string()))
                }
            }
        } else {
            // No rate limiter configured, allow request
            Ok(())
        }
    }

    /// Get a cached provider config
    pub async fn get_cached_provider(&self, name: &str) -> Option<ProviderConfig> {
        let cache = self.provider_cache.read().await;
        cache.get(name).cloned()
    }

    /// Get the provider to use for a memory
    ///
    /// Returns the specified provider if given and enabled,
    /// otherwise returns the default provider.
    pub async fn resolve_provider(&self, provider_name: Option<&str>) -> AppResult<ProviderConfig> {
        if let Some(name) = provider_name {
            // Try to get specified provider
            if let Some(config) = self.get_cached_provider(name).await {
                if config.enabled {
                    return Ok(config);
                }
                return Err(AppError::ProviderDisabled(name.to_string()));
            }
            return Err(AppError::ProviderNotFound(name.to_string()));
        }

        // Get default provider
        let default = self.config_repo.get_default_provider().await?;
        Ok(default.to_provider_config())
    }

    /// Refresh the provider cache from database
    pub async fn refresh_cache(&self) -> AppResult<()> {
        debug!("Refreshing provider cache");

        let providers = self.config_repo.list_providers().await?;

        let mut cache = self.provider_cache.write().await;
        cache.clear();

        for record in providers {
            let config = record.to_provider_config();
            cache.insert(config.name.clone(), config);
        }

        info!(provider_count = cache.len(), "Provider cache refreshed");

        // Refresh LLM provider cache if repository is configured
        if let Some(ref llm_repo) = self.llm_repo {
            let llm_providers = llm_repo.list_providers().await?;

            let mut llm_cache = self.llm_provider_cache.write().await;
            llm_cache.clear();

            for record in llm_providers {
                let config = record.to_provider_config();
                llm_cache.insert(config.name.clone(), config);
            }

            info!(
                llm_provider_count = llm_cache.len(),
                "LLM provider cache refreshed"
            );
        }

        Ok(())
    }

    // =========================================================================
    // LLM Provider Management Methods
    // =========================================================================

    /// List all configured LLM providers
    pub async fn list_llm_providers(&self) -> AppResult<Vec<LlmProviderInfo>> {
        let llm_repo = self.llm_repo.as_ref().ok_or_else(|| {
            AppError::Config("LLM provider repository not configured".to_string())
        })?;

        let records = llm_repo.list_providers().await?;
        Ok(records.into_iter().map(Into::into).collect())
    }

    /// Get an LLM provider by name
    pub async fn get_llm_provider(&self, name: &str) -> AppResult<LlmProviderInfo> {
        let llm_repo = self.llm_repo.as_ref().ok_or_else(|| {
            AppError::Config("LLM provider repository not configured".to_string())
        })?;

        let record = llm_repo.get_provider(name).await?;
        Ok(record.into())
    }

    /// Get the default LLM provider
    pub async fn get_default_llm_provider(&self) -> AppResult<LlmProviderInfo> {
        let llm_repo = self.llm_repo.as_ref().ok_or_else(|| {
            AppError::Config("LLM provider repository not configured".to_string())
        })?;

        let record = llm_repo.get_default_provider().await?;
        Ok(record.into())
    }

    /// Get the default LLM provider name
    pub async fn get_default_llm_provider_name(&self) -> AppResult<Option<String>> {
        let llm_repo = match self.llm_repo.as_ref() {
            Some(repo) => repo,
            None => return Ok(None),
        };

        match llm_repo.get_default_provider().await {
            Ok(record) => Ok(Some(record.name)),
            Err(AppError::NoDefaultProvider) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Create a new LLM provider
    pub async fn create_llm_provider(
        &self,
        config: LlmProviderConfig,
        prompts: Option<LlmPromptConfig>,
    ) -> AppResult<LlmProviderInfo> {
        let llm_repo = self.llm_repo.as_ref().ok_or_else(|| {
            AppError::Config("LLM provider repository not configured".to_string())
        })?;

        debug!(provider_name = %config.name, "Creating LLM provider");

        let record = llm_repo.create_provider(&config, prompts.as_ref()).await?;

        // Update cache
        {
            let mut cache = self.llm_provider_cache.write().await;
            cache.insert(config.name.clone(), config.clone());
        }

        // Initialize rate limiter
        {
            let rate_limit = if config.rpm_limit.is_some() || config.tpm_limit.is_some() {
                Some(RateLimitConfig {
                    requests_per_minute: config.rpm_limit,
                    tokens_per_minute: config.tpm_limit,
                })
            } else {
                None
            };

            let mut limiters = self.llm_rate_limiters.write().await;
            limiters.insert(config.name.clone(), RateLimiterState::new(rate_limit));
        }

        info!(provider_name = %record.name, "LLM provider created");

        Ok(record.into())
    }

    /// Update an existing LLM provider
    pub async fn update_llm_provider(
        &self,
        name: &str,
        update: UpdateLlmProviderInput,
    ) -> AppResult<LlmProviderInfo> {
        let llm_repo = self.llm_repo.as_ref().ok_or_else(|| {
            AppError::Config("LLM provider repository not configured".to_string())
        })?;

        debug!(provider_name = %name, "Updating LLM provider");

        let record = llm_repo.update_provider(name, &update).await?;
        let config = record.to_provider_config();

        // Update cache
        {
            let mut cache = self.llm_provider_cache.write().await;
            cache.insert(name.to_string(), config.clone());
        }

        // Update rate limiter if rate limit changed
        if update.rpm_limit.is_some() || update.tpm_limit.is_some() {
            let rate_limit = if config.rpm_limit.is_some() || config.tpm_limit.is_some() {
                Some(RateLimitConfig {
                    requests_per_minute: config.rpm_limit,
                    tokens_per_minute: config.tpm_limit,
                })
            } else {
                None
            };

            let mut limiters = self.llm_rate_limiters.write().await;
            limiters.insert(name.to_string(), RateLimiterState::new(rate_limit));
        }

        info!(provider_name = %name, "LLM provider updated");

        Ok(record.into())
    }

    /// Enable or disable an LLM provider
    pub async fn set_llm_provider_enabled(
        &self,
        name: &str,
        enabled: bool,
    ) -> AppResult<LlmProviderInfo> {
        let llm_repo = self.llm_repo.as_ref().ok_or_else(|| {
            AppError::Config("LLM provider repository not configured".to_string())
        })?;

        debug!(provider_name = %name, enabled = enabled, "Setting LLM provider enabled state");

        let record = llm_repo.set_provider_enabled(name, enabled).await?;

        // Update cache
        {
            let mut cache = self.llm_provider_cache.write().await;
            if let Some(config) = cache.get_mut(name) {
                config.enabled = enabled;
            }
        }

        info!(provider_name = %name, enabled = enabled, "LLM provider enabled state updated");

        Ok(record.into())
    }

    /// Set the default LLM provider
    pub async fn set_default_llm_provider(&self, name: &str) -> AppResult<LlmProviderInfo> {
        let llm_repo = self.llm_repo.as_ref().ok_or_else(|| {
            AppError::Config("LLM provider repository not configured".to_string())
        })?;

        debug!(provider_name = %name, "Setting default LLM provider");

        let record = llm_repo.set_default_provider(name).await?;

        info!(provider_name = %name, "Default LLM provider set");

        Ok(record.into())
    }

    /// Delete an LLM provider
    pub async fn delete_llm_provider(&self, name: &str) -> AppResult<()> {
        let llm_repo = self.llm_repo.as_ref().ok_or_else(|| {
            AppError::Config("LLM provider repository not configured".to_string())
        })?;

        debug!(provider_name = %name, "Deleting LLM provider");

        llm_repo.delete_provider(name).await?;

        // Remove from cache
        {
            let mut cache = self.llm_provider_cache.write().await;
            cache.remove(name);
        }

        // Remove rate limiter
        {
            let mut limiters = self.llm_rate_limiters.write().await;
            limiters.remove(name);
        }

        info!(provider_name = %name, "LLM provider deleted");

        Ok(())
    }

    /// Check rate limit for an LLM provider
    ///
    /// Returns Ok(()) if request can proceed, or Err with wait time in ms.
    pub async fn check_llm_rate_limit(&self, provider_name: &str, tokens: u32) -> AppResult<()> {
        let mut limiters = self.llm_rate_limiters.write().await;

        if let Some(limiter) = limiters.get_mut(provider_name) {
            match limiter.try_acquire(tokens) {
                Ok(()) => Ok(()),
                Err(wait_ms) => {
                    warn!(
                        provider_name = %provider_name,
                        wait_ms = wait_ms,
                        "LLM rate limit exceeded"
                    );
                    Err(AppError::RateLimitExceeded(provider_name.to_string()))
                }
            }
        } else {
            // No rate limiter configured, allow request
            Ok(())
        }
    }

    /// Get a cached LLM provider config
    pub async fn get_cached_llm_provider(&self, name: &str) -> Option<LlmProviderConfig> {
        let cache = self.llm_provider_cache.read().await;
        cache.get(name).cloned()
    }

    /// Get the LLM provider to use for memory processing
    ///
    /// Returns the specified provider if given and enabled,
    /// otherwise returns the default provider.
    pub async fn resolve_llm_provider(
        &self,
        provider_name: Option<&str>,
    ) -> AppResult<LlmProviderConfig> {
        let llm_repo = self.llm_repo.as_ref().ok_or_else(|| {
            AppError::Config("LLM provider repository not configured".to_string())
        })?;

        if let Some(name) = provider_name {
            // Try to get specified provider
            if let Some(config) = self.get_cached_llm_provider(name).await {
                if config.enabled {
                    return Ok(config);
                }
                return Err(AppError::ProviderDisabled(name.to_string()));
            }
            return Err(AppError::ProviderNotFound(name.to_string()));
        }

        // Get default provider
        let default = llm_repo.get_default_provider().await?;
        Ok(default.to_provider_config())
    }

    /// Update prompt templates for an LLM provider
    pub async fn update_llm_prompts(
        &self,
        name: &str,
        compression_prompt: Option<String>,
        classification_prompt: Option<String>,
    ) -> AppResult<LlmProviderInfo> {
        let llm_repo = self.llm_repo.as_ref().ok_or_else(|| {
            AppError::Config("LLM provider repository not configured".to_string())
        })?;

        debug!(provider_name = %name, "Updating LLM provider prompts");

        let record = llm_repo
            .update_prompts(name, compression_prompt, classification_prompt)
            .await?;

        info!(provider_name = %name, "LLM provider prompts updated");

        Ok(record.into())
    }

    /// Check if any enabled LLM provider exists
    pub async fn has_enabled_llm_provider(&self) -> AppResult<bool> {
        let llm_repo = match self.llm_repo.as_ref() {
            Some(repo) => repo,
            None => return Ok(false),
        };

        llm_repo.has_enabled_provider().await
    }

    /// Get prompt configuration for an LLM provider
    pub async fn get_llm_prompts(&self, name: &str) -> AppResult<LlmPromptConfig> {
        let llm_repo = self.llm_repo.as_ref().ok_or_else(|| {
            AppError::Config("LLM provider repository not configured".to_string())
        })?;

        let record = llm_repo.get_provider(name).await?;
        Ok(record.get_prompt_config())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_state_no_limit() {
        let mut state = RateLimiterState::new(None);

        // Should always succeed with no limit
        assert!(state.try_acquire(100).is_ok());
        assert!(state.try_acquire(1000).is_ok());
        assert!(state.try_acquire(10000).is_ok());
    }

    #[test]
    fn test_rate_limiter_state_rpm_limit() {
        let config = RateLimitConfig {
            requests_per_minute: Some(2),
            tokens_per_minute: None,
        };
        let mut state = RateLimiterState::new(Some(config));

        // First two requests should succeed
        assert!(state.try_acquire(0).is_ok());
        assert!(state.try_acquire(0).is_ok());

        // Third request should fail
        assert!(state.try_acquire(0).is_err());
    }

    #[test]
    fn test_rate_limiter_state_tpm_limit() {
        let config = RateLimitConfig {
            requests_per_minute: None,
            tokens_per_minute: Some(100),
        };
        let mut state = RateLimiterState::new(Some(config));

        // Should succeed until token limit
        assert!(state.try_acquire(50).is_ok());
        assert!(state.try_acquire(40).is_ok());

        // Should fail when exceeding limit
        assert!(state.try_acquire(20).is_err());
    }

    #[test]
    fn test_provider_info_from_record() {
        use chrono::Utc;

        let record = EmbeddingProviderRecord {
            name: "test-provider".to_string(),
            provider_type: ProviderType::OpenAI,
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key_encrypted: None,
            model: "text-embedding-3-small".to_string(),
            dimension: 1536,
            enabled: true,
            is_default: true,
            rpm_limit: Some(500),
            tpm_limit: Some(1000000),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let info: ProviderInfo = record.into();

        assert_eq!(info.name, "test-provider");
        assert_eq!(info.provider_type, ProviderType::OpenAI);
        assert!(info.enabled);
        assert!(info.is_default);
        assert_eq!(info.dimension, 1536);
        assert!(info.rate_limit.is_some());

        let rate_limit = info.rate_limit.unwrap();
        assert_eq!(rate_limit.requests_per_minute, Some(500));
        assert_eq!(rate_limit.tokens_per_minute, Some(1000000));
    }

    #[test]
    fn test_llm_provider_info_from_record() {
        use chrono::Utc;

        let record = LlmProviderRecord {
            name: "test-llm".to_string(),
            provider_type: LlmProviderType::OpenAI,
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key_encrypted: None,
            model: "gpt-4o-mini".to_string(),
            enabled: true,
            is_default: true,
            rpm_limit: Some(500),
            tpm_limit: Some(100000),
            compression_prompt: Some("Compress: {content}".to_string()),
            classification_prompt: None,
            max_input_tokens: 4000,
            max_output_tokens: 1000,
            temperature: 0.3,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let info: LlmProviderInfo = record.into();

        assert_eq!(info.name, "test-llm");
        assert_eq!(info.provider_type, LlmProviderType::OpenAI);
        assert!(info.enabled);
        assert!(info.is_default);
        assert_eq!(info.model, "gpt-4o-mini");
        assert_eq!(info.rpm_limit, Some(500));
        assert_eq!(info.tpm_limit, Some(100000));
        assert_eq!(info.max_input_tokens, 4000);
        assert_eq!(info.max_output_tokens, 1000);
        assert!((info.temperature - 0.3).abs() < f32::EPSILON);
        assert!(info.has_compression_prompt);
        assert!(!info.has_classification_prompt);
    }

    #[test]
    fn test_llm_provider_info_no_prompts() {
        use chrono::Utc;

        let record = LlmProviderRecord {
            name: "local-llm".to_string(),
            provider_type: LlmProviderType::Local,
            endpoint: "http://localhost:11434".to_string(),
            api_key_encrypted: None,
            model: "llama2".to_string(),
            enabled: true,
            is_default: false,
            rpm_limit: None,
            tpm_limit: None,
            compression_prompt: None,
            classification_prompt: None,
            max_input_tokens: 4000,
            max_output_tokens: 1000,
            temperature: 0.7,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let info: LlmProviderInfo = record.into();

        assert_eq!(info.name, "local-llm");
        assert_eq!(info.provider_type, LlmProviderType::Local);
        assert!(info.rpm_limit.is_none());
        assert!(info.tpm_limit.is_none());
        assert!(!info.has_compression_prompt);
        assert!(!info.has_classification_prompt);
    }
}
