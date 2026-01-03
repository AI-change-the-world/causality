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

use crate::embedding::{ProviderConfig, ProviderType};
use crate::error::{AppError, AppResult};
use crate::llm::{LlmProviderConfig, LlmProviderType};
use crate::repository::{
    ConfigRepository, EmbeddingProviderRecord, LlmProviderRecord, LlmProviderRepository,
    QdrantRepository, UpdateLlmProviderInput, UpdateProviderInput,
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
    /// Whether this is the default provider
    pub is_default: bool,
}

impl From<EmbeddingProviderRecord> for ProviderInfo {
    fn from(record: EmbeddingProviderRecord) -> Self {
        ProviderInfo {
            name: record.name,
            provider_type: record.provider_type,
            enabled: record.enabled,
            model: record.model,
            dimension: record.dimension as usize,
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
}

impl From<LlmProviderRecord> for LlmProviderInfo {
    fn from(record: LlmProviderRecord) -> Self {
        LlmProviderInfo {
            name: record.name,
            provider_type: record.provider_type,
            enabled: record.enabled,
            model: record.model,
            is_default: record.is_default,
        }
    }
}

/// ConfigCenter service for provider management
#[derive(Clone)]
pub struct ConfigCenter {
    config_repo: ConfigRepository,
    llm_repo: Option<LlmProviderRepository>,
    qdrant_repo: Option<QdrantRepository>,
    /// Cached embedding provider configs for hot access
    provider_cache: Arc<RwLock<HashMap<String, ProviderConfig>>>,
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
            provider_cache: Arc::new(RwLock::new(HashMap::new())),
            llm_provider_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new ConfigCenter service with LLM provider support
    pub fn with_llm_repo(config_repo: ConfigRepository, llm_repo: LlmProviderRepository) -> Self {
        Self {
            config_repo,
            llm_repo: Some(llm_repo),
            qdrant_repo: None,
            provider_cache: Arc::new(RwLock::new(HashMap::new())),
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
            provider_cache: Arc::new(RwLock::new(HashMap::new())),
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

        for record in &providers {
            let config = record.to_provider_config();
            cache.insert(config.name.clone(), config);
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

            for record in llm_providers {
                let config = record.to_provider_config();
                llm_cache.insert(config.name.clone(), config);
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
    ) -> AppResult<ProviderInfo> {
        debug!(provider_name = %name, "Updating provider");

        let update = UpdateProviderInput {
            endpoint,
            api_key,
            model,
            enabled,
        };

        let record = self.config_repo.update_provider(name, &update).await?;
        let config = record.to_provider_config();

        // Update cache
        {
            let mut cache = self.provider_cache.write().await;
            cache.insert(name.to_string(), config.clone());
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

        info!(provider_name = %name, "Provider deleted");

        Ok(())
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
    ) -> AppResult<LlmProviderInfo> {
        let llm_repo = self.llm_repo.as_ref().ok_or_else(|| {
            AppError::Config("LLM provider repository not configured".to_string())
        })?;

        debug!(provider_name = %config.name, "Creating LLM provider");

        let record = llm_repo.create_provider(&config).await?;

        // Update cache
        {
            let mut cache = self.llm_provider_cache.write().await;
            cache.insert(config.name.clone(), config.clone());
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

        info!(provider_name = %name, "LLM provider deleted");

        Ok(())
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

    /// Check if any enabled LLM provider exists
    pub async fn has_enabled_llm_provider(&self) -> AppResult<bool> {
        let llm_repo = match self.llm_repo.as_ref() {
            Some(repo) => repo,
            None => return Ok(false),
        };

        llm_repo.has_enabled_provider().await
    }

    // =========================================================================
    // Embedding Generation Methods
    // =========================================================================

    /// Generate embedding for content and store it in Qdrant
    ///
    /// This method:
    /// 1. Resolves the embedding provider (specified or default)
    /// 2. Creates the appropriate embedding client (OpenAI or Local)
    /// 3. Generates the embedding vector
    /// 4. Stores the vector in Qdrant
    ///
    /// Returns the provider name used for embedding.
    pub async fn generate_and_store_embedding(
        &self,
        memory_id: uuid::Uuid,
        content: &str,
        provider_name: Option<&str>,
        payload: crate::repository::VectorPayload,
    ) -> AppResult<String> {
        use crate::embedding::{
            EmbeddingProvider, EmbeddingRequest, LocalProvider, OpenAIProvider,
        };

        // Resolve provider
        let config = self.resolve_provider(provider_name).await?;
        let resolved_name = config.name.clone();

        debug!(
            memory_id = %memory_id,
            provider = %resolved_name,
            content_len = content.len(),
            "Generating embedding"
        );

        // Create embedding provider based on type
        let embedding = match config.provider_type {
            crate::embedding::ProviderType::OpenAI | crate::embedding::ProviderType::Azure => {
                let provider = OpenAIProvider::new(config.clone()).map_err(|e| {
                    AppError::Internal(format!("Failed to create OpenAI provider: {}", e))
                })?;

                let request = EmbeddingRequest::new(content);
                provider.embed(request).await.map_err(|e| {
                    AppError::Internal(format!("Embedding generation failed: {}", e))
                })?
            }
            crate::embedding::ProviderType::Local => {
                let provider = LocalProvider::new(config.clone()).map_err(|e| {
                    AppError::Internal(format!("Failed to create local provider: {}", e))
                })?;

                let request = EmbeddingRequest::new(content);
                provider.embed(request).await.map_err(|e| {
                    AppError::Internal(format!("Embedding generation failed: {}", e))
                })?
            }
        };

        debug!(
            memory_id = %memory_id,
            dimension = embedding.embedding.len(),
            "Embedding generated"
        );

        // Store in Qdrant
        let qdrant = self
            .qdrant_repo
            .as_ref()
            .ok_or_else(|| AppError::Internal("Qdrant repository not configured".to_string()))?;

        qdrant
            .upsert_vector(&resolved_name, memory_id, embedding.embedding, payload)
            .await?;

        info!(
            memory_id = %memory_id,
            provider = %resolved_name,
            "Embedding stored in Qdrant"
        );

        Ok(resolved_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let info: ProviderInfo = record.into();

        assert_eq!(info.name, "test-provider");
        assert_eq!(info.provider_type, ProviderType::OpenAI);
        assert!(info.enabled);
        assert!(info.is_default);
        assert_eq!(info.dimension, 1536);
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let info: LlmProviderInfo = record.into();

        assert_eq!(info.name, "test-llm");
        assert_eq!(info.provider_type, LlmProviderType::OpenAI);
        assert!(info.enabled);
        assert!(info.is_default);
        assert_eq!(info.model, "gpt-4o-mini");
    }

    #[test]
    fn test_llm_provider_info_no_api_key() {
        use chrono::Utc;

        let record = LlmProviderRecord {
            name: "local-llm".to_string(),
            provider_type: LlmProviderType::Local,
            endpoint: "http://localhost:11434".to_string(),
            api_key_encrypted: None,
            model: "llama2".to_string(),
            enabled: true,
            is_default: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let info: LlmProviderInfo = record.into();

        assert_eq!(info.name, "local-llm");
        assert_eq!(info.provider_type, LlmProviderType::Local);
    }
}
