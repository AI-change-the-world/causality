//! ScopeType enum representing memory ownership
//!
//! Determines who a memory belongs to in the system hierarchy.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Memory scope type determining ownership
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "scope_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ScopeType {
    /// Memory belongs to a specific user
    User,
    /// Memory belongs to an organization
    Org,
    /// Memory belongs to a project
    Project,
    /// Memory belongs to a specific task
    Task,
    /// Memory belongs to a session
    Session,
}

impl ScopeType {
    /// Get all valid scope types
    pub fn all() -> &'static [ScopeType] {
        &[
            ScopeType::User,
            ScopeType::Org,
            ScopeType::Project,
            ScopeType::Task,
            ScopeType::Session,
        ]
    }

    /// Parse from string, returning None for invalid values
    pub fn from_str(s: &str) -> Option<ScopeType> {
        match s.to_lowercase().as_str() {
            "user" => Some(ScopeType::User),
            "org" => Some(ScopeType::Org),
            "project" => Some(ScopeType::Project),
            "task" => Some(ScopeType::Task),
            "session" => Some(ScopeType::Session),
            _ => None,
        }
    }
}

impl std::fmt::Display for ScopeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScopeType::User => write!(f, "user"),
            ScopeType::Org => write!(f, "org"),
            ScopeType::Project => write!(f, "project"),
            ScopeType::Task => write!(f, "task"),
            ScopeType::Session => write!(f, "session"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_type_all() {
        let all = ScopeType::all();
        assert_eq!(all.len(), 5);
        assert!(all.contains(&ScopeType::User));
        assert!(all.contains(&ScopeType::Org));
        assert!(all.contains(&ScopeType::Project));
        assert!(all.contains(&ScopeType::Task));
        assert!(all.contains(&ScopeType::Session));
    }

    #[test]
    fn test_scope_type_from_str() {
        assert_eq!(ScopeType::from_str("user"), Some(ScopeType::User));
        assert_eq!(ScopeType::from_str("USER"), Some(ScopeType::User));
        assert_eq!(ScopeType::from_str("org"), Some(ScopeType::Org));
        assert_eq!(ScopeType::from_str("project"), Some(ScopeType::Project));
        assert_eq!(ScopeType::from_str("task"), Some(ScopeType::Task));
        assert_eq!(ScopeType::from_str("session"), Some(ScopeType::Session));
        assert_eq!(ScopeType::from_str("invalid"), None);
    }

    #[test]
    fn test_scope_type_serialization() {
        assert_eq!(serde_json::to_string(&ScopeType::User).unwrap(), "\"user\"");
        assert_eq!(serde_json::to_string(&ScopeType::Org).unwrap(), "\"org\"");
        assert_eq!(
            serde_json::to_string(&ScopeType::Project).unwrap(),
            "\"project\""
        );
        assert_eq!(serde_json::to_string(&ScopeType::Task).unwrap(), "\"task\"");
        assert_eq!(
            serde_json::to_string(&ScopeType::Session).unwrap(),
            "\"session\""
        );
    }

    #[test]
    fn test_scope_type_deserialization() {
        assert_eq!(
            serde_json::from_str::<ScopeType>("\"user\"").unwrap(),
            ScopeType::User
        );
        assert_eq!(
            serde_json::from_str::<ScopeType>("\"org\"").unwrap(),
            ScopeType::Org
        );
        assert_eq!(
            serde_json::from_str::<ScopeType>("\"project\"").unwrap(),
            ScopeType::Project
        );
        assert_eq!(
            serde_json::from_str::<ScopeType>("\"task\"").unwrap(),
            ScopeType::Task
        );
        assert_eq!(
            serde_json::from_str::<ScopeType>("\"session\"").unwrap(),
            ScopeType::Session
        );
    }

    #[test]
    fn test_scope_type_display() {
        assert_eq!(ScopeType::User.to_string(), "user");
        assert_eq!(ScopeType::Org.to_string(), "org");
        assert_eq!(ScopeType::Project.to_string(), "project");
        assert_eq!(ScopeType::Task.to_string(), "task");
        assert_eq!(ScopeType::Session.to_string(), "session");
    }
}
