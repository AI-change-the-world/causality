//! ProfileService implementation
//!
//! Provides business logic for SystemProfile management including:
//! - System initialization with LLM-based description parsing
//! - Profile retrieval
//! - Partial and full profile updates

use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::domain::{
    CreateProfileInput, ParsedProfile, ProfileValidation, SystemProfile, UpdateProfileInput,
};
use crate::error::{AppError, AppResult};
use crate::llm::{ChatRequest, LlmProvider};
use crate::repository::ProfileRepository;

/// Default prompt template for parsing system description into structured profile
const PARSE_PROFILE_PROMPT: &str = r#"你是一个系统配置助手。请根据用户提供的系统描述，提取以下结构化信息：

用户描述：
{description}

请以 JSON 格式返回以下字段：
```json
{
  "purpose": "系统的主要用途（简短描述）",
  "domain": "业务领域（如：房地产、电商、金融等）",
  "target_audience": "目标用户群体",
  "event_categories": ["可能的事件类型列表"],
  "memory_focus": ["需要关注的记忆类型"],
  "boundaries": ["系统不处理的内容"],
  "extraction_prompt": "一段完整的 prompt，用于指导 LLM 将原始事件解析为六要素格式（时间、地点、人物、起因、经过、结果、背景、细节）。这个 prompt 应该针对该业务领域定制，包含具体的提取指导和示例。"
}
```

注意：
- event_categories 应该是该业务场景下常见的用户行为/事件类型
- memory_focus 应该是对该业务有价值的用户信息类型
- boundaries 应该明确系统的边界，避免功能发散
- extraction_prompt 应该是一个完整的、可直接使用的 prompt 模板，包含该领域的具体提取指导"#;

/// Service for managing SystemProfile
///
/// Handles business logic for profile initialization, retrieval, and updates.
/// Uses LLM to parse natural language descriptions into structured profile fields.
#[derive(Clone)]
pub struct ProfileService {
    /// Repository for profile persistence
    repo: ProfileRepository,
    /// LLM provider for parsing descriptions
    llm_provider: Arc<dyn LlmProvider>,
}

impl ProfileService {
    /// Create a new ProfileService
    pub fn new(repo: ProfileRepository, llm_provider: Arc<dyn LlmProvider>) -> Self {
        Self { repo, llm_provider }
    }

    /// Initialize the system with a natural language description
    ///
    /// Uses LLM to parse the description into structured profile fields.
    /// Returns an error if a profile already exists.
    ///
    /// # Arguments
    /// * `input` - The creation input containing name and description
    ///
    /// # Returns
    /// * `Ok(SystemProfile)` - The created profile
    /// * `Err(AppError::Validation)` - If profile already exists or validation fails
    pub async fn initialize(&self, input: CreateProfileInput) -> AppResult<SystemProfile> {
        debug!(name = %input.name, "Initializing system profile");

        // Validate input
        ProfileValidation::validate_create(&input)?;

        // Check if profile already exists
        if self.repo.exists().await? {
            return Err(AppError::Validation(
                "System profile already exists. Use update instead.".to_string(),
            ));
        }

        // Parse description using LLM
        let parsed = self.parse_description(&input.description).await?;

        // Create the profile
        let profile = SystemProfile::new(input, parsed);

        // Persist to database
        let created = self.repo.create(&profile).await?;

        info!(id = %created.id, name = %created.name, "System profile initialized");

        Ok(created)
    }

    /// Get the current system profile
    ///
    /// # Returns
    /// * `Ok(SystemProfile)` - The current profile
    /// * `Err(AppError::Validation)` - If no profile exists (404)
    pub async fn get(&self) -> AppResult<SystemProfile> {
        debug!("Getting system profile");

        self.repo.get().await?.ok_or_else(|| {
            AppError::Validation("System profile not found. Initialize first.".to_string())
        })
    }

