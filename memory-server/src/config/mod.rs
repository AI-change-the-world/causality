//! Configuration module for Memory Server
//!
//! Supports loading configuration from YAML files with environment variable overrides.

use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::env;

use crate::embedding::ProviderType;
use crate::llm::LlmProviderType;

/// Main application configuration
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub qdrant: QdrantConfig,
    pub llm: LlmConfig,
    pub embedding: EmbeddingConfig,
    pub lifecycle: LifecycleConfig,
    pub retrieval: RetrievalConfig,
    pub matching: MatchingConfig,
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
    /// Whether to run migrations on startup (default: true)
    #[serde(default = "default_run_migrations")]
    pub run_migrations: bool,
}

fn default_run_migrations() -> bool {
    true
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

/// LLM provider configuration (loaded from config.yaml)
#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    /// Provider type (openai, azure, local)
    pub provider_type: LlmProviderType,
    /// API endpoint URL
    pub endpoint: String,
    /// API key (can be overridden via MEMORY_SERVER__LLM__API_KEY)
    #[serde(default)]
    pub api_key: Option<String>,
    /// Model name/identifier
    pub model: String,
}

/// Embedding provider configuration (loaded from config.yaml)
#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingConfig {
    /// Provider type (openai, azure, local)
    pub provider_type: ProviderType,
    /// API endpoint URL
    pub endpoint: String,
    /// API key (can be overridden via MEMORY_SERVER__EMBEDDING__API_KEY)
    #[serde(default)]
    pub api_key: Option<String>,
    /// Model name/identifier
    pub model: String,
    /// Embedding dimension
    pub dimension: usize,
}

/// Memory lifecycle management configuration
#[derive(Debug, Clone, Deserialize)]
pub struct LifecycleConfig {
    #[serde(default = "default_cooldown_check_interval")]
    pub cooldown_check_interval_seconds: u64,
    #[serde(default)]
    pub decay_config: DecayConfig,
    #[serde(default)]
    pub eviction_config: EvictionConfig,
}

fn default_cooldown_check_interval() -> u64 {
    3600
}

/// Decay score calculation configuration
#[derive(Debug, Clone, Deserialize)]
pub struct DecayConfig {
    /// Base decay rate per day (0.0 - 1.0)
    #[serde(default = "default_base_decay_rate")]
    pub base_decay_rate: f32,
    /// Weight for hit count in decay calculation
    #[serde(default = "default_hit_count_weight")]
    pub hit_count_weight: f32,
    /// Weight for recency in decay calculation
    #[serde(default = "default_recency_weight_decay")]
    pub recency_weight: f32,
    /// Weight for importance in decay calculation
    #[serde(default = "default_importance_weight_decay")]
    pub importance_weight: f32,
    /// Minimum decay score (floor)
    #[serde(default = "default_min_decay_score")]
    pub min_decay_score: f32,
}

impl Default for DecayConfig {
    fn default() -> Self {
        Self {
            base_decay_rate: default_base_decay_rate(),
            hit_count_weight: default_hit_count_weight(),
            recency_weight: default_recency_weight_decay(),
            importance_weight: default_importance_weight_decay(),
            min_decay_score: default_min_decay_score(),
        }
    }
}

fn default_base_decay_rate() -> f32 {
    0.05
}

fn default_recency_weight_decay() -> f32 {
    0.3
}

fn default_importance_weight_decay() -> f32 {
    0.2
}

fn default_min_decay_score() -> f32 {
    0.01
}

/// Eviction configuration for LFU-based memory management
#[derive(Debug, Clone, Deserialize)]
pub struct EvictionConfig {
    /// Days of inactivity before transitioning to cooldown
    #[serde(default = "default_cooldown_threshold_days")]
    pub cooldown_threshold_days: i64,
    /// Days in cooldown before becoming candidate
    #[serde(default = "default_candidate_threshold_days")]
    pub candidate_threshold_days: i64,
    /// Days as candidate before archiving
    #[serde(default = "default_archive_threshold_days")]
    pub archive_threshold_days: i64,
    /// Maximum number of active memories per scope (0 = unlimited)
    #[serde(default = "default_max_memories_per_scope")]
    pub max_memories_per_scope: usize,
    /// Maximum number of global memories (0 = unlimited)
    #[serde(default = "default_max_global_memories")]
    pub max_global_memories: usize,
    /// Batch size for eviction processing
    #[serde(default = "default_eviction_batch_size")]
    pub batch_size: usize,
}

