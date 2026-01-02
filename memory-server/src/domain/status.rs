//! Status enums for Memory Server
//!
//! Contains Status (memory lifecycle state), EmbeddingStatus, UpdateMode,
//! ProcessingStatus, MemoryCategory, and InferenceType.

use serde::{Deserialize, Serialize};
use sqlx::TypeInfo;
use utoipa::ToSchema;

/// Memory lifecycle status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Newly created, pending validation
    Candidate,
    /// Active and participates in retrieval
    Active,
    /// Stable, frequently accessed memory
    Stable,
    /// Cooling down due to inactivity, retrieval score penalized
    Cooldown,
    /// Explicitly ignored by administrator
    Ignored,
    /// Archived (soft deleted or TTL expired)
    Archived,
}

impl Status {
    /// Check if this status allows the memory to be included in retrieval results
    pub fn is_retrievable(&self) -> bool {
        matches!(
            self,
            Status::Candidate | Status::Active | Status::Stable | Status::Cooldown
        )
    }

    /// Check if this status should apply a penalty to retrieval score
    pub fn has_retrieval_penalty(&self) -> bool {
        matches!(self, Status::Cooldown)
    }

    /// Check if this status is excluded from all retrieval
    pub fn is_excluded(&self) -> bool {
        matches!(self, Status::Ignored | Status::Archived)
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
            Status::Candidate => write!(f, "candidate"),
            Status::Active => write!(f, "active"),
            Status::Stable => write!(f, "stable"),
            Status::Cooldown => write!(f, "cooldown"),
            Status::Ignored => write!(f, "ignored"),
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

/// Memory content update mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "update_mode", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// Append new content to existing content
    Append,
    /// Merge new content with existing content (preserving both)
    Merge,
    /// Replace existing content with new content
    Supersede,
}

impl Default for UpdateMode {
    fn default() -> Self {
        UpdateMode::Supersede
    }
}

impl std::fmt::Display for UpdateMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateMode::Append => write!(f, "append"),
            UpdateMode::Merge => write!(f, "merge"),
            UpdateMode::Supersede => write!(f, "supersede"),
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
            ProcessingStatus::Completed => write!(f, "completed"),
            ProcessingStatus::Failed => write!(f, "failed"),
            ProcessingStatus::Skipped => write!(f, "skipped"),
        }
    }
}

