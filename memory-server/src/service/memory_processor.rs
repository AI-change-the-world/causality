//! MemoryProcessor service
//!
//! Responsible for LLM-driven memory processing: compression, classification,
//! and tag extraction in a single LLM call for efficiency.
//! This is an optional processing pipeline that can be enabled per-memory
//! via the `process_with_llm` flag.

use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::domain::MemoryCategory;
use crate::llm::{ChatRequest, LlmError, LlmProvider};

/// Error type for memory processing operations
#[derive(Debug, thiserror::Error)]
pub enum ProcessingError {
    /// LLM provider error
    #[error("LLM error: {0}")]
    LlmError(#[from] LlmError),

    /// Failed to parse LLM response
    #[error("Failed to parse LLM response: {0}")]
    ParseError(String),

    /// No LLM provider configured
    #[error("No LLM provider configured")]
    NoProvider,
}

/// Request for processing memory content
#[derive(Debug, Clone)]
pub struct ProcessMemoryRequest {
    /// Raw content to process
    pub content: String,
    /// Optional context information
    pub context: Option<String>,
}

impl ProcessMemoryRequest {
    /// Create a new process memory request
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            context: None,
        }
    }

    /// Create a new process memory request with context
    pub fn with_context(content: impl Into<String>, context: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            context: Some(context.into()),
        }
    }
}

/// Result of memory processing
#[derive(Debug, Clone)]
pub struct ProcessMemoryResult {
    /// Compressed/refined content
    pub processed_content: String,
    /// Automatically classified category
    pub category: MemoryCategory,
    /// Extracted tags/keywords
    pub tags: Vec<String>,
    /// Processing confidence score
    pub confidence: f32,
}

/// Default unified prompt template for memory processing (compression, classification, tag extraction)
const DEFAULT_UNIFIED_PROMPT: &str = r#"你是一个记忆处理助手。请对以下内容进行处理，完成三个任务：

1. **压缩**：提取关键信息，生成简洁的结构化记忆（保留核心事实和用户偏好，去除冗余）
2. **分类**：将记忆分类到以下类别之一：
   - user_preference: 用户偏好（如喜好、习惯设置）
   - behavior_pattern: 行为模式（如工作习惯、操作方式）
   - business_rule: 业务规则（如流程、规定）
   - factual_knowledge: 事实知识（如日期、数据）
   - other: 其他
3. **标签**：提取3-5个关键词/标签用于检索

原始内容：
{content}

请严格按照以下JSON格式返回结果，不要返回其他内容：
```json
{
  "compressed": "压缩后的记忆内容",
  "category": "分类名称",
  "tags": ["标签1", "标签2", "标签3"]
}
```"#;

/// MemoryProcessor service for LLM-driven memory processing
pub struct MemoryProcessor {
    /// LLM provider for processing
    llm_provider: Arc<dyn LlmProvider>,
    /// Unified processing prompt template
    unified_prompt: String,
}

