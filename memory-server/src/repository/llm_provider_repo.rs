//! LLM Provider Repository implementation
//!
//! Provides CRUD operations for LLM provider configurations with PostgreSQL.
//! Used for memory processing: compression, classification, and tag extraction.

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
        prompts: Option<&LlmPromptConfig>,
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
                compression_prompt, classification_prompt,
                max_input_tokens, max_output_tokens, temperature,
                created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, false, $7, $8, $9, $10, $11, NOW(), NOW())
            RETURNING
                name, provider_type, endpoint, api_key_encrypted, model,
                enabled, is_default,
                compression_prompt, classification_prompt,
                max_input_tokens, max_output_tokens, temperature,
                created_at, updated_at
            "#,
        )
        .bind(&config.name)
        .bind(&config.provider_type)
        .bind(&config.endpoint)
        .bind(&api_key_encrypted)
        .bind(&config.model)
        .bind(config.enabled)
        .bind(prompts.and_then(|p| p.compression_prompt.as_ref()))
        .bind(prompts.and_then(|p| p.classification_prompt.as_ref()))
        .bind(config.max_input_tokens as i32)
        .bind(config.max_output_tokens as i32)
        .bind(config.temperature)
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
                compression_prompt, classification_prompt,
                max_input_tokens, max_output_tokens, temperature,
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
                compression_prompt, classification_prompt,
                max_input_tokens, max_output_tokens, temperature,
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
        let compression_prompt = update
            .compression_prompt
            .as_ref()
            .or(existing.compression_prompt.as_ref());
        let classification_prompt = update
            .classification_prompt
            .as_ref()
            .or(existing.classification_prompt.as_ref());
        let max_input_tokens = update
            .max_input_tokens
            .map(|v| v as i32)
            .unwrap_or(existing.max_input_tokens);
        let max_output_tokens = update
            .max_output_tokens
            .map(|v| v as i32)
            .unwrap_or(existing.max_output_tokens);
        let temperature = update.temperature.unwrap_or(existing.temperature);

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
                compression_prompt = $6,
                classification_prompt = $7,
                max_input_tokens = $8,
                max_output_tokens = $9,
                temperature = $10,
                updated_at = NOW()
            WHERE name = $1
            RETURNING
                name, provider_type, endpoint, api_key_encrypted, model,
                enabled, is_default,
                compression_prompt, classification_prompt,
                max_input_tokens, max_output_tokens, temperature,
                created_at, updated_at
            "#,
        )
        .bind(name)
        .bind(endpoint)
        .bind(&api_key_encrypted)
        .bind(model)
        .bind(enabled)
        .bind(compression_prompt)
        .bind(classification_prompt)
        .bind(max_input_tokens)
        .bind(max_output_tokens)
        .bind(temperature)
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
                compression_prompt, classification_prompt,
                max_input_tokens, max_output_tokens, temperature,
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
                compression_prompt, classification_prompt,
                max_input_tokens, max_output_tokens, temperature,
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
                compression_prompt, classification_prompt,
                max_input_tokens, max_output_tokens, temperature,
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

    /// Update prompt templates for a provider
    pub async fn update_prompts(
        &self,
        name: &str,
        compression_prompt: Option<String>,
        classification_prompt: Option<String>,
    ) -> AppResult<LlmProviderRecord> {
        debug!(provider_name = %name, "Updating LLM provider prompts");

        let existing = self.get_provider(name).await?;

        let compression = compression_prompt.or(existing.compression_prompt);
        let classification = classification_prompt.or(existing.classification_prompt);

        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            UPDATE llm_providers
            SET compression_prompt = $2,
                classification_prompt = $3,
                updated_at = NOW()
            WHERE name = $1
            RETURNING
                name, provider_type, endpoint, api_key_encrypted, model,
                enabled, is_default, rpm_limit, tpm_limit,
                compression_prompt, classification_prompt,
                max_input_tokens, max_output_tokens, temperature,
                created_at, updated_at
            "#,
        )
        .bind(name)
        .bind(&compression)
        .bind(&classification)
        .fetch_one(&self.pool)
        .await?;

        info!(provider_name = %name, "LLM provider prompts updated");
        Ok(row.into())
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
    /// Requests per minute limit
    pub rpm_limit: Option<u32>,
    /// Tokens per minute limit
    pub tpm_limit: Option<u32>,
    /// Compression prompt template
    pub compression_prompt: Option<String>,
    /// Classification prompt template
    pub classification_prompt: Option<String>,
    /// Maximum input tokens
    pub max_input_tokens: Option<u32>,
    /// Maximum output tokens
    pub max_output_tokens: Option<u32>,
    /// Temperature for generation
    pub temperature: Option<f32>,
}

