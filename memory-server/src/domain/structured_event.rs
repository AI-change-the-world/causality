//! StructuredEvent entity - Six Elements Model (六要素模型)
//!
//! StructuredEvent represents an event that has been parsed into
//! a standardized format with six core elements plus two auxiliary elements:
//!
//! Core Elements (六要素):
//! - Time (时间): When the event occurred
//! - Location (地点): Device, page, content position
//! - Actor (人物): User identifier
//! - Cause (起因): Why the event was triggered
//! - Process (经过): What happened step by step
//! - Result (结果): Outcome of the event
//!
//! Auxiliary Elements (两辅助):
//! - Background (背景): Context/scenario
//! - Details (细节): Key details, follow-up actions

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// StructuredEvent - Six Elements + Two Auxiliary
///
/// This entity represents an event that has been parsed by LLM
/// into a standardized structure for systematic memory extraction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredEvent {
    /// Unique identifier
    pub id: Uuid,
    /// Reference to the original event
    pub event_id: Uuid,

    // === Six Core Elements (六要素) ===
    /// Time element: When the event occurred
    pub time_element: Option<String>,
    /// Location element: Device, page, content position
    pub location_element: Option<String>,
    /// Actor element: User identifier (required)
    pub actor_element: String,
    /// Cause element: Why the event was triggered (inferred from scope)
    pub cause_element: Option<String>,
    /// Process element: What happened step by step
    pub process_element: Option<String>,
    /// Result element: Outcome of the event
    pub result_element: Option<String>,

    // === Two Auxiliary Elements (两辅助) ===
    /// Background auxiliary: Context/scenario
    pub background_element: Option<String>,
    /// Details auxiliary: Key details, follow-up actions
    pub details_element: Option<String>,

    // === Classification ===
    /// Event category (based on SystemProfile.event_categories)
    pub category: Option<String>,

    // === Timestamp ===
    /// When this structured event was created
    pub created_at: DateTime<Utc>,
}

/// Input for creating a StructuredEvent from LLM parsing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateStructuredEventInput {
    /// Reference to the original event
    pub event_id: Uuid,
    /// Time element
    pub time_element: Option<String>,
    /// Location element
    pub location_element: Option<String>,
    /// Actor element (required)
    pub actor_element: String,
    /// Cause element
    pub cause_element: Option<String>,
    /// Process element
    pub process_element: Option<String>,
    /// Result element
    pub result_element: Option<String>,
    /// Background element
    pub background_element: Option<String>,
    /// Details element
    pub details_element: Option<String>,
    /// Category
    pub category: Option<String>,
}

/// LLM-parsed structured event fields
///
/// This is the format expected from LLM when parsing raw event content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedStructuredEvent {
    /// Time element
    pub time: Option<String>,
    /// Location element
    pub location: Option<String>,
    /// Actor element
    pub actor: String,
    /// Cause element
    pub cause: Option<String>,
    /// Process element
    pub process: Option<String>,
    /// Result element
    pub result: Option<String>,
    /// Background element
    pub background: Option<String>,
    /// Details element
    pub details: Option<String>,
    /// Category
    pub category: Option<String>,
}

impl StructuredEvent {
    /// Create a new StructuredEvent from input
    pub fn new(input: CreateStructuredEventInput) -> Self {
        StructuredEvent {
            id: Uuid::new_v4(),
            event_id: input.event_id,
            time_element: input.time_element,
            location_element: input.location_element,
            actor_element: input.actor_element,
            cause_element: input.cause_element,
            process_element: input.process_element,
            result_element: input.result_element,
            background_element: input.background_element,
            details_element: input.details_element,
            category: input.category,
            created_at: Utc::now(),
        }
    }

    /// Create a StructuredEvent from LLM-parsed fields
    pub fn from_parsed(event_id: Uuid, parsed: ParsedStructuredEvent) -> Self {
        StructuredEvent {
            id: Uuid::new_v4(),
            event_id,
            time_element: parsed.time,
            location_element: parsed.location,
            actor_element: parsed.actor,
            cause_element: parsed.cause,
            process_element: parsed.process,
            result_element: parsed.result,
            background_element: parsed.background,
            details_element: parsed.details,
            category: parsed.category,
            created_at: Utc::now(),
        }
    }

    /// Check if this structured event has minimal required information
    ///
    /// Returns true if at least the actor and one other element is present.
    pub fn has_minimal_info(&self) -> bool {
        !self.actor_element.trim().is_empty()
            && (self.time_element.is_some()
                || self.location_element.is_some()
                || self.cause_element.is_some()
                || self.process_element.is_some()
                || self.result_element.is_some()
                || self.background_element.is_some()
                || self.details_element.is_some())
    }

