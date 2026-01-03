//! LLM Provider Repository implementation
//!
//! Provides CRUD operations for LLM provider configurations with PostgreSQL.
//! Only stores essential connection parameters - processing parameters are internal.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use tracing::{debug, info};

use crate::error::{AppError, AppResult};
use crate::llm::{LlmProviderConfig, LlmProviderType};

/// Repository for LLM provider configuration operations
#[derive(Clone)]
pub struct LlmProviderRepository {
    pool: PgPool,
}

impl LlmProviderRepository {
    /// Create a new LlmProviderRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new LLM provider configuration
    pub async fn create_provider(
        &self,
        config: &LlmProviderConfig,
    ) -> AppResult<LlmProviderRecord> {
        debug!(
            provider_name = %config.name,
            provider_type = %config.provider_type,
            "Creating LLM provider"
        );

        // Encrypt API key if provided (for now, we store it as-is in bytes)
        // In production, use proper encryption
        let api_key_encrypted = config.api_key.as_ref().map(|k| k.as_bytes().to_vec());

        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            INSERT INTO llm_providers (
                name, provider_type, endpoint, api_key_encrypted, model,
                enabled, is_default,
                created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, false, NOW(), NOW())
            RETURNING
                name, provider_type, endpoint, api_key_encrypted, model,
                enabled, is_default,
                created_at, updated_at
            "#,
        )
        .bind(&config.name)
        .bind(&config.provider_type)
        .bind(&config.endpoint)
        .bind(&api_key_encrypted)
        .bind(&config.model)
        .bind(config.enabled)
        .fetch_one(&self.pool)
        .await?;

        info!(provider_name = %config.name, "LLM provider created");
        Ok(row.into())
    }

    /// Get an LLM provider by name
    pub async fn get_provider(&self, name: &str) -> AppResult<LlmProviderRecord> {
        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            SELECT
                name, provider_type, endpoint, api_key_encrypted, model,
                enabled, is_default,
                created_at, updated_at
            FROM llm_providers
            WHERE name = $1
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| AppError::ProviderNotFound(name.to_string()))?;

        Ok(row.into())
    }

    /// List all LLM providers
    pub async fn list_providers(&self) -> AppResult<Vec<LlmProviderRecord>> {
        let rows = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            SELECT
                name, provider_type, endpoint, api_key_encrypted, model,
                enabled, is_default,
                created_at, updated_at
            FROM llm_providers
            ORDER BY name
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Update an LLM provider configuration
    pub async fn update_provider(
        &self,
        name: &str,
        update: &UpdateLlmProviderInput,
    ) -> AppResult<LlmProviderRecord> {
        debug!(provider_name = %name, "Updating LLM provider");

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

        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            UPDATE llm_providers
            SET endpoint = $2,
                api_key_encrypted = $3,
                model = $4,
                enabled = $5,
                updated_at = NOW()
            WHERE name = $1
            RETURNING
                name, provider_type, endpoint, api_key_encrypted, model,
                enabled, is_default,
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

        info!(provider_name = %name, "LLM provider updated");
        Ok(row.into())
    }

    /// Delete an LLM provider
    pub async fn delete_provider(&self, name: &str) -> AppResult<()> {
        debug!(provider_name = %name, "Deleting LLM provider");

        let result = sqlx::query(
            r#"
            DELETE FROM llm_providers
            WHERE name = $1
            "#,
        )
        .bind(name)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(AppError::ProviderNotFound(name.to_string()));
        }

        info!(provider_name = %name, "LLM provider deleted");
        Ok(())
    }

    /// Set a provider as the default
    pub async fn set_default_provider(&self, name: &str) -> AppResult<LlmProviderRecord> {
        debug!(provider_name = %name, "Setting default LLM provider");

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
            UPDATE llm_providers
            SET is_default = false
            WHERE is_default = true
            "#,
        )
        .execute(&mut *tx)
        .await?;

        // Set new default
        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            UPDATE llm_providers
            SET is_default = true, updated_at = NOW()
            WHERE name = $1
            RETURNING
                name, provider_type, endpoint, api_key_encrypted, model,
                enabled, is_default,
                created_at, updated_at
            "#,
        )
        .bind(name)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;

        info!(provider_name = %name, "Default LLM provider set");
        Ok(row.into())
    }

    /// Get the default LLM provider
    pub async fn get_default_provider(&self) -> AppResult<LlmProviderRecord> {
        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            SELECT
                name, provider_type, endpoint, api_key_encrypted, model,
                enabled, is_default,
                created_at, updated_at
            FROM llm_providers
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
    ) -> AppResult<LlmProviderRecord> {
        debug!(provider_name = %name, enabled = enabled, "Setting LLM provider enabled state");

        // If disabling the default provider, we need to check
        if !enabled {
            let provider = self.get_provider(name).await?;
            if provider.is_default {
                return Err(AppError::Validation(
                    "Cannot disable the default LLM provider. Set another provider as default first."
                        .to_string(),
                ));
            }
        }

        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            UPDATE llm_providers
            SET enabled = $2, updated_at = NOW()
            WHERE name = $1
            RETURNING
                name, provider_type, endpoint, api_key_encrypted, model,
                enabled, is_default,
                created_at, updated_at
            "#,
        )
        .bind(name)
        .bind(enabled)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| AppError::ProviderNotFound(name.to_string()))?;

        info!(provider_name = %name, enabled = enabled, "LLM provider enabled state updated");
        Ok(row.into())
    }

    /// Check if any enabled LLM provider exists
    pub async fn has_enabled_provider(&self) -> AppResult<bool> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM llm_providers WHERE enabled = true
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(count.0 > 0)
    }
}