/// Memory category for classification
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
///
/// Used to mark the type of inference made when extracting memories from raw events.
/// This helps distinguish between direct facts and inferred information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InferenceType {
    /// Direct fact extracted from the event (e.g., "user clicked button X")
    Fact,
    /// Inferred user preference (e.g., "user prefers dark theme")
    Preference,
    /// Identified behavior pattern (e.g., "user usually works in the morning")
    Pattern,
    /// Extracted business rule (e.g., "contracts require 3 signatures")
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
        // Compatible with VARCHAR and TEXT
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
///
/// Controls how much automation the system uses when processing events into memories.
/// This allows developers to balance between convenience and control.
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
///
/// Represents a single memory extracted from a raw event. The LLM automatically
/// determines the inference type, category, tags, and importance based on the
/// event content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMemory {
    /// The extracted memory content
    pub content: String,
    /// Type of inference made (fact, preference, pattern, rule)
    pub inference_type: InferenceType,
    /// Confidence score for this extraction (0.0 - 1.0)
    /// Facts should have confidence >= 0.9, preferences/patterns in [0.6, 0.8]
    pub confidence: f32,
    /// Auto-classified category
    pub category: MemoryCategory,
    /// Auto-extracted tags/keywords
    pub tags: Vec<String>,
    /// Importance score (0.0 - 1.0)
    pub importance: f32,
    /// Reasoning explaining why this memory was extracted
    pub reasoning: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_retrievable() {
        assert!(Status::Candidate.is_retrievable());
        assert!(Status::Active.is_retrievable());
        assert!(Status::Stable.is_retrievable());
        assert!(Status::Cooldown.is_retrievable());
        assert!(!Status::Ignored.is_retrievable());
        assert!(!Status::Archived.is_retrievable());
    }

    #[test]
    fn test_status_penalty() {
        assert!(!Status::Active.has_retrieval_penalty());
        assert!(!Status::Stable.has_retrieval_penalty());
        assert!(Status::Cooldown.has_retrieval_penalty());
    }

    #[test]
    fn test_status_excluded() {
        assert!(!Status::Active.is_excluded());
        assert!(!Status::Cooldown.is_excluded());
        assert!(Status::Ignored.is_excluded());
        assert!(Status::Archived.is_excluded());
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
            serde_json::to_string(&Status::Archived).unwrap(),
            "\"archived\""
        );
    }

    #[test]
    fn test_embedding_status_default() {
        assert_eq!(EmbeddingStatus::default(), EmbeddingStatus::Pending);
    }

    #[test]
    fn test_embedding_status_serialization() {
        assert_eq!(
            serde_json::to_string(&EmbeddingStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&EmbeddingStatus::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&EmbeddingStatus::Failed).unwrap(),
            "\"failed\""
        );
    }

    #[test]
    fn test_update_mode_default() {
        assert_eq!(UpdateMode::default(), UpdateMode::Supersede);
    }

    #[test]
    fn test_update_mode_serialization() {
        assert_eq!(
            serde_json::to_string(&UpdateMode::Append).unwrap(),
            "\"append\""
        );
        assert_eq!(
            serde_json::to_string(&UpdateMode::Merge).unwrap(),
            "\"merge\""
        );
        assert_eq!(
            serde_json::to_string(&UpdateMode::Supersede).unwrap(),
            "\"supersede\""
        );
    }

    #[test]
    fn test_status_display() {
        assert_eq!(Status::Active.to_string(), "active");
        assert_eq!(Status::Cooldown.to_string(), "cooldown");
        assert_eq!(Status::Archived.to_string(), "archived");
    }

    #[test]
    fn test_embedding_status_display() {
        assert_eq!(EmbeddingStatus::Pending.to_string(), "pending");
        assert_eq!(EmbeddingStatus::Completed.to_string(), "completed");
        assert_eq!(EmbeddingStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn test_update_mode_display() {
        assert_eq!(UpdateMode::Append.to_string(), "append");
        assert_eq!(UpdateMode::Merge.to_string(), "merge");
        assert_eq!(UpdateMode::Supersede.to_string(), "supersede");
    }

    #[test]
    fn test_processing_status_default() {
        assert_eq!(ProcessingStatus::default(), ProcessingStatus::Skipped);
    }

    #[test]
    fn test_processing_status_serialization() {
        assert_eq!(
            serde_json::to_string(&ProcessingStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&ProcessingStatus::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&ProcessingStatus::Failed).unwrap(),
            "\"failed\""
        );
        assert_eq!(
            serde_json::to_string(&ProcessingStatus::Skipped).unwrap(),
            "\"skipped\""
        );
    }

    #[test]
    fn test_processing_status_display() {
        assert_eq!(ProcessingStatus::Pending.to_string(), "pending");
        assert_eq!(ProcessingStatus::Completed.to_string(), "completed");
        assert_eq!(ProcessingStatus::Failed.to_string(), "failed");
        assert_eq!(ProcessingStatus::Skipped.to_string(), "skipped");
    }

    #[test]
    fn test_memory_category_default() {
        assert_eq!(MemoryCategory::default(), MemoryCategory::Other);
    }

    #[test]
    fn test_memory_category_serialization() {
        assert_eq!(
            serde_json::to_string(&MemoryCategory::UserPreference).unwrap(),
            "\"user_preference\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryCategory::BehaviorPattern).unwrap(),
            "\"behavior_pattern\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryCategory::BusinessRule).unwrap(),
            "\"business_rule\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryCategory::FactualKnowledge).unwrap(),
            "\"factual_knowledge\""
        );
        assert_eq!(
            serde_json::to_string(&MemoryCategory::Other).unwrap(),
            "\"other\""
        );
    }

    #[test]
    fn test_memory_category_display() {
        assert_eq!(
            MemoryCategory::UserPreference.to_string(),
            "user_preference"
        );
        assert_eq!(
            MemoryCategory::BehaviorPattern.to_string(),
            "behavior_pattern"
        );
        assert_eq!(MemoryCategory::BusinessRule.to_string(), "business_rule");
        assert_eq!(
            MemoryCategory::FactualKnowledge.to_string(),
            "factual_knowledge"
        );
        assert_eq!(MemoryCategory::Other.to_string(), "other");
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
    fn test_inference_type_serialization() {
        assert_eq!(
            serde_json::to_string(&InferenceType::Fact).unwrap(),
            "\"fact\""
        );
        assert_eq!(
            serde_json::to_string(&InferenceType::Preference).unwrap(),
            "\"preference\""
        );
        assert_eq!(
            serde_json::to_string(&InferenceType::Pattern).unwrap(),
            "\"pattern\""
        );
        assert_eq!(
            serde_json::to_string(&InferenceType::Rule).unwrap(),
            "\"rule\""
        );
    }

    #[test]
    fn test_inference_type_deserialization() {
        assert_eq!(
            serde_json::from_str::<InferenceType>("\"fact\"").unwrap(),
            InferenceType::Fact
        );
        assert_eq!(
            serde_json::from_str::<InferenceType>("\"preference\"").unwrap(),
            InferenceType::Preference
        );
        assert_eq!(
            serde_json::from_str::<InferenceType>("\"pattern\"").unwrap(),
            InferenceType::Pattern
        );
        assert_eq!(
            serde_json::from_str::<InferenceType>("\"rule\"").unwrap(),
            InferenceType::Rule
        );
    }

    #[test]
    fn test_inference_type_display() {
        assert_eq!(InferenceType::Fact.to_string(), "fact");
        assert_eq!(InferenceType::Preference.to_string(), "preference");
        assert_eq!(InferenceType::Pattern.to_string(), "pattern");
        assert_eq!(InferenceType::Rule.to_string(), "rule");
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
        // Test case insensitivity
        assert_eq!(
            "FACT".parse::<InferenceType>().unwrap(),
            InferenceType::Fact
        );
        assert_eq!(
            "Preference".parse::<InferenceType>().unwrap(),
            InferenceType::Preference
        );
        // Test unknown value
        assert!("unknown".parse::<InferenceType>().is_err());
    }

    #[test]
    fn test_processing_mode_default() {
        assert_eq!(ProcessingMode::default(), ProcessingMode::Assisted);
    }

    #[test]
    fn test_processing_mode_serialization() {
        assert_eq!(
            serde_json::to_string(&ProcessingMode::Auto).unwrap(),
            "\"auto\""
        );
        assert_eq!(
            serde_json::to_string(&ProcessingMode::Assisted).unwrap(),
            "\"assisted\""
        );
        assert_eq!(
            serde_json::to_string(&ProcessingMode::Manual).unwrap(),
            "\"manual\""
        );
    }

    #[test]
    fn test_processing_mode_deserialization() {
        assert_eq!(
            serde_json::from_str::<ProcessingMode>("\"auto\"").unwrap(),
            ProcessingMode::Auto
        );
        assert_eq!(
            serde_json::from_str::<ProcessingMode>("\"assisted\"").unwrap(),
            ProcessingMode::Assisted
        );
        assert_eq!(
            serde_json::from_str::<ProcessingMode>("\"manual\"").unwrap(),
            ProcessingMode::Manual
        );
    }

    #[test]
    fn test_processing_mode_display() {
        assert_eq!(ProcessingMode::Auto.to_string(), "auto");
        assert_eq!(ProcessingMode::Assisted.to_string(), "assisted");
        assert_eq!(ProcessingMode::Manual.to_string(), "manual");
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
        // Test case insensitivity
        assert_eq!(
            "AUTO".parse::<ProcessingMode>().unwrap(),
            ProcessingMode::Auto
        );
        assert_eq!(
            "Assisted".parse::<ProcessingMode>().unwrap(),
            ProcessingMode::Assisted
        );
        // Test unknown value
        assert!("unknown".parse::<ProcessingMode>().is_err());
    }
}