impl MemoryProcessor {
    /// Create a new MemoryProcessor with the given LLM provider
    pub fn new(llm_provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            llm_provider,
            unified_prompt: DEFAULT_UNIFIED_PROMPT.to_string(),
        }
    }

    /// Create a new MemoryProcessor with a custom unified prompt
    pub fn with_prompt(llm_provider: Arc<dyn LlmProvider>, unified_prompt: Option<String>) -> Self {
        Self {
            llm_provider,
            unified_prompt: unified_prompt.unwrap_or_else(|| DEFAULT_UNIFIED_PROMPT.to_string()),
        }
    }

    /// Process raw content: compress, classify, and extract tags in a single LLM call
    pub async fn process(
        &self,
        request: ProcessMemoryRequest,
    ) -> Result<ProcessMemoryResult, ProcessingError> {
        debug!(
            content_len = request.content.len(),
            has_context = request.context.is_some(),
            "Processing memory content"
        );

        // Build the prompt with content
        let prompt = self.unified_prompt.replace("{content}", &request.content);

        // Single LLM call for all processing tasks with JSON response format
        let chat_request = ChatRequest::new(prompt)
            .with_temperature(0.3)
            .with_json_response();

        let response = self.llm_provider.chat(chat_request).await?;

        // Parse the JSON response
        let result = Self::parse_unified_response(&response.content)?;

        info!(
            original_len = request.content.len(),
            processed_len = result.processed_content.len(),
            category = %result.category,
            tag_count = result.tags.len(),
            "Memory processing completed"
        );

        Ok(result)
    }

    /// Parse the unified JSON response from LLM
    fn parse_unified_response(response: &str) -> Result<ProcessMemoryResult, ProcessingError> {
        // Try to extract JSON from the response (handle markdown code blocks)
        let json_str = Self::extract_json(response);

        // Parse JSON
        let parsed: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
            warn!(
                response = %response,
                error = %e,
                "Failed to parse JSON response"
            );
            ProcessingError::ParseError(format!("Invalid JSON: {}", e))
        })?;

        // Extract fields
        let compressed = parsed["compressed"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        let category_str = parsed["category"].as_str().unwrap_or("other");
        let category = Self::parse_category(category_str);

        let tags: Vec<String> = parsed["tags"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty() && s.len() <= 50)
                    .take(10)
                    .collect()
            })
            .unwrap_or_default();

        // Fallback if compressed content is empty
        let processed_content = if compressed.is_empty() {
            warn!("Compressed content is empty, this may indicate a parsing issue");
            response.to_string()
        } else {
            compressed
        };

        Ok(ProcessMemoryResult {
            processed_content,
            category,
            tags,
            confidence: 0.9,
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
                // Skip language identifier if present
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

        // Return as-is if no JSON found
        response.to_string()
    }

    /// Parse category from string
    fn parse_category(response: &str) -> MemoryCategory {
        let response_lower = response.to_lowercase();

        if response_lower.contains("user_preference") || response_lower.contains("userpreference") {
            MemoryCategory::UserPreference
        } else if response_lower.contains("behavior_pattern")
            || response_lower.contains("behaviorpattern")
        {
            MemoryCategory::BehaviorPattern
        } else if response_lower.contains("business_rule")
            || response_lower.contains("businessrule")
        {
            MemoryCategory::BusinessRule
        } else if response_lower.contains("factual_knowledge")
            || response_lower.contains("factualknowledge")
        {
            MemoryCategory::FactualKnowledge
        } else {
            MemoryCategory::Other
        }
    }

    /// Parse tags from comma-separated string (used in tests)
    #[cfg(test)]
    fn parse_tags(response: &str) -> Vec<String> {
        response
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.len() <= 50)
            .take(10)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_memory_request_new() {
        let request = ProcessMemoryRequest::new("Test content");
        assert_eq!(request.content, "Test content");
        assert!(request.context.is_none());
    }

    #[test]
    fn test_process_memory_request_with_context() {
        let request = ProcessMemoryRequest::with_context("Test content", "Some context");
        assert_eq!(request.content, "Test content");
        assert_eq!(request.context, Some("Some context".to_string()));
    }

    #[test]
    fn test_parse_category_user_preference() {
        assert_eq!(
            MemoryProcessor::parse_category("user_preference"),
            MemoryCategory::UserPreference
        );
        assert_eq!(
            MemoryProcessor::parse_category("UserPreference"),
            MemoryCategory::UserPreference
        );
        assert_eq!(
            MemoryProcessor::parse_category("This is user_preference category"),
            MemoryCategory::UserPreference
        );
    }

    #[test]
    fn test_parse_category_behavior_pattern() {
        assert_eq!(
            MemoryProcessor::parse_category("behavior_pattern"),
            MemoryCategory::BehaviorPattern
        );
        assert_eq!(
            MemoryProcessor::parse_category("BehaviorPattern"),
            MemoryCategory::BehaviorPattern
        );
    }

    #[test]
    fn test_parse_category_business_rule() {
        assert_eq!(
            MemoryProcessor::parse_category("business_rule"),
            MemoryCategory::BusinessRule
        );
        assert_eq!(
            MemoryProcessor::parse_category("BusinessRule"),
            MemoryCategory::BusinessRule
        );
    }

    #[test]
    fn test_parse_category_factual_knowledge() {
        assert_eq!(
            MemoryProcessor::parse_category("factual_knowledge"),
            MemoryCategory::FactualKnowledge
        );
        assert_eq!(
            MemoryProcessor::parse_category("FactualKnowledge"),
            MemoryCategory::FactualKnowledge
        );
    }

    #[test]
    fn test_parse_category_other() {
        assert_eq!(
            MemoryProcessor::parse_category("other"),
            MemoryCategory::Other
        );
        assert_eq!(
            MemoryProcessor::parse_category("unknown category"),
            MemoryCategory::Other
        );
        assert_eq!(MemoryProcessor::parse_category(""), MemoryCategory::Other);
    }

    #[test]
    fn test_parse_tags_simple() {
        let tags = MemoryProcessor::parse_tags("tag1, tag2, tag3");
        assert_eq!(tags, vec!["tag1", "tag2", "tag3"]);
    }

    #[test]
    fn test_parse_tags_with_whitespace() {
        let tags = MemoryProcessor::parse_tags("  tag1  ,  tag2  ,  tag3  ");
        assert_eq!(tags, vec!["tag1", "tag2", "tag3"]);
    }

    #[test]
    fn test_parse_tags_filters_empty() {
        let tags = MemoryProcessor::parse_tags("tag1, , tag2, , tag3");
        assert_eq!(tags, vec!["tag1", "tag2", "tag3"]);
    }

    #[test]
    fn test_parse_tags_limits_count() {
        let tags = MemoryProcessor::parse_tags("1,2,3,4,5,6,7,8,9,10,11,12");
        assert_eq!(tags.len(), 10);
    }

    #[test]
    fn test_parse_tags_filters_long() {
        let long_tag = "a".repeat(100);
        let tags = MemoryProcessor::parse_tags(&format!("short, {}", long_tag));
        assert_eq!(tags, vec!["short"]);
    }

    #[test]
    fn test_default_prompt_contains_placeholder() {
        assert!(DEFAULT_UNIFIED_PROMPT.contains("{content}"));
    }

    #[test]
    fn test_extract_json_from_code_block() {
        let response = r#"```json
{
  "compressed": "test content",
  "category": "user_preference",
  "tags": ["tag1", "tag2"]
}
```"#;
        let json = MemoryProcessor::extract_json(response);
        assert!(json.contains("compressed"));
        assert!(json.contains("user_preference"));
    }

    #[test]
    fn test_extract_json_raw() {
        let response = r#"{"compressed": "test", "category": "other", "tags": []}"#;
        let json = MemoryProcessor::extract_json(response);
        assert_eq!(json, response);
    }

    #[test]
    fn test_parse_unified_response() {
        let response = r#"{"compressed": "压缩后的内容", "category": "user_preference", "tags": ["标签1", "标签2"]}"#;
        let result = MemoryProcessor::parse_unified_response(response).unwrap();

        assert_eq!(result.processed_content, "压缩后的内容");
        assert_eq!(result.category, MemoryCategory::UserPreference);
        assert_eq!(result.tags, vec!["标签1", "标签2"]);
    }

    #[test]
    fn test_parse_unified_response_with_code_block() {
        let response = r#"```json
{
  "compressed": "测试内容",
  "category": "behavior_pattern",
  "tags": ["工作", "习惯"]
}
```"#;
        let result = MemoryProcessor::parse_unified_response(response).unwrap();

        assert_eq!(result.processed_content, "测试内容");
        assert_eq!(result.category, MemoryCategory::BehaviorPattern);
        assert_eq!(result.tags, vec!["工作", "习惯"]);
    }
}
