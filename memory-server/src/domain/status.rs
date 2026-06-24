//! Status enums for Memory Server
//!
//! Contains Status (memory lifecycle state), EmbeddingStatus,
//! ProcessingStatus, MemoryCategory, and InferenceType.

use serde::{Deserialize, Serialize};
use sqlx::TypeInfo;
use utoipa::ToSchema;

/// Memory lifecycle status (simplified for new architecture)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Active and participates in retrieval
    Active,
    /// Cooling down due to inactivity (no hits for cooldown_threshold)
    Cooldown,
    /// Candidate for eviction (no hits for candidate_threshold)
    Candidate,
    /// Superseded by a newer version (content conflict)
    Superseded,
    /// Archived (evicted or manually archived)
    Archived,
}

impl Status {
    /// Check if this status allows the memory to be included in retrieval results
    pub fn is_retrievable(&self) -> bool {
        matches!(self, Status::Active | Status::Cooldown)
    }

    /// Check if this status should apply a penalty to retrieval score
    pub fn has_retrieval_penalty(&self) -> bool {
        matches!(self, Status::Cooldown)
    }

    /// Check if this status is excluded from all retrieval
    pub fn is_excluded(&self) -> bool {
        matches!(
            self,
            Status::Candidate | Status::Superseded | Status::Archived
        )
    }

    /// Check if this is a current version status (not superseded)
    pub fn is_current(&self) -> bool {
        !matches!(self, Status::Superseded)
    }
}

impl Default for Status {
    fn default() -> Self {
        Status::Active
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::Active => write!(f, "active"),
            Status::Cooldown => write!(f, "cooldown"),
            Status::Candidate => write!(f, "candidate"),
            Status::Superseded => write!(f, "superseded"),
            Status::Archived => write!(f, "archived"),
        }
    }
}

/// Embedding generation status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "embedding_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingStatus {
    /// Embedding generation pending
    Pending,
    /// Embedding successfully generated
    Completed,
    /// Embedding generation failed after retries
    Failed,
}

impl Default for EmbeddingStatus {
    fn default() -> Self {
        EmbeddingStatus::Pending
    }
}

impl std::fmt::Display for EmbeddingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbeddingStatus::Pending => write!(f, "pending"),
            EmbeddingStatus::Completed => write!(f, "completed"),
            EmbeddingStatus::Failed => write!(f, "failed"),
        }
    }
}

/// LLM processing status for memory content
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "processing_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ProcessingStatus {
    /// Waiting for LLM processing
    Pending,
    /// Currently claimed by a worker or request.
    Processing,
    /// LLM processing completed successfully
    Completed,
    /// LLM processing failed
    Failed,
    /// LLM processing was skipped (not requested)
    Skipped,
}

impl Default for ProcessingStatus {
    fn default() -> Self {
        ProcessingStatus::Skipped
    }
}

impl std::fmt::Display for ProcessingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessingStatus::Pending => write!(f, "pending"),
            ProcessingStatus::Processing => write!(f, "processing"),
            ProcessingStatus::Completed => write!(f, "completed"),
            ProcessingStatus::Failed => write!(f, "failed"),
            ProcessingStatus::Skipped => write!(f, "skipped"),
        }
    }
}

/// Memory category for classification (kept for backward compatibility)
/// Note: In the new architecture, category is a hierarchical string field
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "memory_category", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    /// User preferences (e.g., likes dark theme)
    UserPreference,
    /// Behavior patterns (e.g., usually processes emails in the morning)
    BehaviorPattern,
    /// Business rules (e.g., contract approval requires three signatures)
    BusinessRule,
    /// Factual knowledge (e.g., project deadline is X)
    FactualKnowledge,
    /// Other category
    Other,
}

impl Default for MemoryCategory {
    fn default() -> Self {
        MemoryCategory::Other
    }
}

impl std::fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryCategory::UserPreference => write!(f, "user_preference"),
            MemoryCategory::BehaviorPattern => write!(f, "behavior_pattern"),
            MemoryCategory::BusinessRule => write!(f, "business_rule"),
            MemoryCategory::FactualKnowledge => write!(f, "factual_knowledge"),
            MemoryCategory::Other => write!(f, "other"),
        }
    }
}

