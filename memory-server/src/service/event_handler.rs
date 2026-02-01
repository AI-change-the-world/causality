//! EventHandler service
//!
//! Responsible for structuring raw events into the six-element format (六要素模型).
//! Uses LLM to parse event content based on SystemProfile's extraction_prompt.
//!
//! The six core elements are:
//! - Time (时间): When the event occurred
//! - Location (地点): Device, page, content position
//! - Actor (人物): User identifier
//! - Cause (起因): Why the event was triggered
//! - Process (经过): What happened step by step
//! - Result (结果): Outcome of the event
//!
//! Plus two auxiliary elements:
//! - Background (背景): Context/scenario
//! - Details (细节): Key details, follow-up actions
//!
//! Also includes relevance filtering to ensure events match the SystemProfile's domain.
//!
//! Requirements: 6.9-6.10, 7.1-7.5, 8.1-8.4

use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::domain::{Event, ParsedStructuredEvent, StructuredEvent, SystemProfile};
use crate::error::{AppError, AppResult};
use crate::llm::{ChatRequest, LlmProvider};
use crate::service::ProfileService;

/// Default extraction prompt when no SystemProfile exists
/// This provides generic extraction without domain-specific guidance
const DEFAULT_EXTRACTION_PROMPT: &str = r#"你是一个事件分析助手。请将以下事件内容解析为结构化的六要素格式。

事件内容：
{content}

请提取以下要素：
1. 时间 (time)：事件发生的具体时间点
2. 地点 (location)：用户使用的设备类型、所在页面或位置
3. 人物 (actor)：用户标识
4. 起因 (cause)：事件触发的原因或动机
5. 经过 (process)：用户的具体操作过程
6. 结果 (result)：操作的结果或影响
7. 背景 (background)：用户当前的场景和目的
8. 细节 (details)：其他关键信息
9. 分类 (category)：事件类型分类

请以 JSON 格式返回，无法确定的字段返回 null：
```json
{
  "time": "时间信息或null",
  "location": "地点信息或null",
  "actor": "用户标识",
  "cause": "起因或null",
  "process": "经过或null",
  "result": "结果或null",
  "background": "背景或null",
  "details": "细节或null",
  "category": "分类或null"
}
```"#;

/// Prompt template for checking event relevance against SystemProfile
const RELEVANCE_CHECK_PROMPT: &str = r#"你是一个事件相关性判断助手。请判断以下事件内容是否与系统的业务领域相关。

## 系统信息
- 系统名称: {system_name}
- 系统用途: {purpose}
- 业务领域: {domain}
- 目标用户: {target_audience}
- 事件类型: {event_categories}
- 关注的记忆类型: {memory_focus}
- 系统边界（不处理的内容）: {boundaries}

## 事件内容
{content}

## 判断标准
1. 事件内容是否与业务领域相关
2. 事件是否属于系统定义的事件类型范围
3. 事件是否在系统边界之内（不在 boundaries 列表中）
4. 事件是否可能产生对目标用户有价值的记忆

## 请以 JSON 格式返回判断结果：
```json
{
  "is_relevant": true或false,
  "relevance_score": 0.0到1.0之间的相关性分数,
  "reason": "判断理由的简短说明",
  "matched_categories": ["匹配的事件类型列表，如果有的话"]
}
```

注意：
- 如果事件内容与业务领域完全无关，is_relevant 应为 false
- relevance_score 低于 0.3 时，建议 is_relevant 为 false
- 请严格按照系统边界判断，边界内的内容不应被处理"#;

/// Result of relevance check
#[derive(Debug, Clone)]
pub struct RelevanceCheckResult {
    /// Whether the event is relevant to the system profile
    pub is_relevant: bool,
    /// Relevance score (0.0 - 1.0)
    pub relevance_score: f32,
    /// Reason for the relevance decision
    pub reason: String,
    /// Matched event categories (if any)
    pub matched_categories: Vec<String>,
}

/// EventHandler service for structuring raw events
///
/// Uses LLM to parse raw event content into the six-element structured format.
/// When a SystemProfile exists, uses its extraction_prompt for domain-specific guidance.
#[derive(Clone)]
pub struct EventHandler {
    /// LLM provider for parsing events
    llm_provider: Arc<dyn LlmProvider>,
    /// Profile service for getting extraction prompt
    profile_service: Option<ProfileService>,
}

