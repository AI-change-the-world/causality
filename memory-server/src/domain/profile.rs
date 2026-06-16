//! SystemProfile entity and validation logic
//!
//! SystemProfile is the core configuration entity for the Memory System,
//! defining business boundaries, target audience, and behavior guidelines.
//! Each profile is a business-system namespace for memories and events.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;

/// Maximum allowed length for the system name
pub const MAX_NAME_LENGTH: usize = 100;

/// SystemProfile entity representing the system's configuration and boundaries
///
/// Each profile represents one business system's memory namespace.
/// It guides LLM processing for memory extraction and event classification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemProfile {
    /// Unique identifier
    pub id: Uuid,
    /// System name (max 100 characters)
    pub name: String,
    /// Original user description (raw input)
    pub description: String,
    /// System purpose (LLM parsed)
    pub purpose: String,
    /// Business domain (LLM parsed)
    pub domain: String,
    /// Target audience (LLM parsed)
    pub target_audience: String,
    /// Valid event categories (LLM parsed)
    pub event_categories: Vec<String>,
    /// Memory types to prioritize (LLM parsed)
    pub memory_focus: Vec<String>,
    /// Things the system should not handle (LLM parsed)
    pub boundaries: Vec<String>,
    /// Complete prompt template for event extraction (LLM generated)
    pub extraction_prompt: String,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a new SystemProfile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProfileInput {
    /// System name (max 100 characters)
    pub name: String,
    /// User's natural language description of the system
    pub description: String,
}

/// Parsed profile fields from LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedProfile {
    /// System purpose
    pub purpose: String,
    /// Business domain
    pub domain: String,
    /// Target audience
    pub target_audience: String,
    /// Event categories
    pub event_categories: Vec<String>,
    /// Memory focus areas
    pub memory_focus: Vec<String>,
    /// System boundaries
    pub boundaries: Vec<String>,
    /// Extraction prompt template
    pub extraction_prompt: String,
}

/// Input for updating a SystemProfile
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateProfileInput {
    /// New system name (optional)
    pub name: Option<String>,
    /// New description (optional, triggers re-parse if reparse=true)
    pub description: Option<String>,
    /// New purpose (optional)
    pub purpose: Option<String>,
    /// New domain (optional)
    pub domain: Option<String>,
    /// New target audience (optional)
    pub target_audience: Option<String>,
    /// New event categories (optional)
    pub event_categories: Option<Vec<String>>,
    /// New memory focus (optional)
    pub memory_focus: Option<Vec<String>>,
    /// New boundaries (optional)
    pub boundaries: Option<Vec<String>>,
    /// New extraction prompt (optional)
    pub extraction_prompt: Option<String>,
    /// Whether to re-parse description with LLM
    #[serde(default)]
    pub reparse: bool,
}

/// Validation for SystemProfile creation and updates
#[derive(Debug)]
pub struct ProfileValidation;