impl Default for EvictionConfig {
    fn default() -> Self {
        Self {
            cooldown_threshold_days: default_cooldown_threshold_days(),
            candidate_threshold_days: default_candidate_threshold_days(),
            archive_threshold_days: default_archive_threshold_days(),
            max_memories_per_scope: default_max_memories_per_scope(),
            max_global_memories: default_max_global_memories(),
            batch_size: default_eviction_batch_size(),
        }
    }
}

fn default_cooldown_threshold_days() -> i64 {
    7
}

fn default_candidate_threshold_days() -> i64 {
    14
}

fn default_archive_threshold_days() -> i64 {
    30
}

fn default_max_memories_per_scope() -> usize {
    1000
}

fn default_max_global_memories() -> usize {
    10000
}

fn default_eviction_batch_size() -> usize {
    100
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
    #[serde(default = "default_adaptive_expansion_enabled")]
    pub adaptive_expansion_enabled: bool,
    #[serde(default = "default_short_memory_char_threshold")]
    pub short_memory_char_threshold: usize,
    #[serde(default = "default_short_memory_context_budget_chars")]
    pub short_memory_context_budget_chars: usize,
    #[serde(default = "default_short_memory_max_results")]
    pub short_memory_max_results: usize,
    #[serde(default = "default_short_memory_score_slack")]
    pub short_memory_score_slack: f32,
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

fn default_adaptive_expansion_enabled() -> bool {
    true
}

fn default_short_memory_char_threshold() -> usize {
    120
}

fn default_short_memory_context_budget_chars() -> usize {
    2400
}

fn default_short_memory_max_results() -> usize {
    20
}

fn default_short_memory_score_slack() -> f32 {
    0.08
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
            adaptive_expansion_enabled: default_adaptive_expansion_enabled(),
            short_memory_char_threshold: default_short_memory_char_threshold(),
            short_memory_context_budget_chars: default_short_memory_context_budget_chars(),
            short_memory_max_results: default_short_memory_max_results(),
            short_memory_score_slack: default_short_memory_score_slack(),
            score_weights: ScoreWeights::default(),
        }
    }
}

/// Event-to-memory matching configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct MatchingConfig {
    /// Minimum vector similarity for an extracted memory to match an existing memory.
    #[serde(default = "default_match_similarity_threshold")]
    pub match_similarity_threshold: f32,
    /// Similarity threshold for candidate conflicts. Reserved for richer conflict checks.
    #[serde(default = "default_conflict_check_similarity_threshold")]
    pub conflict_check_similarity_threshold: f32,
    /// Maximum vector candidates to consider during reconciliation.
    #[serde(default = "default_max_match_candidates")]
    pub max_match_candidates: usize,
}

fn default_match_similarity_threshold() -> f32 {
    0.70
}

fn default_conflict_check_similarity_threshold() -> f32 {
    0.70
}

fn default_max_match_candidates() -> usize {
    10
}

impl Default for MatchingConfig {
    fn default() -> Self {
        Self {
            match_similarity_threshold: default_match_similarity_threshold(),
            conflict_check_similarity_threshold: default_conflict_check_similarity_threshold(),
            max_match_candidates: default_max_match_candidates(),
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
            .set_default("retrieval.adaptive_expansion_enabled", true)?
            .set_default("retrieval.short_memory_char_threshold", 120)?
            .set_default("retrieval.short_memory_context_budget_chars", 2400)?
            .set_default("retrieval.short_memory_max_results", 20)?
            .set_default("retrieval.short_memory_score_slack", 0.08)?
            .set_default("matching.match_similarity_threshold", 0.70)?
            .set_default("matching.conflict_check_similarity_threshold", 0.70)?
            .set_default("matching.max_match_candidates", 10)?
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
            .set_default("retrieval.adaptive_expansion_enabled", true)?
            .set_default("retrieval.short_memory_char_threshold", 120)?
            .set_default("retrieval.short_memory_context_budget_chars", 2400)?
            .set_default("retrieval.short_memory_max_results", 20)?
            .set_default("retrieval.short_memory_score_slack", 0.08)?
            .set_default("matching.match_similarity_threshold", 0.70)?
            .set_default("matching.conflict_check_similarity_threshold", 0.70)?
            .set_default("matching.max_match_candidates", 10)?
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
