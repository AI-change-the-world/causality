//! Event entity and related types
//!
//! Events represent raw occurrences (clicks, conversations, operations) that
//! can be processed to extract or reinforce memories.
//!
//! In the new architecture:
//! - Events are completely immutable after creation
//! - scope_id is user-defined (not validated semantically)
//! - No scene field (removed for simplicity)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::fmt;
use uuid::Uuid;

use crate::error::AppError;

/// Relation type between an event and a memory
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[sqlx(type_name = "event_memory_relation_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EventMemoryRelationType {
    /// Memory was created from this event
    CreatedFrom,
    /// Memory was reinforced by this event (evidence accumulation)
    ReinforcedBy,
}

impl fmt::Display for EventMemoryRelationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventMemoryRelationType::CreatedFrom => write!(f, "created_from"),
            EventMemoryRelationType::ReinforcedBy => write!(f, "reinforced_by"),
        }
    }
}

impl std::str::FromStr for EventMemoryRelationType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "created_from" | "createdfrom" => Ok(EventMemoryRelationType::CreatedFrom),
            "reinforced_by" | "reinforcedby" => Ok(EventMemoryRelationType::ReinforcedBy),
            _ => Err(format!("Unknown relation type: {}", s)),
        }
    }
}

/// Event entity representing a raw occurrence (completely immutable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Unique identifier
    pub id: Uuid,
    /// System profile ID - which business system this event belongs to
    pub profile_id: Uuid,
    /// Owner ID - user-defined, not validated semantically
    pub owner_id: String,
    /// Scope identifier - user-defined, null = global context
    pub scope_id: Option<String>,
    /// Raw event content (immutable)
    pub content: String,
    /// Optional context to help understand the event (immutable)
    pub context: Option<String>,
    /// LLM-generated summary of the event
    pub summary: Option<String>,
    /// Event source (e.g., "user_created", "api", "conversation")
    pub source: Option<String>,
    /// Whether the event has been processed
    pub processed: bool,
    /// When the event occurred
    pub event_time: DateTime<Utc>,
    /// When the event record was created
    pub created_at: DateTime<Utc>,
}

impl Event {
    /// Create a new Event
    pub fn new(input: CreateEventInput) -> Self {
        let now = Utc::now();
        Event {
            id: Uuid::new_v4(),
            profile_id: input.profile_id,
            owner_id: input.owner_id,
            scope_id: input.scope_id,
            content: input.content,
            context: input.context,
            summary: None,
            source: input.source,
            processed: false,
            event_time: input.event_time.unwrap_or(now),
            created_at: now,
        }
    }

    /// Mark the event as processed with a summary
    /// Note: This is the only mutable operation allowed on an Event
    pub fn mark_processed(&mut self, summary: String) {
        self.summary = Some(summary);
        self.processed = true;
    }
}

/// Input for creating a new event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEventInput {
    /// System profile ID - which business system this event belongs to
    pub profile_id: Uuid,
    /// Owner ID - user-defined, not validated semantically
    pub owner_id: String,
    /// Scope identifier - user-defined, null = global context
    pub scope_id: Option<String>,
    /// Raw event content
    pub content: String,
    /// Optional context
    pub context: Option<String>,
    /// Event source (e.g., "user_created", "api", "conversation")
    pub source: Option<String>,
    /// When the event occurred (defaults to now)
    pub event_time: Option<DateTime<Utc>>,
}

/// Validation for event creation
#[derive(Debug)]
pub struct CreateEventValidation;

impl CreateEventValidation {
    /// Validate an event creation request
    ///
    /// Validation rules:
    /// - owner_id cannot be empty
    /// - content cannot be empty
    pub fn validate(input: &CreateEventInput) -> Result<(), AppError> {
        if input.owner_id.trim().is_empty() {
            return Err(AppError::Validation("owner_id cannot be empty".to_string()));
        }

        if input.content.trim().is_empty() {
            return Err(AppError::Validation("content cannot be empty".to_string()));
        }

        Ok(())
    }
}

/// Relation between an event and a memory (completely immutable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMemoryRelation {
    /// Unique identifier
    pub id: Uuid,
    /// Event ID
    pub event_id: Uuid,
    /// Memory ID
    pub memory_id: Uuid,
    /// Type of relation
    pub relation_type: EventMemoryRelationType,
    /// When the relation was created
    pub created_at: DateTime<Utc>,
}

impl EventMemoryRelation {
    /// Create a new relation
    pub fn new(event_id: Uuid, memory_id: Uuid, relation_type: EventMemoryRelationType) -> Self {
        EventMemoryRelation {
            id: Uuid::new_v4(),
            event_id,
            memory_id,
            relation_type,
            created_at: Utc::now(),
        }
    }

    /// Create a "created_from" relation
    pub fn created_from(event_id: Uuid, memory_id: Uuid) -> Self {
        Self::new(event_id, memory_id, EventMemoryRelationType::CreatedFrom)
    }

