//! Config Repository implementation
//!
//! Provides CRUD operations for embedding provider configurations with PostgreSQL.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use tracing::{debug, info};

use crate::embedding::{ProviderConfig, ProviderType};
use crate::error::{AppError, AppResult};

/// Repository for embedding provider configuration operations
#[derive(Clone)]
pub struct ConfigRepository {
    pool: PgPool,
}

impl ConfigRepository {
    /// Create a new ConfigRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new embedding provider configuration
    pub async fn create_provider(
        &self,
        config: &ProviderConfig,
    ) -> AppResult<EmbeddingProviderRecord> {
        debug!(
            provider_name = %config.name,
            provider_type = %config.provider_type,
            "Creating embedding provider"
        );

        // Encrypt API key if provided (for now, we store it as-is in bytes)
        // In production, use proper encryption
        let api_key_encrypted = config.api_key.as_ref().map(|k| k.as_bytes().to_vec());

        let row = sqlx::query_as::<_, EmbeddingProviderRow>(
            r#"
            INSERT INTO embedding_providers (
                name, provider_type, endpoint, api_key_encrypted, model,
                dimension, enabled, is_default,
                created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, false, NOW(), NOW())
            RETURNING
                name, provider_type, endpoint, api_key_encrypted, model,
                dimension, enabled, is_default,
                created_at, updated_at
            "#,
        )
        .bind(&config.name)
        .bind(&config.provider_type)
        .bind(&config.endpoint)
        .bind(&api_key_encrypted)
        .bind(&config.model)
        .bind(config.dimension as i32)
        .bind(config.enabled)
        .fetch_one(&self.pool)
        .await?;

        info!(provider_name = %config.name, "Embedding provider created");
        Ok(row.into())
    }

    /// Get an embedding provider by name
    pub async fn get_provider(&self, name: &str) -> AppResult<EmbeddingProviderRecord> {
        let row = sqlx::query_as::<_, EmbeddingProviderRow>(
            r#"
            SELECT
                name, provider_type, endpoint, api_key_encrypted, model,
                dimension, enabled, is_default,
                created_at, updated_at
            FROM embedding_providers
            WHERE name = $1
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| AppError::ProviderNotFound(name.to_string()))?;

        Ok(row.into())
    }

    /// List all embedding providers
    pub async fn list_providers(&self) -> AppResult<Vec<EmbeddingProviderRecord>> {
        let rows = sqlx::query_as::<_, EmbeddingProviderRow>(
            r#"
            SELECT
                name, provider_type, endpoint, api_key_encrypted, model,
                dimension, enabled, is_default,
                created_at, updated_at
            FROM embedding_providers
            ORDER BY name
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Update an embedding provider configuration
    pub async fn update_provider(
        &self,
        name: &str,
        update: &UpdateProviderInput,
    ) -> AppResult<EmbeddingProviderRecord> {
        debug!(provider_name = %name, "Updating embedding provider");

        // First check if provider exists
        let existing = self.get_provider(name).await?;

        // Build update values
        let endpoint = update.endpoint.as_ref().unwrap_or(&existing.endpoint);
        let model = update.model.as_ref().unwrap_or(&existing.model);
        let enabled = update.enabled.unwrap_or(existing.enabled);

        // Handle API key update
        let api_key_encrypted = if let Some(ref key) = update.api_key {
            Some(key.as_bytes().to_vec())
        } else {
            existing.api_key_encrypted
        };

        let row = sqlx::query_as::<_, EmbeddingProviderRow>(
            r#"
            UPDATE embedding_providers
            SET endpoint = $2,
                api_key_encrypted = $3,
                model = $4,
                enabled = $5,
                updated_at = NOW()
            WHERE name = $1
            RETURNING
                name, provider_type, endpoint, api_key_encrypted, model,
                dimension, enabled, is_default,
                created_at, updated_at
            "#,
        )
        .bind(name)
        .bind(endpoint)
        .bind(&api_key_encrypted)
        .bind(model)
        .bind(enabled)
        .fetch_one(&self.pool)
        .await?;

        info!(provider_name = %name, "Embedding provider updated");
        Ok(row.into())
    }

    /// Delete an embedding provider
    pub async fn delete_provider(&self, name: &str) -> AppResult<()> {
        debug!(provider_name = %name, "Deleting embedding provider");

        let result = sqlx::query(
            r#"
            DELETE FROM embedding_providers
            WHERE name = $1
            "#,
        )
        .bind(name)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(AppError::ProviderNotFound(name.to_string()));
        }

        info!(provider_name = %name, "Embedding provider deleted");
        Ok(())
    }

    /// Set a provider as the default
    pub async fn set_default_provider(&self, name: &str) -> AppResult<EmbeddingProviderRecord> {
        debug!(provider_name = %name, "Setting default embedding provider");

        // First check if provider exists and is enabled
        let provider = self.get_provider(name).await?;
        if !provider.enabled {
            return Err(AppError::ProviderDisabled(name.to_string()));
        }

        // Use a transaction to ensure atomicity
        let mut tx = self.pool.begin().await?;

        // Clear existing default
        sqlx::query(
            r#"
            UPDATE embedding_providers
            SET is_default = false
            WHERE is_default = true
            "#,
        )
        .execute(&mut *tx)
        .await?;

        // Set new default
        let row = sqlx::query_as::<_, EmbeddingProviderRow>(
            r#"
            UPDATE embedding_providers
            SET is_default = true, updated_at = NOW()
            WHERE name = $1
            RETURNING
                name, provider_type, endpoint, api_key_encrypted, model,
                dimension, enabled, is_default,
                created_at, updated_at
            "#,
        )
        .bind(name)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;

        info!(provider_name = %name, "Default embedding provider set");
        Ok(row.into())
    }

    /// Get the default embedding provider
    pub async fn get_default_provider(&self) -> AppResult<EmbeddingProviderRecord> {
        let row = sqlx::query_as::<_, EmbeddingProviderRow>(
            r#"
            SELECT
                name, provider_type, endpoint, api_key_encrypted, model,
                dimension, enabled, is_default,
                created_at, updated_at
            FROM embedding_providers
            WHERE is_default = true AND enabled = true
            "#,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::NoDefaultProvider)?;

        Ok(row.into())
    }

    /// Enable or disable a provider
    pub async fn set_provider_enabled(
        &self,
        name: &str,
        enabled: bool,
    ) -> AppResult<EmbeddingProviderRecord> {
        debug!(provider_name = %name, enabled = enabled, "Setting provider enabled state");

        // If disabling the default provider, we need to check
        if !enabled {
            let provider = self.get_provider(name).await?;
            if provider.is_default {
                return Err(AppError::Validation(
                    "Cannot disable the default provider. Set another provider as default first."
                        .to_string(),
                ));
            }
        }

        let row = sqlx::query_as::<_, EmbeddingProviderRow>(
            r#"
            UPDATE embedding_providers
            SET enabled = $2, updated_at = NOW()
            WHERE name = $1
            RETURNING
                name, provider_type, endpoint, api_key_encrypted, model,
                dimension, enabled, is_default,
                created_at, updated_at
            "#,
        )
        .bind(name)
        .bind(enabled)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| AppError::ProviderNotFound(name.to_string()))?;

        info!(provider_name = %name, enabled = enabled, "Provider enabled state updated");
        Ok(row.into())
    }

    /// Check if any enabled provider exists
    pub async fn has_enabled_provider(&self) -> AppResult<bool> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM embedding_providers WHERE enabled = true
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(count.0 > 0)
    }
}