/// Input for updating an LLM provider
#[derive(Debug, Clone, Default)]
pub struct UpdateLlmProviderInput {
    /// New endpoint URL
    pub endpoint: Option<String>,
    /// New API key
    pub api_key: Option<String>,
    /// New model name
    pub model: Option<String>,
    /// Enable/disable the provider
    pub enabled: Option<bool>,
}

/// LLM provider record from database
#[derive(Debug, Clone)]
pub struct LlmProviderRecord {
    /// Provider name
    pub name: String,
    /// Provider type
    pub provider_type: LlmProviderType,
    /// API endpoint
    pub endpoint: String,
    /// Encrypted API key
    pub api_key_encrypted: Option<Vec<u8>>,
    /// Model name
    pub model: String,
    /// Whether the provider is enabled
    pub enabled: bool,
    /// Whether this is the default provider
    pub is_default: bool,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl LlmProviderRecord {
    /// Convert to LlmProviderConfig
    pub fn to_provider_config(&self) -> LlmProviderConfig {
        let api_key = self
            .api_key_encrypted
            .as_ref()
            .and_then(|bytes| String::from_utf8(bytes.clone()).ok());

        LlmProviderConfig {
            name: self.name.clone(),
            provider_type: self.provider_type,
            endpoint: self.endpoint.clone(),
            api_key,
            model: self.model.clone(),
            enabled: self.enabled,
        }
    }
}

/// Internal row type for sqlx mapping
#[derive(Debug, FromRow)]
struct LlmProviderRow {
    name: String,
    provider_type: LlmProviderType,
    endpoint: String,
    api_key_encrypted: Option<Vec<u8>>,
    model: String,
    enabled: bool,
    is_default: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<LlmProviderRow> for LlmProviderRecord {
    fn from(row: LlmProviderRow) -> Self {
        LlmProviderRecord {
            name: row.name,
            provider_type: row.provider_type,
            endpoint: row.endpoint,
            api_key_encrypted: row.api_key_encrypted,
            model: row.model,
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
    fn test_llm_provider_record_to_config() {
        let record = LlmProviderRecord {
            name: "test-llm".to_string(),
            provider_type: LlmProviderType::OpenAI,
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key_encrypted: Some("sk-test".as_bytes().to_vec()),
            model: "gpt-4o-mini".to_string(),
            enabled: true,
            is_default: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let config = record.to_provider_config();

        assert_eq!(config.name, "test-llm");
        assert_eq!(config.provider_type, LlmProviderType::OpenAI);
        assert_eq!(config.endpoint, "https://api.openai.com/v1");
        assert_eq!(config.api_key, Some("sk-test".to_string()));
        assert_eq!(config.model, "gpt-4o-mini");
        assert!(config.enabled);
    }

    #[test]
    fn test_llm_provider_record_to_config_no_api_key() {
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

        let config = record.to_provider_config();

        assert_eq!(config.name, "local-llm");
        assert_eq!(config.provider_type, LlmProviderType::Local);
        assert!(config.api_key.is_none());
    }

    #[test]
    fn test_update_llm_provider_input_default() {
        let input = UpdateLlmProviderInput::default();

        assert!(input.endpoint.is_none());
        assert!(input.api_key.is_none());
        assert!(input.model.is_none());
        assert!(input.enabled.is_none());
    }
}