    /// Create a "reinforced_by" relation
    pub fn reinforced_by(event_id: Uuid, memory_id: Uuid) -> Self {
        Self::new(event_id, memory_id, EventMemoryRelationType::ReinforcedBy)
    }
}

/// Criteria for promoting a memory to global
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionCriteria {
    /// Minimum number of different scopes that reinforced the memory
    pub min_scope_diversity: i32,
    /// Minimum number of reinforcements
    pub min_reinforcements: i32,
    /// Minimum confidence score
    pub min_confidence: f32,
    /// Minimum age in hours before promotion is considered
    pub min_age_hours: i64,
}

impl Default for PromotionCriteria {
    fn default() -> Self {
        PromotionCriteria {
            min_scope_diversity: 2,
            min_reinforcements: 3,
            min_confidence: 0.7,
            min_age_hours: 24,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_event_input() -> CreateEventInput {
        CreateEventInput {
            profile_id: Uuid::new_v4(),
            owner_id: "owner123".to_string(),
            scope_id: Some("scope456".to_string()),
            content: "User clicked dark mode button".to_string(),
            context: Some("Settings page".to_string()),
            source: Some("user_action".to_string()),
            event_time: None,
        }
    }

    #[test]
    fn test_event_creation() {
        let input = valid_event_input();
        let profile_id = input.profile_id;
        let event = Event::new(input);

        assert_eq!(event.profile_id, profile_id);
        assert_eq!(event.owner_id, "owner123");
        assert_eq!(event.scope_id, Some("scope456".to_string()));
        assert_eq!(event.content, "User clicked dark mode button");
        assert_eq!(event.context, Some("Settings page".to_string()));
        assert_eq!(event.source, Some("user_action".to_string()));
        assert!(!event.processed);
        assert!(event.summary.is_none());
    }

    #[test]
    fn test_event_creation_without_scope() {
        let input = CreateEventInput {
            profile_id: Uuid::new_v4(),
            owner_id: "owner123".to_string(),
            scope_id: None,
            content: "Global event".to_string(),
            context: None,
            source: None,
            event_time: None,
        };

        let event = Event::new(input);
        assert!(event.scope_id.is_none());
    }

    #[test]
    fn test_event_mark_processed() {
        let input = valid_event_input();
        let mut event = Event::new(input);

        event.mark_processed("User changed theme preference".to_string());

        assert!(event.processed);
        assert_eq!(
            event.summary,
            Some("User changed theme preference".to_string())
        );
    }

    #[test]
    fn test_event_validation_valid() {
        let input = valid_event_input();
        let result = CreateEventValidation::validate(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_event_validation_empty_owner_id() {
        let mut input = valid_event_input();
        input.owner_id = "".to_string();
        let result = CreateEventValidation::validate(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_event_validation_empty_content() {
        let mut input = valid_event_input();
        input.content = "".to_string();
        let result = CreateEventValidation::validate(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_relation_type_display() {
        assert_eq!(
            EventMemoryRelationType::CreatedFrom.to_string(),
            "created_from"
        );
        assert_eq!(
            EventMemoryRelationType::ReinforcedBy.to_string(),
            "reinforced_by"
        );
    }

    #[test]
    fn test_relation_type_from_str() {
        assert_eq!(
            "created_from".parse::<EventMemoryRelationType>().unwrap(),
            EventMemoryRelationType::CreatedFrom
        );
        assert_eq!(
            "reinforced_by".parse::<EventMemoryRelationType>().unwrap(),
            EventMemoryRelationType::ReinforcedBy
        );
        assert!("unknown".parse::<EventMemoryRelationType>().is_err());
    }

    #[test]
    fn test_event_memory_relation_creation() {
        let event_id = Uuid::new_v4();
        let memory_id = Uuid::new_v4();

        let relation =
            EventMemoryRelation::new(event_id, memory_id, EventMemoryRelationType::ReinforcedBy);

        assert_eq!(relation.event_id, event_id);
        assert_eq!(relation.memory_id, memory_id);
        assert_eq!(
            relation.relation_type,
            EventMemoryRelationType::ReinforcedBy
        );
    }

    #[test]
    fn test_event_memory_relation_helpers() {
        let event_id = Uuid::new_v4();
        let memory_id = Uuid::new_v4();

        let created = EventMemoryRelation::created_from(event_id, memory_id);
        assert_eq!(created.relation_type, EventMemoryRelationType::CreatedFrom);

        let reinforced = EventMemoryRelation::reinforced_by(event_id, memory_id);
        assert_eq!(
            reinforced.relation_type,
            EventMemoryRelationType::ReinforcedBy
        );
    }

    #[test]
    fn test_promotion_criteria_default() {
        let criteria = PromotionCriteria::default();

        assert_eq!(criteria.min_scope_diversity, 2);
        assert_eq!(criteria.min_reinforcements, 3);
        assert_eq!(criteria.min_confidence, 0.7);
        assert_eq!(criteria.min_age_hours, 24);
    }
}