impl std::str::FromStr for MemoryCategory {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "user_preference" | "userpreference" => Ok(MemoryCategory::UserPreference),
            "behavior_pattern" | "behaviorpattern" => Ok(MemoryCategory::BehaviorPattern),
            "business_rule" | "businessrule" => Ok(MemoryCategory::BusinessRule),
            "factual_knowledge" | "factualknowledge" => Ok(MemoryCategory::FactualKnowledge),
            "other" => Ok(MemoryCategory::Other),
            _ => Err(format!("Unknown memory category: {}", s)),
        }
    }
}

/// Inference type for memories extracted from events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InferenceType {
    /// Direct fact extracted from the event
    Fact,
    /// Inferred user preference
    Preference,
    /// Identified behavior pattern
    Pattern,
    /// Extracted business rule
    Rule,
}

// Manual sqlx implementation for VARCHAR storage
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for InferenceType {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, Box<dyn std::error::Error + 'static + Send + Sync>> {
        let s = <&str as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        s.parse::<InferenceType>().map_err(|e| {
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
                as Box<dyn std::error::Error + Send + Sync>
        })
    }
}

impl sqlx::Type<sqlx::Postgres> for InferenceType {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }

    fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
        *ty == <String as sqlx::Type<sqlx::Postgres>>::type_info()
            || ty.name() == "VARCHAR"
            || ty.name() == "TEXT"
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Postgres> for InferenceType {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, Box<dyn std::error::Error + Send + Sync>> {
        let s = self.to_string();
        <String as sqlx::Encode<sqlx::Postgres>>::encode(s, buf)
    }
}

impl Default for InferenceType {
    fn default() -> Self {
        InferenceType::Fact
    }
}

impl std::fmt::Display for InferenceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InferenceType::Fact => write!(f, "fact"),
            InferenceType::Preference => write!(f, "preference"),
            InferenceType::Pattern => write!(f, "pattern"),
            InferenceType::Rule => write!(f, "rule"),
        }
    }
}

impl std::str::FromStr for InferenceType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "fact" => Ok(InferenceType::Fact),
            "preference" => Ok(InferenceType::Preference),
            "pattern" => Ok(InferenceType::Pattern),
            "rule" => Ok(InferenceType::Rule),
            _ => Err(format!("Unknown inference type: {}", s)),
        }
    }
}

/// Processing mode for event-to-memory extraction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "processing_mode", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ProcessingMode {
    /// Full automation: extract and create memories without confirmation
    Auto,
    /// Assisted mode: return proposed memories for user approval (default)
    Assisted,
    /// Manual mode: only summarize events without extraction
    Manual,
}

impl Default for ProcessingMode {
    fn default() -> Self {
        ProcessingMode::Assisted
    }
}

impl std::fmt::Display for ProcessingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessingMode::Auto => write!(f, "auto"),
            ProcessingMode::Assisted => write!(f, "assisted"),
            ProcessingMode::Manual => write!(f, "manual"),
        }
    }
}

impl std::str::FromStr for ProcessingMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(ProcessingMode::Auto),
            "assisted" => Ok(ProcessingMode::Assisted),
            "manual" => Ok(ProcessingMode::Manual),
            _ => Err(format!("Unknown processing mode: {}", s)),
        }
    }
}

