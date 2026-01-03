//! Event entity and related types
//!
//! Events represent raw occurrences (clicks, conversations, operations) that
//! can be processed to extract or reinforce memories.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::fmt;
use uuid::Uuid;

use super::ScopeType;

/// Relation type between an event and a memory
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[sqlx(type_name = "event_memory_relation_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EventMemoryRelationType {
    /// Memory was created from this event
    CreatedFrom,
    /// Memory was reinforced by this event (evidence accumulation)
    ReinforcedBy,
    /// Memory was contradicted by this event
    ContradictedBy,
}

impl fmt::Display for EventMemoryRelationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventMemoryRelationType::CreatedFrom => write!(f, "created_from"),
            EventMemoryRelationType::ReinforcedBy => write!(f, "reinforced_by"),
            EventMemoryRelationType::ContradictedBy => write!(f, "contradicted_by"),
        }
    }
}

/// Event entity representing a raw occurrence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Unique identifier
    pub id: Uuid,
    /// Owner ID - the unique identifier of the event owner
    pub owner_id: String,
    /// Raw event content
    pub content: String,
    /// Optional context to help understand the event
    pub context: Option<String>,
    /// Event type (e.g., "click", "conversation", "operation")
    pub event_type: Option<String>,
    /// Scope type for related memories
    pub scope_type: ScopeType,
    /// Scope identifier
    pub scope_id: String,
    /// Usage scene
    pub scene: String,
    /// LLM-generated summary of the event
    pub summary: Option<String>,
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
            owner_id: input.owner_id,
            content: input.content,
            context: input.context,
            event_type: input.event_type,
            scope_type: input.scope_type,
            scope_id: input.scope_id,
            scene: input.scene,
            summary: None,
            processed: false,
            event_time: input.event_time.unwrap_or(now),
            created_at: now,
        }
    }

    /// Mark the event as processed with a summary
    pub fn mark_processed(&mut self, summary: String) {
        self.summary = Some(summary);
        self.processed = true;
    }
}

/// Input for creating a new event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEventInput {
    /// Owner ID
    pub owner_id: String,
    /// Raw event content
    pub content: String,
    /// Optional context
    pub context: Option<String>,
    /// Event type
    pub event_type: Option<String>,
    /// Scope type
    pub scope_type: ScopeType,
    /// Scope identifier
    pub scope_id: String,
    /// Usage scene
    pub scene: String,
    /// When the event occurred (defaults to now)
    pub event_time: Option<DateTime<Utc>>,
}

/// Relation between an event and a memory
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
    /// Similarity score when matching (for reinforced_by relations)
    pub similarity_score: Option<f32>,
    /// When the relation was created
    pub created_at: DateTime<Utc>,
}

impl EventMemoryRelation {
    /// Create a new relation
    pub fn new(
        event_id: Uuid,
        memory_id: Uuid,
        relation_type: EventMemoryRelationType,
        similarity_score: Option<f32>,
    ) -> Self {
        EventMemoryRelation {
            id: Uuid::new_v4(),
            event_id,
            memory_id,
            relation_type,
            similarity_score,
            created_at: Utc::now(),
        }
    }
}

/// Criteria for promoting a memory to long-term
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionCriteria {
    /// Minimum number of supporting events
    pub min_evidence_count: i32,
    /// Minimum confidence score
    pub min_confidence: f32,
    /// Minimum number of different scopes that reinforced the memory
    pub min_scope_diversity: i32,
    /// Minimum age in hours before promotion is considered
    pub min_age_hours: i64,
}

impl Default for PromotionCriteria {
    fn default() -> Self {
        PromotionCriteria {
            min_evidence_count: 3,
            min_confidence: 0.7,
            min_scope_diversity: 2,
            min_age_hours: 24,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_creation() {
        let input = CreateEventInput {
            owner_id: "owner123".to_string(),
            content: "User clicked dark mode button".to_string(),
            context: Some("Settings page".to_string()),
            event_type: Some("click".to_string()),
            scope_type: ScopeType::User,
            scope_id: "user123".to_string(),
            scene: "settings.preferences".to_string(),
            event_time: None,
        };

        let event = Event::new(input);

        assert_eq!(event.owner_id, "owner123");
        assert_eq!(event.content, "User clicked dark mode button");
        assert_eq!(event.context, Some("Settings page".to_string()));
        assert!(!event.processed);
        assert!(event.summary.is_none());
    }

    #[test]
    fn test_event_mark_processed() {
        let input = CreateEventInput {
            owner_id: "owner123".to_string(),
            content: "Test content".to_string(),
            context: None,
            event_type: None,
            scope_type: ScopeType::User,
            scope_id: "user123".to_string(),
            scene: "test".to_string(),
            event_time: None,
        };

        let mut event = Event::new(input);
        event.mark_processed("User changed theme preference".to_string());

        assert!(event.processed);
        assert_eq!(
            event.summary,
            Some("User changed theme preference".to_string())
        );
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
        assert_eq!(
            EventMemoryRelationType::ContradictedBy.to_string(),
            "contradicted_by"
        );
    }

    #[test]
    fn test_event_memory_relation_creation() {
        let event_id = Uuid::new_v4();
        let memory_id = Uuid::new_v4();

        let relation = EventMemoryRelation::new(
            event_id,
            memory_id,
            EventMemoryRelationType::ReinforcedBy,
            Some(0.85),
        );

        assert_eq!(relation.event_id, event_id);
        assert_eq!(relation.memory_id, memory_id);
        assert_eq!(
            relation.relation_type,
            EventMemoryRelationType::ReinforcedBy
        );
        assert_eq!(relation.similarity_score, Some(0.85));
    }

    #[test]
    fn test_promotion_criteria_default() {
        let criteria = PromotionCriteria::default();

        assert_eq!(criteria.min_evidence_count, 3);
        assert_eq!(criteria.min_confidence, 0.7);
        assert_eq!(criteria.min_scope_diversity, 2);
        assert_eq!(criteria.min_age_hours, 24);
    }
}