    /// Update the system profile
    ///
    /// Supports two modes:
    /// 1. Partial update: Only updates provided fields
    /// 2. Re-parse: If `reparse=true` and description is provided, uses LLM to re-parse
    ///
    /// # Arguments
    /// * `input` - The update input with optional fields
    ///
    /// # Returns
    /// * `Ok(SystemProfile)` - The updated profile
    /// * `Err(AppError::Validation)` - If no profile exists or validation fails
    pub async fn update(&self, input: UpdateProfileInput) -> AppResult<SystemProfile> {
        debug!(reparse = input.reparse, "Updating system profile");

        // Validate input
        ProfileValidation::validate_update(&input)?;

        // Get existing profile
        let mut profile = self.get().await?;

        // Handle re-parse mode
        if input.reparse {
            if let Some(ref description) = input.description {
                let parsed = self.parse_description(description).await?;
                profile.description = description.clone();
                profile.apply_parsed(parsed);
            } else {
                // Re-parse with existing description
                let parsed = self.parse_description(&profile.description).await?;
                profile.apply_parsed(parsed);
            }
        } else {
            // Partial update mode
            profile.apply_update(input);
        }

        // Persist changes
        let updated = self.repo.update(&profile).await?;

        info!(id = %updated.id, "System profile updated");

        Ok(updated)
    }

    /// Parse a natural language description into structured profile fields using LLM
    ///
    /// # Arguments
    /// * `description` - The natural language description to parse
    ///
    /// # Returns
    /// * `Ok(ParsedProfile)` - The parsed profile fields
    /// * `Err(AppError::Internal)` - If LLM parsing fails
    async fn parse_description(&self, description: &str) -> AppResult<ParsedProfile> {
        debug!(
            description_len = description.len(),
            "Parsing description with LLM"
        );

        let prompt = PARSE_PROFILE_PROMPT.replace("{description}", description);

        let chat_request = ChatRequest::new(prompt)
            .with_temperature(0.3)
            .with_json_response();

        let response = self.llm_provider.chat(chat_request).await.map_err(|e| {
            warn!(error = %e, "LLM parsing failed");
            AppError::Internal(format!("Failed to parse description: {}", e))
        })?;

        let parsed = Self::parse_llm_response(&response.content)?;

        info!(
            purpose = %parsed.purpose,
            domain = %parsed.domain,
            event_categories_count = parsed.event_categories.len(),
            "Description parsed successfully"
        );

        Ok(parsed)
    }