/// Memory extracted from an event by LLM processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMemory {
    /// The extracted memory content
    pub content: String,
    /// Hierarchical category (e.g., "work.code.eslint")
    pub category: Option<String>,
    /// Auto-extracted tags/keywords
    pub tags: Option<Vec<String>>,
    /// Importance score (0.0 - 1.0)
    pub importance: f32,
    /// Confidence score for this extraction (0.0 - 1.0)
    pub confidence: f32,
    /// Type of inference made (fact, preference, pattern, rule)
    pub inference_type: InferenceType,
    /// Reasoning explaining why this memory was extracted
    pub reasoning: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_retrievable() {
        assert!(Status::Active.is_retrievable());
        assert!(Status::Cooldown.is_retrievable());
        assert!(!Status::Candidate.is_retrievable());
        assert!(!Status::Superseded.is_retrievable());
        assert!(!Status::Archived.is_retrievable());
    }

    #[test]
    fn test_status_penalty() {
        assert!(!Status::Active.has_retrieval_penalty());
        assert!(Status::Cooldown.has_retrieval_penalty());
        assert!(!Status::Superseded.has_retrieval_penalty());
    }

    #[test]
    fn test_status_excluded() {
        assert!(!Status::Active.is_excluded());
        assert!(!Status::Cooldown.is_excluded());
        assert!(Status::Candidate.is_excluded());
        assert!(Status::Superseded.is_excluded());
        assert!(Status::Archived.is_excluded());
    }

    #[test]
    fn test_status_current() {
        assert!(Status::Active.is_current());
        assert!(Status::Cooldown.is_current());
        assert!(Status::Candidate.is_current());
        assert!(!Status::Superseded.is_current());
        assert!(Status::Archived.is_current());
    }

    #[test]
    fn test_status_default() {
        assert_eq!(Status::default(), Status::Active);
    }

    #[test]
    fn test_status_serialization() {
        assert_eq!(
            serde_json::to_string(&Status::Active).unwrap(),
            "\"active\""
        );
        assert_eq!(
            serde_json::to_string(&Status::Cooldown).unwrap(),
            "\"cooldown\""
        );
        assert_eq!(
            serde_json::to_string(&Status::Candidate).unwrap(),
            "\"candidate\""
        );
        assert_eq!(
            serde_json::to_string(&Status::Superseded).unwrap(),
            "\"superseded\""
        );
        assert_eq!(
            serde_json::to_string(&Status::Archived).unwrap(),
            "\"archived\""
        );
    }

    #[test]
    fn test_status_display() {
        assert_eq!(Status::Active.to_string(), "active");
        assert_eq!(Status::Cooldown.to_string(), "cooldown");
        assert_eq!(Status::Candidate.to_string(), "candidate");
        assert_eq!(Status::Superseded.to_string(), "superseded");
        assert_eq!(Status::Archived.to_string(), "archived");
    }

    #[test]
    fn test_embedding_status_default() {
        assert_eq!(EmbeddingStatus::default(), EmbeddingStatus::Pending);
    }

    #[test]
    fn test_processing_status_default() {
        assert_eq!(ProcessingStatus::default(), ProcessingStatus::Skipped);
    }

    #[test]
    fn test_processing_status_processing_display() {
        assert_eq!(ProcessingStatus::Processing.to_string(), "processing");
    }

    #[test]
    fn test_memory_category_default() {
        assert_eq!(MemoryCategory::default(), MemoryCategory::Other);
    }

    #[test]
    fn test_memory_category_from_str() {
        assert_eq!(
            "user_preference".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::UserPreference
        );
        assert_eq!(
            "behavior_pattern".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::BehaviorPattern
        );
        assert_eq!(
            "business_rule".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::BusinessRule
        );
        assert_eq!(
            "factual_knowledge".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::FactualKnowledge
        );
        assert_eq!(
            "other".parse::<MemoryCategory>().unwrap(),
            MemoryCategory::Other
        );
        assert!("unknown".parse::<MemoryCategory>().is_err());
    }

    #[test]
    fn test_inference_type_default() {
        assert_eq!(InferenceType::default(), InferenceType::Fact);
    }

    #[test]
    fn test_inference_type_from_str() {
        assert_eq!(
            "fact".parse::<InferenceType>().unwrap(),
            InferenceType::Fact
        );
        assert_eq!(
            "preference".parse::<InferenceType>().unwrap(),
            InferenceType::Preference
        );
        assert_eq!(
            "pattern".parse::<InferenceType>().unwrap(),
            InferenceType::Pattern
        );
        assert_eq!(
            "rule".parse::<InferenceType>().unwrap(),
            InferenceType::Rule
        );
        assert!("unknown".parse::<InferenceType>().is_err());
    }

    #[test]
    fn test_processing_mode_default() {
        assert_eq!(ProcessingMode::default(), ProcessingMode::Assisted);
    }

    #[test]
    fn test_processing_mode_from_str() {
        assert_eq!(
            "auto".parse::<ProcessingMode>().unwrap(),
            ProcessingMode::Auto
        );
        assert_eq!(
            "assisted".parse::<ProcessingMode>().unwrap(),
            ProcessingMode::Assisted
        );
        assert_eq!(
            "manual".parse::<ProcessingMode>().unwrap(),
            ProcessingMode::Manual
        );
        assert!("unknown".parse::<ProcessingMode>().is_err());
    }
}
