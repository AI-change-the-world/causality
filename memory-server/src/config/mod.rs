//! Configuration module for Memory Server
//!
//! Supports loading configuration from YAML files with environment variable overrides.

use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::env;

/// Main application configuration
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub qdrant: QdrantConfig,
    pub embedding: EmbeddingConfig,
    pub lifecycle: LifecycleConfig,
    pub retrieval: RetrievalConfig,
    pub audit: AuditConfig,
}

/// HTTP server configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8080
}

/// Database connection configuration
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_min_connections")]
    pub min_connections: u32,
}

fn default_max_connections() -> u32 {
    10
}

fn default_min_connections() -> u32 {
    2
}

/// Qdrant vector database configuration
#[derive(Debug, Clone, Deserialize)]
pub struct QdrantConfig {
    pub url: String,
    #[serde(default = "default_collection_name")]
    pub collection_name: String,
}

fn default_collection_name() -> String {
    "memories".to_string()
}

/// Embedding provider configuration
#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingConfig {
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
}

/// Individual embedding provider configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub provider_type: String,
    pub endpoint: String,
    #[serde(default)]
    pub api_key: Option<String>,
    pub model: String,
    pub dimension: usize,
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
}

/// Rate limiting configuration for embedding providers
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    pub requests_per_minute: Option<u32>,
    pub tokens_per_minute: Option<u32>,
}

/// Memory lifecycle management configuration
#[derive(Debug, Clone, Deserialize)]
pub struct LifecycleConfig {
    #[serde(default = "default_cooldown_check_interval")]
    pub cooldown_check_interval_seconds: u64,
    #[serde(default)]
    pub cooldown_thresholds: CooldownThresholds,
    #[serde(default)]
    pub default_ttl: DefaultTtl,
}

fn default_cooldown_check_interval() -> u64 {
    3600
}

/// Cooldown thresholds per layer (in seconds)
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CooldownThresholds {
    #[serde(default = "default_session_cooldown")]
    pub session: u64,
    #[serde(default = "default_task_cooldown")]
    pub task: u64,
    #[serde(default = "default_long_term_cooldown")]
    pub long_term: u64,
}

fn default_session_cooldown() -> u64 {
    86400 // 1 day
}

fn default_task_cooldown() -> u64 {
    604800 // 7 days
}

fn default_long_term_cooldown() -> u64 {
    2592000 // 30 days
}

/// Default TTL per layer (in seconds)
#[derive(Debug, Clone, Deserialize, Default)]
pub struct DefaultTtl {
    #[serde(default = "default_session_ttl")]
    pub session: Option<u64>,
    #[serde(default = "default_task_ttl")]
    pub task: Option<u64>,
    #[serde(default)]
    pub long_term: Option<u64>,
}

fn default_session_ttl() -> Option<u64> {
    Some(3600) // 1 hour
}

fn default_task_ttl() -> Option<u64> {
    Some(604800) // 7 days
}

/// Retrieval engine configuration
#[derive(Debug, Clone, Deserialize)]
pub struct RetrievalConfig {
    #[serde(default = "default_top_k")]
    pub default_top_k: usize,
    #[serde(default = "default_max_top_k")]
    pub max_top_k: usize,
    #[serde(default = "default_cooldown_penalty")]
    pub cooldown_penalty: f32,
    #[serde(default)]
    pub score_weights: ScoreWeights,
}

fn default_top_k() -> usize {
    10
}

fn default_max_top_k() -> usize {
    100
}

fn default_cooldown_penalty() -> f32 {
    0.5
}

/// Score weights for composite scoring
#[derive(Debug, Clone, Deserialize)]
pub struct ScoreWeights {
    #[serde(default = "default_similarity_weight")]
    pub similarity: f32,
    #[serde(default = "default_importance_weight")]
    pub importance: f32,
    #[serde(default = "default_recency_weight")]
    pub recency: f32,
    #[serde(default = "default_hit_count_weight")]
    pub hit_count: f32,
    #[serde(default = "default_fulltext_weight")]
    pub fulltext: f32,
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            similarity: default_similarity_weight(),
            importance: default_importance_weight(),
            recency: default_recency_weight(),
            hit_count: default_hit_count_weight(),
            fulltext: default_fulltext_weight(),
        }
    }
}

fn default_similarity_weight() -> f32 {
    0.35
}