/// Prompt configuration for LLM processing
#[derive(Debug, Clone, Default)]
pub struct LlmPromptConfig {
    /// Prompt template for memory compression
    pub compression_prompt: Option<String>,
    /// Prompt template for memory classification
    pub classification_prompt: Option<String>,
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
    /// Compression prompt template
    pub compression_prompt: Option<String>,
    /// Classification prompt template
    pub classification_prompt: Option<String>,
    /// Maximum input tokens
    pub max_input_tokens: i32,
    /// Maximum output tokens
    pub max_output_tokens: i32,
    /// Temperature for generation
    pub temperature: f32,
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
            max_input_tokens: self.max_input_tokens as u32,
            max_output_tokens: self.max_output_tokens as u32,
            temperature: self.temperature,
        }
    }

    /// Get prompt configuration
    pub fn get_prompt_config(&self) -> LlmPromptConfig {
        LlmPromptConfig {
            compression_prompt: self.compression_prompt.clone(),
            classification_prompt: self.classification_prompt.clone(),
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
    compression_prompt: Option<String>,
    classification_prompt: Option<String>,
    max_input_tokens: i32,
    max_output_tokens: i32,
    temperature: f32,
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
            compression_prompt: row.compression_prompt,
            classification_prompt: row.classification_prompt,
            max_input_tokens: row.max_input_tokens,
            max_output_tokens: row.max_output_tokens,
            temperature: row.temperature,
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
            compression_prompt: Some("Compress this: {content}".to_string()),
            classification_prompt: Some("Classify this: {content}".to_string()),
            max_input_tokens: 4000,
            max_output_tokens: 1000,
            temperature: 0.3,
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
        assert_eq!(config.max_input_tokens, 4000);
        assert_eq!(config.max_output_tokens, 1000);
        assert!((config.temperature - 0.3).abs() < f32::EPSILON);
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
            compression_prompt: None,
            classification_prompt: None,
            max_input_tokens: 4000,
            max_output_tokens: 1000,
            temperature: 0.3,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let config = record.to_provider_config();

        assert_eq!(config.name, "local-llm");
        assert_eq!(config.provider_type, LlmProviderType::Local);
        assert!(config.api_key.is_none());
    }

    #[test]
    fn test_llm_provider_record_get_prompt_config() {
        let record = LlmProviderRecord {
            name: "test-llm".to_string(),
            provider_type: LlmProviderType::OpenAI,
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key_encrypted: None,
            model: "gpt-4o-mini".to_string(),
            enabled: true,
            is_default: false,
            compression_prompt: Some("Compress: {content}".to_string()),
            classification_prompt: Some("Classify: {content}".to_string()),
            max_input_tokens: 4000,
            max_output_tokens: 1000,
            temperature: 0.3,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let prompts = record.get_prompt_config();

        assert_eq!(
            prompts.compression_prompt,
            Some("Compress: {content}".to_string())
        );
        assert_eq!(
            prompts.classification_prompt,
            Some("Classify: {content}".to_string())
        );
    }

    #[test]
    fn test_update_llm_provider_input_default() {
        let input = UpdateLlmProviderInput::default();

        assert!(input.endpoint.is_none());
        assert!(input.api_key.is_none());
        assert!(input.model.is_none());
        assert!(input.enabled.is_none());
        assert!(input.rpm_limit.is_none());
        assert!(input.tpm_limit.is_none());
        assert!(input.compression_prompt.is_none());
        assert!(input.classification_prompt.is_none());
        assert!(input.max_input_tokens.is_none());
        assert!(input.max_output_tokens.is_none());
        assert!(input.temperature.is_none());
    }

    #[test]
    fn test_llm_prompt_config_default() {
        let config = LlmPromptConfig::default();

        assert!(config.compression_prompt.is_none());
        assert!(config.classification_prompt.is_none());
    }
}