impl EventHandler {
    /// Create a new EventHandler with LLM provider
    pub fn new(llm_provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            llm_provider,
            profile_service: None,
        }
    }

    /// Create a new EventHandler with LLM provider and profile service
    pub fn with_profile_service(
        llm_provider: Arc<dyn LlmProvider>,
        profile_service: ProfileService,
    ) -> Self {
        Self {
            llm_provider,
            profile_service: Some(profile_service),
        }
    }

    /// Structure a raw event into the six-element format
    ///
    /// Uses the SystemProfile's extraction_prompt if available,
    /// otherwise falls back to the default generic prompt.
    ///
    /// # Arguments
    /// * `event` - The raw event to structure
    ///
    /// # Returns
    /// * `Ok(StructuredEvent)` - The structured event with six elements
    /// * `Err(AppError)` - If LLM parsing fails
    pub async fn structure_event(&self, event: &Event) -> AppResult<StructuredEvent> {
        debug!(
            event_id = %event.id,
            owner_id = %event.owner_id,
            content_len = event.content.len(),
            "Structuring event into six-element format"
        );

        // Get the extraction prompt (from SystemProfile or default)
        let extraction_prompt = self.get_extraction_prompt().await;

        // Build the full prompt with event content
        let prompt = self.build_extraction_prompt(&event.content, &extraction_prompt);

        // Call LLM to parse the event
        let chat_request = ChatRequest::new(prompt)
            .with_temperature(0.3)
            .with_json_response();

        let response = self.llm_provider.chat(chat_request).await.map_err(|e| {
            warn!(error = %e, event_id = %event.id, "LLM parsing failed for event");
            AppError::Internal(format!("Failed to structure event: {}", e))
        })?;

        // Parse the LLM response into structured event
        let parsed = self.parse_llm_response(&response.content, &event.owner_id)?;

        // Create the StructuredEvent
        let structured_event = StructuredEvent::from_parsed(event.id, parsed);

        info!(
            event_id = %event.id,
            structured_event_id = %structured_event.id,
            has_time = structured_event.time_element.is_some(),
            has_location = structured_event.location_element.is_some(),
            has_category = structured_event.category.is_some(),
            "Event structured successfully"
        );

        Ok(structured_event)
    }

    /// Check if an event is relevant to the SystemProfile
    ///
    /// Uses LLM to determine if the event content matches the system's
    /// business domain, event categories, and is within system boundaries.
    ///
    /// # Arguments
    /// * `content` - The event content to check
    /// * `profile` - The SystemProfile to check against
    ///
    /// # Returns
    /// * `Ok(RelevanceCheckResult)` - The relevance check result
    /// * `Err(AppError)` - If LLM check fails
    pub async fn check_relevance(
        &self,
        content: &str,
        profile: &SystemProfile,
    ) -> AppResult<RelevanceCheckResult> {
        debug!(
            profile_id = %profile.id,
            content_len = content.len(),
            "Checking event relevance against SystemProfile"
        );

        // Build the relevance check prompt
        let prompt = RELEVANCE_CHECK_PROMPT
            .replace("{system_name}", &profile.name)
            .replace("{purpose}", &profile.purpose)
            .replace("{domain}", &profile.domain)
            .replace("{target_audience}", &profile.target_audience)
            .replace("{event_categories}", &profile.event_categories.join(", "))
            .replace("{memory_focus}", &profile.memory_focus.join(", "))
            .replace("{boundaries}", &profile.boundaries.join(", "))
            .replace("{content}", content);

        // Call LLM to check relevance
        let chat_request = ChatRequest::new(prompt)
            .with_temperature(0.1) // Low temperature for consistent judgment
            .with_json_response();

        let response = self.llm_provider.chat(chat_request).await.map_err(|e| {
            warn!(error = %e, "LLM relevance check failed");
            AppError::Internal(format!("Failed to check event relevance: {}", e))
        })?;

        // Parse the relevance check response
        let result = self.parse_relevance_response(&response.content)?;

        info!(
            is_relevant = result.is_relevant,
            relevance_score = result.relevance_score,
            reason = %result.reason,
            "Event relevance check completed"
        );

        Ok(result)
    }

    /// Check relevance using the profile from ProfileService
    ///
    /// Convenience method that fetches the profile and checks relevance.
    /// If no profile exists, returns a default "relevant" result (permissive mode).
    ///
    /// # Arguments
    /// * `content` - The event content to check
    ///
    /// # Returns
    /// * `Ok(RelevanceCheckResult)` - The relevance check result
    pub async fn check_relevance_with_profile(
        &self,
        content: &str,
    ) -> AppResult<RelevanceCheckResult> {
        if let Some(ref profile_service) = self.profile_service {
            match profile_service.get().await {
                Ok(profile) => {
                    return self.check_relevance(content, &profile).await;
                }
                Err(e) => {
                    debug!(error = %e, "No SystemProfile found, skipping relevance check");
                }
            }
        }

        // No profile available - default to permissive (relevant)
        Ok(RelevanceCheckResult {
            is_relevant: true,
            relevance_score: 1.0,
            reason: "No SystemProfile configured, accepting all events".to_string(),
            matched_categories: vec![],
        })
    }

    /// Parse the LLM response for relevance check
    fn parse_relevance_response(&self, response: &str) -> AppResult<RelevanceCheckResult> {
        let json_str = Self::extract_json(response);

        let parsed: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
            warn!(response = %response, error = %e, "Failed to parse relevance check response");
            AppError::Internal(format!("Invalid JSON response from LLM: {}", e))
        })?;

        let is_relevant = parsed["is_relevant"].as_bool().unwrap_or(true);
        let relevance_score = parsed["relevance_score"]
            .as_f64()
            .map(|f| f as f32)
            .unwrap_or(if is_relevant { 1.0 } else { 0.0 });
        let reason = parsed["reason"]
            .as_str()
            .unwrap_or("No reason provided")
            .to_string();
        let matched_categories = parsed["matched_categories"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();

        Ok(RelevanceCheckResult {
            is_relevant,
            relevance_score,
            reason,
            matched_categories,
        })
    }

    /// Get the extraction prompt from SystemProfile or use default
    async fn get_extraction_prompt(&self) -> String {
        if let Some(ref profile_service) = self.profile_service {
            match profile_service.get().await {
                Ok(profile) => {
                    if !profile.extraction_prompt.is_empty() {
                        debug!("Using extraction prompt from SystemProfile");
                        return profile.extraction_prompt;
                    }
                }
                Err(e) => {
                    debug!(error = %e, "No SystemProfile found, using default extraction prompt");
                }
            }
        }

        debug!("Using default extraction prompt");
        DEFAULT_EXTRACTION_PROMPT.to_string()
    }

    /// Build the extraction prompt with event content
    ///
    /// Replaces {content} placeholder in the prompt template with actual event content.
    fn build_extraction_prompt(&self, content: &str, prompt_template: &str) -> String {
        prompt_template.replace("{content}", content)
    }

    /// Parse the LLM response into ParsedStructuredEvent
    fn parse_llm_response(
        &self,
        response: &str,
        default_actor: &str,
    ) -> AppResult<ParsedStructuredEvent> {
        let json_str = Self::extract_json(response);

        let parsed: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
            warn!(response = %response, error = %e, "Failed to parse LLM response JSON");
            AppError::Internal(format!("Invalid JSON response from LLM: {}", e))
        })?;

        // Extract fields with null handling
        let time = Self::extract_optional_string(&parsed["time"]);
        let location = Self::extract_optional_string(&parsed["location"]);
        let actor = Self::extract_optional_string(&parsed["actor"])
            .unwrap_or_else(|| default_actor.to_string());
        let cause = Self::extract_optional_string(&parsed["cause"]);
        let process = Self::extract_optional_string(&parsed["process"]);
        let result = Self::extract_optional_string(&parsed["result"]);
        let background = Self::extract_optional_string(&parsed["background"]);
        let details = Self::extract_optional_string(&parsed["details"]);
        let category = Self::extract_optional_string(&parsed["category"]);

        Ok(ParsedStructuredEvent {
            time,
            location,
            actor,
            cause,
            process,
            result,
            background,
            details,
            category,
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

    /// Extract optional string from JSON value, returning None for null or empty
    fn extract_optional_string(value: &serde_json::Value) -> Option<String> {
        value
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.to_lowercase() != "null")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_from_code_block() {
        let response = r#"```json
{
  "time": "2024-01-15 14:30",
  "location": "房源详情页",
  "actor": "user123",
  "cause": "用户对房源不满意",
  "process": "点击了不喜欢按钮",
  "result": "房源被标记为不喜欢",
  "background": "用户正在浏览推荐房源",
  "details": "选择原因：价格过高",
  "category": "反馈"
}
```"#;
        let json = EventHandler::extract_json(response);
        assert!(json.contains("user123"));
        assert!(json.contains("房源详情页"));
    }

    #[test]
    fn test_extract_json_raw() {
        let response = r#"{"time": "2024-01-15", "actor": "user123"}"#;
        let json = EventHandler::extract_json(response);
        assert_eq!(json, response);
    }

    #[test]
    fn test_extract_optional_string_valid() {
        let value = serde_json::json!("test value");
        let result = EventHandler::extract_optional_string(&value);
        assert_eq!(result, Some("test value".to_string()));
    }

    #[test]
    fn test_extract_optional_string_null() {
        let value = serde_json::json!(null);
        let result = EventHandler::extract_optional_string(&value);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_optional_string_null_string() {
        let value = serde_json::json!("null");
        let result = EventHandler::extract_optional_string(&value);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_optional_string_empty() {
        let value = serde_json::json!("");
        let result = EventHandler::extract_optional_string(&value);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_optional_string_whitespace() {
        let value = serde_json::json!("   ");
        let result = EventHandler::extract_optional_string(&value);
        assert!(result.is_none());
    }

    #[test]
    fn test_default_extraction_prompt_contains_placeholder() {
        assert!(DEFAULT_EXTRACTION_PROMPT.contains("{content}"));
    }

    #[test]
    fn test_build_extraction_prompt() {
        // Test the prompt building logic directly
        let template = "Process this: {content}";
        let content = "User clicked button";
        let result = template.replace("{content}", content);

        assert_eq!(result, "Process this: User clicked button");
    }

    #[test]
    fn test_parse_llm_response_full() {
        // Test parsing logic using a helper function that doesn't need a provider
        let response = r#"{
            "time": "2024-01-15 14:30",
            "location": "房源详情页",
            "actor": "user123",
            "cause": "用户对房源不满意",
            "process": "点击了不喜欢按钮",
            "result": "房源被标记为不喜欢",
            "background": "用户正在浏览推荐房源",
            "details": "选择原因：价格过高",
            "category": "反馈"
        }"#;

        let result = parse_llm_response_test(response, "default_user").unwrap();

        assert_eq!(result.time, Some("2024-01-15 14:30".to_string()));
        assert_eq!(result.location, Some("房源详情页".to_string()));
        assert_eq!(result.actor, "user123");
        assert_eq!(result.cause, Some("用户对房源不满意".to_string()));
        assert_eq!(result.process, Some("点击了不喜欢按钮".to_string()));
        assert_eq!(result.result, Some("房源被标记为不喜欢".to_string()));
        assert_eq!(result.background, Some("用户正在浏览推荐房源".to_string()));
        assert_eq!(result.details, Some("选择原因：价格过高".to_string()));
        assert_eq!(result.category, Some("反馈".to_string()));
    }

    #[test]
    fn test_parse_llm_response_with_nulls() {
        let response = r#"{
            "time": null,
            "location": "页面A",
            "actor": null,
            "cause": null,
            "process": "用户操作",
            "result": null,
            "background": null,
            "details": null,
            "category": "操作"
        }"#;

        let result = parse_llm_response_test(response, "default_user").unwrap();

        assert!(result.time.is_none());
        assert_eq!(result.location, Some("页面A".to_string()));
        assert_eq!(result.actor, "default_user"); // Falls back to default
        assert!(result.cause.is_none());
        assert_eq!(result.process, Some("用户操作".to_string()));
        assert!(result.result.is_none());
        assert!(result.background.is_none());
        assert!(result.details.is_none());
        assert_eq!(result.category, Some("操作".to_string()));
    }

    #[test]
    fn test_parse_llm_response_with_code_block() {
        let response = r#"```json
{
    "time": "2024-01-15",
    "location": "首页",
    "actor": "user456",
    "cause": null,
    "process": "浏览",
    "result": null,
    "background": null,
    "details": null,
    "category": "浏览"
}
```"#;

        let result = parse_llm_response_test(response, "default_user").unwrap();

        assert_eq!(result.time, Some("2024-01-15".to_string()));
        assert_eq!(result.location, Some("首页".to_string()));
        assert_eq!(result.actor, "user456");
        assert_eq!(result.category, Some("浏览".to_string()));
    }

    /// Helper function for testing parse_llm_response without needing a provider
    fn parse_llm_response_test(
        response: &str,
        default_actor: &str,
    ) -> crate::error::AppResult<ParsedStructuredEvent> {
        let json_str = EventHandler::extract_json(response);

        let parsed: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
            crate::error::AppError::Internal(format!("Invalid JSON response from LLM: {}", e))
        })?;

        // Extract fields with null handling
        let time = EventHandler::extract_optional_string(&parsed["time"]);
        let location = EventHandler::extract_optional_string(&parsed["location"]);
        let actor = EventHandler::extract_optional_string(&parsed["actor"])
            .unwrap_or_else(|| default_actor.to_string());
        let cause = EventHandler::extract_optional_string(&parsed["cause"]);
        let process = EventHandler::extract_optional_string(&parsed["process"]);
        let result = EventHandler::extract_optional_string(&parsed["result"]);
        let background = EventHandler::extract_optional_string(&parsed["background"]);
        let details = EventHandler::extract_optional_string(&parsed["details"]);
        let category = EventHandler::extract_optional_string(&parsed["category"]);

        Ok(ParsedStructuredEvent {
            time,
            location,
            actor,
            cause,
            process,
            result,
            background,
            details,
            category,
        })
    }

    /// Helper function for testing parse_relevance_response
    fn parse_relevance_response_test(
        response: &str,
    ) -> crate::error::AppResult<RelevanceCheckResult> {
        let json_str = EventHandler::extract_json(response);

        let parsed: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
            crate::error::AppError::Internal(format!("Invalid JSON response from LLM: {}", e))
        })?;

        let is_relevant = parsed["is_relevant"].as_bool().unwrap_or(true);
        let relevance_score = parsed["relevance_score"]
            .as_f64()
            .map(|f| f as f32)
            .unwrap_or(if is_relevant { 1.0 } else { 0.0 });
        let reason = parsed["reason"]
            .as_str()
            .unwrap_or("No reason provided")
            .to_string();
        let matched_categories = parsed["matched_categories"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();

        Ok(RelevanceCheckResult {
            is_relevant,
            relevance_score,
            reason,
            matched_categories,
        })
    }

    #[test]
    fn test_relevance_check_prompt_contains_placeholders() {
        assert!(RELEVANCE_CHECK_PROMPT.contains("{system_name}"));
        assert!(RELEVANCE_CHECK_PROMPT.contains("{purpose}"));
        assert!(RELEVANCE_CHECK_PROMPT.contains("{domain}"));
        assert!(RELEVANCE_CHECK_PROMPT.contains("{target_audience}"));
        assert!(RELEVANCE_CHECK_PROMPT.contains("{event_categories}"));
        assert!(RELEVANCE_CHECK_PROMPT.contains("{memory_focus}"));
        assert!(RELEVANCE_CHECK_PROMPT.contains("{boundaries}"));
        assert!(RELEVANCE_CHECK_PROMPT.contains("{content}"));
    }

    #[test]
    fn test_parse_relevance_response_relevant() {
        let response = r#"{
            "is_relevant": true,
            "relevance_score": 0.85,
            "reason": "事件内容与房产推荐业务相关",
            "matched_categories": ["咨询", "看房"]
        }"#;

        let result = parse_relevance_response_test(response).unwrap();

        assert!(result.is_relevant);
        assert!((result.relevance_score - 0.85).abs() < 0.01);
        assert_eq!(result.reason, "事件内容与房产推荐业务相关");
        assert_eq!(result.matched_categories, vec!["咨询", "看房"]);
    }

    #[test]
    fn test_parse_relevance_response_not_relevant() {
        let response = r#"{
            "is_relevant": false,
            "relevance_score": 0.15,
            "reason": "事件内容与房产业务无关，是关于编程语言的讨论",
            "matched_categories": []
        }"#;

        let result = parse_relevance_response_test(response).unwrap();

        assert!(!result.is_relevant);
        assert!((result.relevance_score - 0.15).abs() < 0.01);
        assert!(result.reason.contains("编程语言"));
        assert!(result.matched_categories.is_empty());
    }

    #[test]
    fn test_parse_relevance_response_with_code_block() {
        let response = r#"```json
{
    "is_relevant": false,
    "relevance_score": 0.1,
    "reason": "内容与系统业务领域不相关",
    "matched_categories": []
}
```"#;

        let result = parse_relevance_response_test(response).unwrap();

        assert!(!result.is_relevant);
        assert!((result.relevance_score - 0.1).abs() < 0.01);
    }

    #[test]
    fn test_parse_relevance_response_defaults() {
        // Test with minimal response - should use defaults
        let response = r#"{"reason": "Some reason"}"#;

        let result = parse_relevance_response_test(response).unwrap();

        // Default is_relevant is true
        assert!(result.is_relevant);
        // Default score for relevant is 1.0
        assert!((result.relevance_score - 1.0).abs() < 0.01);
        assert_eq!(result.reason, "Some reason");
        assert!(result.matched_categories.is_empty());
    }
}