fn default_importance_weight() -> f32 {
    0.25
}

fn default_recency_weight() -> f32 {
    0.15
}

fn default_hit_count_weight() -> f32 {
    0.1
}

fn default_fulltext_weight() -> f32 {
    0.15
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            default_top_k: default_top_k(),
            max_top_k: default_max_top_k(),
            cooldown_penalty: default_cooldown_penalty(),
            score_weights: ScoreWeights::default(),
        }
    }
}

/// Audit logging configuration
#[derive(Debug, Clone, Deserialize)]
pub struct AuditConfig {
    #[serde(default = "default_audit_enabled")]
    pub enabled: bool,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

fn default_audit_enabled() -> bool {
    true
}

fn default_retention_days() -> u32 {
    90
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: default_audit_enabled(),
            retention_days: default_retention_days(),
        }
    }
}

impl AppConfig {
    /// Load configuration from YAML file with environment variable overrides.
    ///
    /// Configuration is loaded in the following order (later sources override earlier):
    /// 1. Default values
    /// 2. YAML config file (path from CONFIG_PATH env var or default "config/config.yaml")
    /// 3. Environment variables (prefixed with MEMORY_SERVER__)
    pub fn load() -> Result<Self, ConfigError> {
        let config_path =
            env::var("CONFIG_PATH").unwrap_or_else(|_| "config/config.yaml".to_string());

        let builder = Config::builder()
            // Start with default values
            .set_default("server.host", "0.0.0.0")?
            .set_default("server.port", 8080)?
            .set_default("database.max_connections", 10)?
            .set_default("database.min_connections", 2)?
            .set_default("qdrant.collection_name", "memories")?
            .set_default("lifecycle.cooldown_check_interval_seconds", 3600)?
            .set_default("retrieval.default_top_k", 10)?
            .set_default("retrieval.max_top_k", 100)?
            .set_default("retrieval.cooldown_penalty", 0.5)?
            .set_default("audit.enabled", true)?
            .set_default("audit.retention_days", 90)?
            // Load from YAML file if it exists
            .add_source(File::with_name(&config_path).required(false))
            // Override with environment variables
            // e.g., MEMORY_SERVER__DATABASE__URL -> database.url
            .add_source(
                Environment::with_prefix("MEMORY_SERVER")
                    .separator("__")
                    .try_parsing(true),
            );

        let config = builder.build()?;
        config.try_deserialize()
    }

    /// Load configuration from a specific YAML file path
    pub fn load_from_file(path: &str) -> Result<Self, ConfigError> {
        let builder = Config::builder()
            .set_default("server.host", "0.0.0.0")?
            .set_default("server.port", 8080)?
            .set_default("database.max_connections", 10)?
            .set_default("database.min_connections", 2)?
            .set_default("qdrant.collection_name", "memories")?
            .set_default("lifecycle.cooldown_check_interval_seconds", 3600)?
            .set_default("retrieval.default_top_k", 10)?
            .set_default("retrieval.max_top_k", 100)?
            .set_default("retrieval.cooldown_penalty", 0.5)?
            .set_default("audit.enabled", true)?
            .set_default("audit.retention_days", 90)?
            .add_source(File::with_name(path).required(true))
            .add_source(
                Environment::with_prefix("MEMORY_SERVER")
                    .separator("__")
                    .try_parsing(true),
            );

        let config = builder.build()?;
        config.try_deserialize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        assert_eq!(default_host(), "0.0.0.0");
        assert_eq!(default_port(), 8080);
        assert_eq!(default_max_connections(), 10);
        assert_eq!(default_min_connections(), 2);
        assert_eq!(default_collection_name(), "memories");
    }

    #[test]
    fn test_score_weights_default() {
        let weights = ScoreWeights::default();
        assert!((weights.similarity - 0.35).abs() < f32::EPSILON);
        assert!((weights.importance - 0.25).abs() < f32::EPSILON);
        assert!((weights.recency - 0.15).abs() < f32::EPSILON);
        assert!((weights.hit_count - 0.1).abs() < f32::EPSILON);
        assert!((weights.fulltext - 0.15).abs() < f32::EPSILON);
    }

    #[test]
    fn test_audit_config_default() {
        let audit = AuditConfig::default();
        assert!(audit.enabled);
        assert_eq!(audit.retention_days, 90);
    }
}