    /// Parse the LLM response JSON into ParsedProfile
    fn parse_llm_response(response: &str) -> AppResult<ParsedProfile> {
        let json_str = Self::extract_json(response);

        let parsed: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
            warn!(response = %response, error = %e, "Failed to parse LLM response JSON");
            AppError::Internal(format!("Invalid JSON response from LLM: {}", e))
        })?;

        let purpose = parsed["purpose"].as_str().unwrap_or("").trim().to_string();

        let domain = parsed["domain"].as_str().unwrap_or("").trim().to_string();

        let target_audience = parsed["target_audience"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        let event_categories = Self::parse_string_array(&parsed["event_categories"]);
        let memory_focus = Self::parse_string_array(&parsed["memory_focus"]);
        let boundaries = Self::parse_string_array(&parsed["boundaries"]);

        let extraction_prompt = parsed["extraction_prompt"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        // Validate required fields
        if purpose.is_empty() {
            return Err(AppError::Internal(
                "LLM response missing required field: purpose".to_string(),
            ));
        }

        if domain.is_empty() {
            return Err(AppError::Internal(
                "LLM response missing required field: domain".to_string(),
            ));
        }

        Ok(ParsedProfile {
            purpose,
            domain,
            target_audience,
            event_categories,
            memory_focus,
            boundaries,
            extraction_prompt,
        })
    }

    /// Extract JSON from response (handles markdown code blocks)
    fn extract_json(response: &str) -> String {
        let response = response.trim();

        // Try to find JSON in markdown code block
        if let Some(start) = response.find("```json") {
            if let Some(end) = response[start..]
                .find("```\n")
                .or(response[start..].rfind("```"))
            {
                let json_start = start + 7; // Skip "```json"
                let json_end = start + end;
                if json_start < json_end {
                    return response[json_start..json_end].trim().to_string();
                }
            }
        }

        // Try to find JSON in generic code block
        if let Some(start) = response.find("```") {
            let after_start = start + 3;
            if let Some(end) = response[after_start..].find("```") {
                let content = &response[after_start..after_start + end];
                let json_content = content
                    .lines()
                    .skip_while(|line| !line.trim().starts_with('{'))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !json_content.is_empty() {
                    return json_content.trim().to_string();
                }
            }
        }

        // Try to find raw JSON object
        if let Some(start) = response.find('{') {
            if let Some(end) = response.rfind('}') {
                if start < end {
                    return response[start..=end].to_string();
                }
            }
        }

        response.to_string()
    }

    /// Parse a JSON array of strings
    fn parse_string_array(value: &serde_json::Value) -> Vec<String> {
        value
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_from_code_block() {
        let response = r#"```json
{
  "purpose": "房产推荐",
  "domain": "房地产",
  "target_audience": "购房者",
  "event_categories": ["咨询", "看房"],
  "memory_focus": ["偏好"],
  "boundaries": ["租房"],
  "extraction_prompt": "提取prompt"
}
```"#;
        let json = ProfileService::extract_json(response);
        assert!(json.contains("房产推荐"));
        assert!(json.contains("房地产"));
    }

    #[test]
    fn test_extract_json_raw() {
        let response = r#"{"purpose": "test", "domain": "test"}"#;
        let json = ProfileService::extract_json(response);
        assert_eq!(json, response);
    }

    #[test]
    fn test_parse_string_array() {
        let value = serde_json::json!(["item1", "item2", "item3"]);
        let result = ProfileService::parse_string_array(&value);
        assert_eq!(result, vec!["item1", "item2", "item3"]);
    }

    #[test]
    fn test_parse_string_array_with_whitespace() {
        let value = serde_json::json!(["  item1  ", "item2", "  "]);
        let result = ProfileService::parse_string_array(&value);
        assert_eq!(result, vec!["item1", "item2"]);
    }

    #[test]
    fn test_parse_string_array_empty() {
        let value = serde_json::json!(null);
        let result = ProfileService::parse_string_array(&value);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_llm_response_valid() {
        let response = r#"{
            "purpose": "房产推荐",
            "domain": "房地产",
            "target_audience": "购房者",
            "event_categories": ["咨询", "看房", "成交"],
            "memory_focus": ["购房偏好", "预算范围"],
            "boundaries": ["不处理租房"],
            "extraction_prompt": "请将用户行为解析为结构化格式..."
        }"#;

        let result = ProfileService::parse_llm_response(response).unwrap();

        assert_eq!(result.purpose, "房产推荐");
        assert_eq!(result.domain, "房地产");
        assert_eq!(result.target_audience, "购房者");
        assert_eq!(result.event_categories, vec!["咨询", "看房", "成交"]);
        assert_eq!(result.memory_focus, vec!["购房偏好", "预算范围"]);
        assert_eq!(result.boundaries, vec!["不处理租房"]);
        assert!(!result.extraction_prompt.is_empty());
    }

    #[test]
    fn test_parse_llm_response_missing_purpose() {
        let response = r#"{
            "domain": "房地产",
            "target_audience": "购房者"
        }"#;

        let result = ProfileService::parse_llm_response(response);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_llm_response_missing_domain() {
        let response = r#"{
            "purpose": "房产推荐",
            "target_audience": "购房者"
        }"#;

        let result = ProfileService::parse_llm_response(response);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_llm_response_with_code_block() {
        let response = r#"```json
{
    "purpose": "电商推荐",
    "domain": "电子商务",
    "target_audience": "消费者",
    "event_categories": ["浏览", "购买"],
    "memory_focus": ["购物偏好"],
    "boundaries": ["B2B"],
    "extraction_prompt": "提取prompt"
}
```"#;

        let result = ProfileService::parse_llm_response(response).unwrap();
        assert_eq!(result.purpose, "电商推荐");
        assert_eq!(result.domain, "电子商务");
    }

    #[test]
    fn test_default_prompt_contains_placeholder() {
        assert!(PARSE_PROFILE_PROMPT.contains("{description}"));
    }
}