/// Input for updating a provider
#[derive(Debug, Clone, Default)]
pub struct UpdateProviderInput {
    /// New endpoint URL
    pub endpoint: Option<String>,
    /// New API key
    pub api_key: Option<String>,
    /// New model name
    pub model: Option<String>,
    /// Enable/disable the provider
    pub enabled: Option<bool>,
}

/// Embedding provider record from database
#[derive(Debug, Clone)]
pub struct EmbeddingProviderRecord {
    /// Provider name
    pub name: String,
    /// Provider type
    pub provider_type: ProviderType,
    /// API endpoint
    pub endpoint: String,
    /// Encrypted API key
    pub api_key_encrypted: Option<Vec<u8>>,
    /// Model name
    pub model: String,
    /// Embedding dimension
    pub dimension: i32,
    /// Whether the provider is enabled
    pub enabled: bool,
    /// Whether this is the default provider
    pub is_default: bool,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl EmbeddingProviderRecord {
    /// Convert to ProviderConfig
    pub fn to_provider_config(&self) -> ProviderConfig {
        let api_key = self
            .api_key_encrypted
            .as_ref()
            .and_then(|bytes| String::from_utf8(bytes.clone()).ok());

        ProviderConfig {
            name: self.name.clone(),
            provider_type: self.provider_type,
            endpoint: self.endpoint.clone(),
            api_key,
            model: self.model.clone(),
            dimension: self.dimension as usize,
            enabled: self.enabled,
        }
    }
}

/// Internal row type for sqlx mapping
#[derive(Debug, FromRow)]
struct EmbeddingProviderRow {
    name: String,
    provider_type: ProviderType,
    endpoint: String,
    api_key_encrypted: Option<Vec<u8>>,
    model: String,
    dimension: i32,
    enabled: bool,
    is_default: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<EmbeddingProviderRow> for EmbeddingProviderRecord {
    fn from(row: EmbeddingProviderRow) -> Self {
        EmbeddingProviderRecord {
            name: row.name,
            provider_type: row.provider_type,
            endpoint: row.endpoint,
            api_key_encrypted: row.api_key_encrypted,
            model: row.model,
            dimension: row.dimension,
            enabled: row.enabled,
            is_default: row.is_default,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_provider_record_to_config() {
        let record = EmbeddingProviderRecord {
            name: "test-provider".to_string(),
            provider_type: ProviderType::OpenAI,
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key_encrypted: Some("sk-test".as_bytes().to_vec()),
            model: "text-embedding-3-small".to_string(),
            dimension: 1536,
            enabled: true,
            is_default: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let config = record.to_provider_config();

        assert_eq!(config.name, "test-provider");
        assert_eq!(config.provider_type, ProviderType::OpenAI);
        assert_eq!(config.endpoint, "https://api.openai.com/v1");
        assert_eq!(config.api_key, Some("sk-test".to_string()));
        assert_eq!(config.model, "text-embedding-3-small");
        assert_eq!(config.dimension, 1536);
        assert!(config.enabled);
    }

    #[test]
    fn test_embedding_provider_record_to_config_no_rate_limit() {
        let record = EmbeddingProviderRecord {
            name: "local-provider".to_string(),
            provider_type: ProviderType::Local,
            endpoint: "http://localhost:8000".to_string(),
            api_key_encrypted: None,
            model: "bge-large".to_string(),
            dimension: 1024,
            enabled: true,
            is_default: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let config = record.to_provider_config();

        assert_eq!(config.name, "local-provider");
        assert_eq!(config.provider_type, ProviderType::Local);
        assert!(config.api_key.is_none());
    }

    #[test]
    fn test_update_provider_input_default() {
        let input = UpdateProviderInput::default();

        assert!(input.endpoint.is_none());
        assert!(input.api_key.is_none());
        assert!(input.model.is_none());
        assert!(input.enabled.is_none());
    }
}