impl ProfileValidation {
    /// Validate a profile creation request
    ///
    /// Validation rules:
    /// - name cannot be empty
    /// - name must be <= 100 characters
    /// - description cannot be empty
    pub fn validate_create(input: &CreateProfileInput) -> Result<(), AppError> {
        Self::validate_name(&input.name)?;

        if input.description.trim().is_empty() {
            return Err(AppError::Validation(
                "description cannot be empty".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate a profile update request
    ///
    /// Validation rules:
    /// - if name is provided, it must be <= 100 characters and not empty
    pub fn validate_update(input: &UpdateProfileInput) -> Result<(), AppError> {
        if let Some(ref name) = input.name {
            Self::validate_name(name)?;
        }
        Ok(())
    }

    /// Validate a name field
    fn validate_name(name: &str) -> Result<(), AppError> {
        if name.trim().is_empty() {
            return Err(AppError::Validation("name cannot be empty".to_string()));
        }

        if name.len() > MAX_NAME_LENGTH {
            return Err(AppError::Validation(format!(
                "name must be {} characters or less",
                MAX_NAME_LENGTH
            )));
        }

        Ok(())
    }
}

impl SystemProfile {
    /// Create a new SystemProfile from validated input and parsed fields
    pub fn new(input: CreateProfileInput, parsed: ParsedProfile) -> Self {
        let now = Utc::now();
        SystemProfile {
            id: Uuid::new_v4(),
            name: input.name,
            description: input.description,
            purpose: parsed.purpose,
            domain: parsed.domain,
            target_audience: parsed.target_audience,
            event_categories: parsed.event_categories,
            memory_focus: parsed.memory_focus,
            boundaries: parsed.boundaries,
            extraction_prompt: parsed.extraction_prompt,
            created_at: now,
            updated_at: now,
        }
    }

    /// Apply a partial update to this profile
    ///
    /// Only updates fields that are Some in the input.
    /// Always updates the updated_at timestamp.
    pub fn apply_update(&mut self, input: UpdateProfileInput) {
        if let Some(name) = input.name {
            self.name = name;
        }
        if let Some(description) = input.description {
            self.description = description;
        }
        if let Some(purpose) = input.purpose {
            self.purpose = purpose;
        }
        if let Some(domain) = input.domain {
            self.domain = domain;
        }
        if let Some(target_audience) = input.target_audience {
            self.target_audience = target_audience;
        }
        if let Some(event_categories) = input.event_categories {
            self.event_categories = event_categories;
        }
        if let Some(memory_focus) = input.memory_focus {
            self.memory_focus = memory_focus;
        }
        if let Some(boundaries) = input.boundaries {
            self.boundaries = boundaries;
        }
        if let Some(extraction_prompt) = input.extraction_prompt {
            self.extraction_prompt = extraction_prompt;
        }
        self.updated_at = Utc::now();
    }

    /// Apply parsed fields from LLM re-parsing
    pub fn apply_parsed(&mut self, parsed: ParsedProfile) {
        self.purpose = parsed.purpose;
        self.domain = parsed.domain;
        self.target_audience = parsed.target_audience;
        self.event_categories = parsed.event_categories;
        self.memory_focus = parsed.memory_focus;
        self.boundaries = parsed.boundaries;
        self.extraction_prompt = parsed.extraction_prompt;
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_create_input() -> CreateProfileInput {
        CreateProfileInput {
            name: "房产推荐系统".to_string(),
            description: "这是一个房产推荐系统，帮助购房者找到合适的房源".to_string(),
        }
    }

    fn valid_parsed_profile() -> ParsedProfile {
        ParsedProfile {
            purpose: "房产推荐".to_string(),
            domain: "房地产".to_string(),
            target_audience: "购房者".to_string(),
            event_categories: vec!["咨询".to_string(), "看房".to_string(), "成交".to_string()],
            memory_focus: vec!["购房偏好".to_string(), "预算范围".to_string()],
            boundaries: vec!["不处理租房".to_string()],
            extraction_prompt: "请将用户行为解析为结构化格式...".to_string(),
        }
    }

    #[test]
    fn test_valid_profile_creation() {
        let input = valid_create_input();
        let result = ProfileValidation::validate_create(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_name_rejection() {
        let mut input = valid_create_input();
        input.name = "".to_string();
        let result = ProfileValidation::validate_create(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_whitespace_name_rejection() {
        let mut input = valid_create_input();
        input.name = "   ".to_string();
        let result = ProfileValidation::validate_create(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_name_too_long_rejection() {
        let mut input = valid_create_input();
        input.name = "a".repeat(101);
        let result = ProfileValidation::validate_create(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_name_at_max_length_accepted() {
        let mut input = valid_create_input();
        input.name = "a".repeat(100);
        let result = ProfileValidation::validate_create(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_description_rejection() {
        let mut input = valid_create_input();
        input.description = "".to_string();
        let result = ProfileValidation::validate_create(&input);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_profile_new() {
        let input = valid_create_input();
        let parsed = valid_parsed_profile();
        let profile = SystemProfile::new(input.clone(), parsed.clone());

        assert_eq!(profile.name, input.name);
        assert_eq!(profile.description, input.description);
        assert_eq!(profile.purpose, parsed.purpose);
        assert_eq!(profile.domain, parsed.domain);
        assert_eq!(profile.target_audience, parsed.target_audience);
        assert_eq!(profile.event_categories, parsed.event_categories);
        assert_eq!(profile.memory_focus, parsed.memory_focus);
        assert_eq!(profile.boundaries, parsed.boundaries);
        assert_eq!(profile.extraction_prompt, parsed.extraction_prompt);
    }

    #[test]
    fn test_profile_partial_update() {
        let input = valid_create_input();
        let parsed = valid_parsed_profile();
        let mut profile = SystemProfile::new(input, parsed);
        let original_purpose = profile.purpose.clone();
        let original_updated_at = profile.updated_at;

        // Wait a tiny bit to ensure timestamp changes
        std::thread::sleep(std::time::Duration::from_millis(10));

        let update = UpdateProfileInput {
            name: Some("新系统名称".to_string()),
            event_categories: Some(vec!["新分类".to_string()]),
            ..Default::default()
        };

        profile.apply_update(update);

        assert_eq!(profile.name, "新系统名称");
        assert_eq!(profile.event_categories, vec!["新分类".to_string()]);
        // Unchanged fields should remain the same
        assert_eq!(profile.purpose, original_purpose);
        // updated_at should be updated
        assert!(profile.updated_at > original_updated_at);
    }

    #[test]
    fn test_profile_apply_parsed() {
        let input = valid_create_input();
        let parsed = valid_parsed_profile();
        let mut profile = SystemProfile::new(input, parsed);
        let original_updated_at = profile.updated_at;

        std::thread::sleep(std::time::Duration::from_millis(10));

        let new_parsed = ParsedProfile {
            purpose: "新用途".to_string(),
            domain: "新领域".to_string(),
            target_audience: "新受众".to_string(),
            event_categories: vec!["新分类1".to_string()],
            memory_focus: vec!["新关注点".to_string()],
            boundaries: vec!["新边界".to_string()],
            extraction_prompt: "新提取prompt".to_string(),
        };

        profile.apply_parsed(new_parsed.clone());

        assert_eq!(profile.purpose, new_parsed.purpose);
        assert_eq!(profile.domain, new_parsed.domain);
        assert_eq!(profile.target_audience, new_parsed.target_audience);
        assert_eq!(profile.event_categories, new_parsed.event_categories);
        assert_eq!(profile.memory_focus, new_parsed.memory_focus);
        assert_eq!(profile.boundaries, new_parsed.boundaries);
        assert_eq!(profile.extraction_prompt, new_parsed.extraction_prompt);
        assert!(profile.updated_at > original_updated_at);
    }

    #[test]
    fn test_update_validation_with_long_name() {
        let update = UpdateProfileInput {
            name: Some("a".repeat(101)),
            ..Default::default()
        };
        let result = ProfileValidation::validate_update(&update);
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_update_validation_with_valid_name() {
        let update = UpdateProfileInput {
            name: Some("Valid Name".to_string()),
            ..Default::default()
        };
        let result = ProfileValidation::validate_update(&update);
        assert!(result.is_ok());
    }

    #[test]
    fn test_profile_serialization_roundtrip() {
        let input = valid_create_input();
        let parsed = valid_parsed_profile();
        let profile = SystemProfile::new(input, parsed);

        // Serialize to JSON
        let json = serde_json::to_string(&profile).expect("Failed to serialize");

        // Deserialize back
        let deserialized: SystemProfile =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(profile, deserialized);
    }
}
