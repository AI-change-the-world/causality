//! Layer enum representing memory lifecycle tiers
//!
//! Memories are organized into three layers based on their expected lifetime:
//! - Session: Short-lived, minute-level TTL
//! - Task: Medium-lived, day-level TTL
//! - LongTerm: Long-lived, requires manual confirmation to create

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Memory layer determining lifecycle and TTL behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "layer", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    /// Short-lived memories, typically minute-level TTL (default: 1 hour)
    Session,
    /// Medium-lived memories, typically day-level TTL (default: 7 days)
    Task,
    /// Long-lived memories, no automatic expiry, requires manual confirmation
    #[serde(rename = "long_term")]
    #[sqlx(rename = "long_term")]
    LongTerm,
}

impl Layer {
    /// Check if this layer allows direct creation
    /// Long-term memories cannot be created directly and require manual confirmation
    pub fn allows_direct_creation(&self) -> bool {
        !matches!(self, Layer::LongTerm)
    }

    /// Get the default TTL in seconds for this layer
    /// Returns None for LongTerm as it has no automatic expiry
    pub fn default_ttl_seconds(&self) -> Option<i64> {
        match self {
            Layer::Session => Some(3600), // 1 hour
            Layer::Task => Some(604800),  // 7 days
            Layer::LongTerm => None,      // No expiry
        }
    }

    /// Get the cooldown threshold in seconds for this layer
    pub fn cooldown_threshold_seconds(&self) -> i64 {
        match self {
            Layer::Session => 86400,    // 1 day
            Layer::Task => 604800,      // 7 days
            Layer::LongTerm => 2592000, // 30 days
        }
    }
}

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Layer::Session => write!(f, "session"),
            Layer::Task => write!(f, "task"),
            Layer::LongTerm => write!(f, "long_term"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_direct_creation() {
        assert!(Layer::Session.allows_direct_creation());
        assert!(Layer::Task.allows_direct_creation());
        assert!(!Layer::LongTerm.allows_direct_creation());
    }

    #[test]
    fn test_layer_default_ttl() {
        assert_eq!(Layer::Session.default_ttl_seconds(), Some(3600));
        assert_eq!(Layer::Task.default_ttl_seconds(), Some(604800));
        assert_eq!(Layer::LongTerm.default_ttl_seconds(), None);
    }

    #[test]
    fn test_layer_serialization() {
        assert_eq!(
            serde_json::to_string(&Layer::Session).unwrap(),
            "\"session\""
        );
        assert_eq!(serde_json::to_string(&Layer::Task).unwrap(), "\"task\"");
        assert_eq!(
            serde_json::to_string(&Layer::LongTerm).unwrap(),
            "\"long_term\""
        );
    }

    #[test]
    fn test_layer_deserialization() {
        assert_eq!(
            serde_json::from_str::<Layer>("\"session\"").unwrap(),
            Layer::Session
        );
        assert_eq!(
            serde_json::from_str::<Layer>("\"task\"").unwrap(),
            Layer::Task
        );
        assert_eq!(
            serde_json::from_str::<Layer>("\"long_term\"").unwrap(),
            Layer::LongTerm
        );
    }

    #[test]
    fn test_layer_display() {
        assert_eq!(Layer::Session.to_string(), "session");
        assert_eq!(Layer::Task.to_string(), "task");
        assert_eq!(Layer::LongTerm.to_string(), "long_term");
    }
}