    /// Get a summary of the structured event for logging/debugging
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("actor={}", self.actor_element)];

        if let Some(ref time) = self.time_element {
            parts.push(format!("time={}", time));
        }
        if let Some(ref location) = self.location_element {
            parts.push(format!("location={}", location));
        }
        if let Some(ref category) = self.category {
            parts.push(format!("category={}", category));
        }

        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_input() -> CreateStructuredEventInput {
        CreateStructuredEventInput {
            event_id: Uuid::new_v4(),
            time_element: Some("2024-01-15 14:30".to_string()),
            location_element: Some("房源详情页".to_string()),
            actor_element: "user123".to_string(),
            cause_element: Some("用户对房源不满意".to_string()),
            process_element: Some("点击了不喜欢按钮".to_string()),
            result_element: Some("房源被标记为不喜欢".to_string()),
            background_element: Some("用户正在浏览推荐房源".to_string()),
            details_element: Some("选择原因：价格过高".to_string()),
            category: Some("反馈".to_string()),
        }
    }

    fn valid_parsed() -> ParsedStructuredEvent {
        ParsedStructuredEvent {
            time: Some("2024-01-15 14:30".to_string()),
            location: Some("房源详情页".to_string()),
            actor: "user123".to_string(),
            cause: Some("用户对房源不满意".to_string()),
            process: Some("点击了不喜欢按钮".to_string()),
            result: Some("房源被标记为不喜欢".to_string()),
            background: Some("用户正在浏览推荐房源".to_string()),
            details: Some("选择原因：价格过高".to_string()),
            category: Some("反馈".to_string()),
        }
    }

    #[test]
    fn test_structured_event_creation() {
        let input = valid_input();
        let event_id = input.event_id;
        let event = StructuredEvent::new(input);

        assert_eq!(event.event_id, event_id);
        assert_eq!(event.actor_element, "user123");
        assert_eq!(event.time_element, Some("2024-01-15 14:30".to_string()));
        assert_eq!(event.location_element, Some("房源详情页".to_string()));
        assert_eq!(event.cause_element, Some("用户对房源不满意".to_string()));
        assert_eq!(event.process_element, Some("点击了不喜欢按钮".to_string()));
        assert_eq!(event.result_element, Some("房源被标记为不喜欢".to_string()));
        assert_eq!(
            event.background_element,
            Some("用户正在浏览推荐房源".to_string())
        );
        assert_eq!(
            event.details_element,
            Some("选择原因：价格过高".to_string())
        );
        assert_eq!(event.category, Some("反馈".to_string()));
    }

    #[test]
    fn test_structured_event_from_parsed() {
        let event_id = Uuid::new_v4();
        let parsed = valid_parsed();
        let event = StructuredEvent::from_parsed(event_id, parsed);

        assert_eq!(event.event_id, event_id);
        assert_eq!(event.actor_element, "user123");
        assert_eq!(event.time_element, Some("2024-01-15 14:30".to_string()));
    }

    #[test]
    fn test_structured_event_with_minimal_fields() {
        let input = CreateStructuredEventInput {
            event_id: Uuid::new_v4(),
            time_element: None,
            location_element: None,
            actor_element: "user123".to_string(),
            cause_element: None,
            process_element: Some("用户执行了某操作".to_string()),
            result_element: None,
            background_element: None,
            details_element: None,
            category: None,
        };

        let event = StructuredEvent::new(input);
        assert!(event.has_minimal_info());
    }

    #[test]
    fn test_structured_event_without_minimal_info() {
        let input = CreateStructuredEventInput {
            event_id: Uuid::new_v4(),
            time_element: None,
            location_element: None,
            actor_element: "user123".to_string(),
            cause_element: None,
            process_element: None,
            result_element: None,
            background_element: None,
            details_element: None,
            category: None,
        };

        let event = StructuredEvent::new(input);
        assert!(!event.has_minimal_info());
    }

    #[test]
    fn test_structured_event_empty_actor_not_minimal() {
        let input = CreateStructuredEventInput {
            event_id: Uuid::new_v4(),
            time_element: Some("2024-01-15".to_string()),
            location_element: None,
            actor_element: "   ".to_string(), // whitespace only
            cause_element: None,
            process_element: None,
            result_element: None,
            background_element: None,
            details_element: None,
            category: None,
        };

        let event = StructuredEvent::new(input);
        assert!(!event.has_minimal_info());
    }

    #[test]
    fn test_structured_event_summary() {
        let input = valid_input();
        let event = StructuredEvent::new(input);
        let summary = event.summary();

        assert!(summary.contains("actor=user123"));
        assert!(summary.contains("time=2024-01-15 14:30"));
        assert!(summary.contains("location=房源详情页"));
        assert!(summary.contains("category=反馈"));
    }

    #[test]
    fn test_structured_event_serialization_roundtrip() {
        let input = valid_input();
        let event = StructuredEvent::new(input);

        // Serialize to JSON
        let json = serde_json::to_string(&event).expect("Failed to serialize");

        // Deserialize back
        let deserialized: StructuredEvent =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(event, deserialized);
    }

    #[test]
    fn test_parsed_structured_event_serialization() {
        let parsed = valid_parsed();

        // Serialize to JSON
        let json = serde_json::to_string(&parsed).expect("Failed to serialize");

        // Deserialize back
        let deserialized: ParsedStructuredEvent =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(parsed.actor, deserialized.actor);
        assert_eq!(parsed.time, deserialized.time);
        assert_eq!(parsed.category, deserialized.category);
    }
}
