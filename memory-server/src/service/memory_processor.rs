//! MemoryProcessor service
//!
//! Responsible for LLM-driven memory processing: compression, classification,
//! tag extraction, event-to-memory extraction, and query enhancement.
//! This is an optional processing pipeline that can be enabled per-memory
//! via the `process_with_llm` flag.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::domain::{ExtractedMemory, InferenceType, MemoryCategory};
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

    /// Event content too short for processing
    #[error("Event content too short: minimum {min} characters required")]
    EventContentTooShort { min: usize },

    /// No valid memories could be extracted from event
    #[error("No valid memories could be extracted from event")]
    NoMemoriesExtracted,

    /// Query enhancement failed
    #[error("Query enhancement failed: {0}")]
    QueryEnhancementFailed(String),
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

/// Request for extracting memories from an event
///
/// This struct represents a request to extract structured memories from raw event content.
/// The LLM will automatically understand the event type and extract relevant memories.
#[derive(Debug, Clone)]
pub struct ExtractFromEventRequest {
    /// Event content (can be any form: click description, conversation history, operation log, etc.)
    pub content: String,
    /// Optional context to help LLM better understand the event
    pub context: Option<String>,
}

impl ExtractFromEventRequest {
    /// Create a new extract from event request
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            context: None,
        }
    }

    /// Create a new extract from event request with context
    pub fn with_context(content: impl Into<String>, context: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            context: Some(context.into()),
        }
    }
}

/// Result of extracting memories from an event
///
/// Contains the event summary, extracted memories, and overall confidence score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractFromEventResult {
    /// Event summary (LLM-generated understanding of the event)
    pub event_summary: String,
    /// List of extracted memories
    pub extracted_memories: Vec<ExtractedMemory>,
    /// Overall processing confidence score (0.0 - 1.0)
    pub confidence: f32,
}

/// Enhanced query result containing both original and enhanced queries
///
/// Used for query enhancement functionality where the LLM expands the query
/// with semantic synonyms and related terms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedQuery {
    /// The original query provided by the user
    pub original_query: String,
    /// The enhanced query with semantic expansions
    pub enhanced_query: String,
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

/// Prompt template for summarizing long conversations
const SUMMARIZE_PROMPT: &str = r#"你是一个对话摘要助手。请对以下长对话/事件内容进行摘要，保留关键信息：

1. **保留关键事实**：用户明确提到的事实、数据、日期等
2. **保留用户偏好**：用户表达的喜好、习惯、倾向
3. **保留重要决策**：用户做出的决定或选择
4. **去除冗余**：删除重复内容、寒暄、无关信息

原始内容：
{content}

请直接返回摘要内容，不要添加任何格式标记或解释。摘要应该简洁但完整，保留所有重要信息。"#;

/// Prompt template for extracting memories from events
const EXTRACT_FROM_EVENT_PROMPT: &str = r#"你是一个记忆提取助手。请从以下事件内容中提取有价值的记忆。

事件内容：
{content}
{context_section}

请分析事件内容，提取以下类型的记忆：
1. **事实(fact)**：直接从事件中提取的明确事实，置信度 >= 0.9
2. **偏好(preference)**：推断出的用户偏好，置信度在 0.6-0.8 之间
3. **模式(pattern)**：识别出的行为模式，置信度在 0.6-0.8 之间
4. **规则(rule)**：提取的业务规则，置信度在 0.7-0.9 之间

对于每条记忆，请：
- 生成层级式分类路径（如 "work.code.eslint", "personal.food.chinese", "preference.ui.theme"）
- 提取3-5个关键词标签
- 评估重要性(0.0-1.0)
- 提供推断理由

请严格按照以下JSON格式返回结果：
```json
{
  "event_summary": "对事件的简要理解",
  "extracted_memories": [
    {
      "content": "提取的记忆内容",
      "inference_type": "fact|preference|pattern|rule",
      "confidence": 0.9,
      "category": "层级式分类路径（如 work.code.eslint）",
      "tags": ["标签1", "标签2"],
      "importance": 0.7,
      "reasoning": "为什么提取这条记忆"
    }
  ],
  "confidence": 0.85
}
```

注意：
- 如果事件中包含多个独立的见解，请提取多条记忆
- 如果无法提取有价值的记忆，返回空的 extracted_memories 数组
- 事实类型的置信度必须 >= 0.9
- 偏好和模式类型的置信度应在 0.6-0.8 之间
- category 必须是层级式路径，用点号分隔（如 work.code.eslint）"#;

/// Prompt template for query enhancement
const ENHANCE_QUERY_PROMPT: &str = r#"你是一个查询增强助手。请对以下查询进行语义扩展，添加同义词和相关术语以提高检索效果。

原始查询：
{query}
{context_section}

请扩展查询，包括：
1. 同义词和近义词
2. 相关概念和术语
3. 可能的变体表达

请严格按照以下JSON格式返回结果：
```json
{
  "original_query": "原始查询",
  "enhanced_query": "扩展后的查询（包含原始查询和扩展词）"
}
```

注意：
- 扩展后的查询应该是一个自然的搜索字符串
- 保留原始查询的核心意图
- 不要过度扩展，保持相关性"#;

/// Minimum content length for event processing
const MIN_EVENT_CONTENT_LENGTH: usize = 10;

/// Content length threshold for summarization (approximately 2000 tokens)
const SUMMARIZE_THRESHOLD: usize = 4000;

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

    /// Check if content needs summarization before processing
    ///
    /// Returns true if content exceeds 4000 characters (approximately 2000 tokens).
    pub fn needs_summary(&self, content: &str) -> bool {
        content.len() > SUMMARIZE_THRESHOLD
    }

    /// Summarize long conversation/event content
    ///
    /// Preserves key facts and user preferences while removing redundancy.
    /// This method should be called before extract_from_event for long content.
    pub async fn summarize_conversation(&self, content: &str) -> Result<String, ProcessingError> {
        debug!(content_len = content.len(), "Summarizing long conversation");

        let prompt = SUMMARIZE_PROMPT.replace("{content}", content);

        let chat_request = ChatRequest::new(prompt).with_temperature(0.3);

        let response = self.llm_provider.chat(chat_request).await?;

        let summary = response.content.trim().to_string();

        info!(
            original_len = content.len(),
            summary_len = summary.len(),
            "Conversation summarized"
        );

        Ok(summary)
    }

    /// Build the extraction prompt with optional context
    fn build_extraction_prompt(&self, content: &str, context: &Option<String>) -> String {
        let context_section = match context {
            Some(ctx) => format!("\n上下文信息：\n{}", ctx),
            None => String::new(),
        };

        EXTRACT_FROM_EVENT_PROMPT
            .replace("{content}", content)
            .replace("{context_section}", &context_section)
    }

    /// Extract memories from an event
    ///
    /// The LLM will automatically:
    /// 1. Understand the event type and content
    /// 2. Extract facts and infer preferences
    /// 3. Auto-classify and tag each memory
    /// 4. Evaluate importance and confidence
    pub async fn extract_from_event(
        &self,
        request: ExtractFromEventRequest,
    ) -> Result<ExtractFromEventResult, ProcessingError> {
        debug!(
            content_len = request.content.len(),
            has_context = request.context.is_some(),
            "Extracting memories from event"
        );

        // Validate minimum content length
        if request.content.trim().len() < MIN_EVENT_CONTENT_LENGTH {
            return Err(ProcessingError::EventContentTooShort {
                min: MIN_EVENT_CONTENT_LENGTH,
            });
        }

        // Summarize if content is too long
        let content = if self.needs_summary(&request.content) {
            self.summarize_conversation(&request.content).await?
        } else {
            request.content.clone()
        };

        // Build the extraction prompt
        let prompt = self.build_extraction_prompt(&content, &request.context);

        // Call LLM for extraction
        let chat_request = ChatRequest::new(prompt)
            .with_temperature(0.3)
            .with_json_response();

        let response = self.llm_provider.chat(chat_request).await?;

        // Parse the extraction response
        let result = Self::parse_extraction_response(&response.content)?;

        info!(
            memory_count = result.extracted_memories.len(),
            confidence = result.confidence,
            "Memories extracted from event"
        );

        Ok(result)
    }

    /// Parse the extraction response from LLM
    fn parse_extraction_response(
        response: &str,
    ) -> Result<ExtractFromEventResult, ProcessingError> {
        let json_str = Self::extract_json(response);

        let parsed: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
            warn!(
                response = %response,
                error = %e,
                "Failed to parse extraction response"
            );
            ProcessingError::ParseError(format!("Invalid JSON: {}", e))
        })?;

        let event_summary = parsed["event_summary"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        let confidence = parsed["confidence"]
            .as_f64()
            .map(|v| v as f32)
            .unwrap_or(0.5);

        let extracted_memories = Self::parse_extracted_memories(&parsed["extracted_memories"])?;

        Ok(ExtractFromEventResult {
            event_summary,
            extracted_memories,
            confidence,
        })
    }

    /// Parse extracted memories from JSON array
    fn parse_extracted_memories(
        value: &serde_json::Value,
    ) -> Result<Vec<ExtractedMemory>, ProcessingError> {
        let arr = match value.as_array() {
            Some(arr) => arr,
            None => return Ok(Vec::new()),
        };

        let mut memories = Vec::new();

        for item in arr {
            let content = item["content"].as_str().unwrap_or("").trim().to_string();

            if content.is_empty() {
                continue;
            }

            let inference_type =
                Self::parse_inference_type(item["inference_type"].as_str().unwrap_or("fact"));

            let confidence = item["confidence"]
                .as_f64()
                .map(|v| (v as f32).clamp(0.0, 1.0))
                .unwrap_or(0.5);

            // Use hierarchical category string directly (e.g., "work.code.eslint")
            let category = item["category"]
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            let tags: Vec<String> = item["tags"]
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

            let importance = item["importance"]
                .as_f64()
                .map(|v| (v as f32).clamp(0.0, 1.0))
                .unwrap_or(0.5);

            let reasoning = item["reasoning"].as_str().unwrap_or("").trim().to_string();

            memories.push(ExtractedMemory {
                content,
                inference_type,
                confidence,
                category,
                tags: Some(tags),
                importance,
                reasoning,
            });
        }

        Ok(memories)
    }

    /// Parse inference type from string
    fn parse_inference_type(s: &str) -> InferenceType {
        match s.to_lowercase().as_str() {
            "fact" => InferenceType::Fact,
            "preference" => InferenceType::Preference,
            "pattern" => InferenceType::Pattern,
            "rule" => InferenceType::Rule,
            _ => InferenceType::Fact,
        }
    }

    /// Enhance a query with semantic synonyms
    ///
    /// Expands the query with related terms to improve retrieval results.
    pub async fn enhance_query(
        &self,
        query: &str,
        context: Option<&str>,
    ) -> Result<EnhancedQuery, ProcessingError> {
        debug!(
            query_len = query.len(),
            has_context = context.is_some(),
            "Enhancing query"
        );

        let context_section = match context {
            Some(ctx) => format!("\n上下文信息：\n{}", ctx),
            None => String::new(),
        };

        let prompt = ENHANCE_QUERY_PROMPT
            .replace("{query}", query)
            .replace("{context_section}", &context_section);

        let chat_request = ChatRequest::new(prompt)
            .with_temperature(0.3)
            .with_json_response();

        let response = self
            .llm_provider
            .chat(chat_request)
            .await
            .map_err(|e| ProcessingError::QueryEnhancementFailed(e.to_string()))?;

        let result = Self::parse_enhanced_query_response(&response.content, query)?;

        info!(
            original_len = query.len(),
            enhanced_len = result.enhanced_query.len(),
            "Query enhanced"
        );

        Ok(result)
    }

    /// Parse enhanced query response from LLM
    fn parse_enhanced_query_response(
        response: &str,
        original_query: &str,
    ) -> Result<EnhancedQuery, ProcessingError> {
        let json_str = Self::extract_json(response);

        let parsed: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
            warn!(
                response = %response,
                error = %e,
                "Failed to parse enhanced query response"
            );
            ProcessingError::QueryEnhancementFailed(format!("Invalid JSON: {}", e))
        })?;

        let enhanced_query = parsed["enhanced_query"]
            .as_str()
            .unwrap_or(original_query)
            .trim()
            .to_string();

        Ok(EnhancedQuery {
            original_query: original_query.to_string(),
            enhanced_query,
        })
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

    // Tests for ExtractFromEventRequest
    #[test]
    fn test_extract_from_event_request_new() {
        let request = ExtractFromEventRequest::new("Test event content");
        assert_eq!(request.content, "Test event content");
        assert!(request.context.is_none());
    }

    #[test]
    fn test_extract_from_event_request_with_context() {
        let request = ExtractFromEventRequest::with_context("Test event", "Some context");
        assert_eq!(request.content, "Test event");
        assert_eq!(request.context, Some("Some context".to_string()));
    }

    // Tests for needs_summary
    #[test]
    fn test_needs_summary_short_content() {
        use crate::llm::{LlmProviderConfig, LlmProviderType, LocalLlmProvider};
        use std::sync::Arc;

        let config = LlmProviderConfig {
            name: "test".to_string(),
            provider_type: LlmProviderType::Local,
            endpoint: "http://localhost:11434".to_string(),
            api_key: None,
            model: "test".to_string(),
            enabled: true,
            max_input_tokens: 4000,
            max_output_tokens: 1000,
            temperature: 0.3,
        };
        let provider = Arc::new(LocalLlmProvider::new(config).unwrap());
        let processor = MemoryProcessor::new(provider);

        // Short content should not need summary
        let short_content = "a".repeat(100);
        assert!(!processor.needs_summary(&short_content));

        // Content at threshold should not need summary
        let threshold_content = "a".repeat(4000);
        assert!(!processor.needs_summary(&threshold_content));

        // Content above threshold should need summary
        let long_content = "a".repeat(4001);
        assert!(processor.needs_summary(&long_content));
    }

    // Tests for parse_inference_type
    #[test]
    fn test_parse_inference_type() {
        assert_eq!(
            MemoryProcessor::parse_inference_type("fact"),
            InferenceType::Fact
        );
        assert_eq!(
            MemoryProcessor::parse_inference_type("FACT"),
            InferenceType::Fact
        );
        assert_eq!(
            MemoryProcessor::parse_inference_type("preference"),
            InferenceType::Preference
        );
        assert_eq!(
            MemoryProcessor::parse_inference_type("pattern"),
            InferenceType::Pattern
        );
        assert_eq!(
            MemoryProcessor::parse_inference_type("rule"),
            InferenceType::Rule
        );
        assert_eq!(
            MemoryProcessor::parse_inference_type("unknown"),
            InferenceType::Fact
        );
    }

    // Tests for parse_extraction_response
    #[test]
    fn test_parse_extraction_response() {
        let response = r#"{
            "event_summary": "用户表达了对深色主题的偏好",
            "extracted_memories": [
                {
                    "content": "用户喜欢深色主题",
                    "inference_type": "preference",
                    "confidence": 0.8,
                    "category": "user_preference",
                    "tags": ["主题", "偏好"],
                    "importance": 0.7,
                    "reasoning": "用户明确表示喜欢深色主题"
                }
            ],
            "confidence": 0.85
        }"#;

        let result = MemoryProcessor::parse_extraction_response(response).unwrap();

        assert_eq!(result.event_summary, "用户表达了对深色主题的偏好");
        assert_eq!(result.confidence, 0.85);
        assert_eq!(result.extracted_memories.len(), 1);

        let memory = &result.extracted_memories[0];
        assert_eq!(memory.content, "用户喜欢深色主题");
        assert_eq!(memory.inference_type, InferenceType::Preference);
        assert!((memory.confidence - 0.8).abs() < 0.01);
        assert_eq!(memory.category, Some("user_preference".to_string()));
        assert_eq!(
            memory.tags,
            Some(vec!["主题".to_string(), "偏好".to_string()])
        );
        assert!((memory.importance - 0.7).abs() < 0.01);
        assert_eq!(memory.reasoning, "用户明确表示喜欢深色主题");
    }

    #[test]
    fn test_parse_extraction_response_with_code_block() {
        let response = r#"```json
{
    "event_summary": "测试事件",
    "extracted_memories": [],
    "confidence": 0.5
}
```"#;

        let result = MemoryProcessor::parse_extraction_response(response).unwrap();

        assert_eq!(result.event_summary, "测试事件");
        assert!(result.extracted_memories.is_empty());
        assert!((result.confidence - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_parse_extraction_response_multiple_memories() {
        let response = r#"{
            "event_summary": "用户讨论了工作习惯",
            "extracted_memories": [
                {
                    "content": "用户每天早上9点开始工作",
                    "inference_type": "fact",
                    "confidence": 0.95,
                    "category": "behavior_pattern",
                    "tags": ["工作", "时间"],
                    "importance": 0.6,
                    "reasoning": "用户明确提到工作时间"
                },
                {
                    "content": "用户喜欢在安静环境中工作",
                    "inference_type": "preference",
                    "confidence": 0.7,
                    "category": "user_preference",
                    "tags": ["工作", "环境"],
                    "importance": 0.5,
                    "reasoning": "从对话中推断"
                }
            ],
            "confidence": 0.8
        }"#;

        let result = MemoryProcessor::parse_extraction_response(response).unwrap();

        assert_eq!(result.extracted_memories.len(), 2);
        assert_eq!(
            result.extracted_memories[0].inference_type,
            InferenceType::Fact
        );
        assert_eq!(
            result.extracted_memories[1].inference_type,
            InferenceType::Preference
        );
    }

    // Tests for parse_enhanced_query_response
    #[test]
    fn test_parse_enhanced_query_response() {
        let response = r#"{
            "original_query": "深色主题",
            "enhanced_query": "深色主题 暗色模式 dark theme 夜间模式"
        }"#;

        let result = MemoryProcessor::parse_enhanced_query_response(response, "深色主题").unwrap();

        assert_eq!(result.original_query, "深色主题");
        assert_eq!(
            result.enhanced_query,
            "深色主题 暗色模式 dark theme 夜间模式"
        );
    }

    #[test]
    fn test_parse_enhanced_query_response_fallback() {
        let response = r#"{"invalid": "response"}"#;

        let result = MemoryProcessor::parse_enhanced_query_response(response, "original").unwrap();

        assert_eq!(result.original_query, "original");
        assert_eq!(result.enhanced_query, "original");
    }

    // Tests for prompt templates
    #[test]
    fn test_summarize_prompt_contains_placeholder() {
        assert!(SUMMARIZE_PROMPT.contains("{content}"));
    }

    #[test]
    fn test_extract_from_event_prompt_contains_placeholders() {
        assert!(EXTRACT_FROM_EVENT_PROMPT.contains("{content}"));
        assert!(EXTRACT_FROM_EVENT_PROMPT.contains("{context_section}"));
    }

    #[test]
    fn test_enhance_query_prompt_contains_placeholders() {
        assert!(ENHANCE_QUERY_PROMPT.contains("{query}"));
        assert!(ENHANCE_QUERY_PROMPT.contains("{context_section}"));
    }

    // Tests for confidence clamping
    #[test]
    fn test_parse_extracted_memories_clamps_confidence() {
        let json = serde_json::json!([
            {
                "content": "test",
                "inference_type": "fact",
                "confidence": 1.5,
                "category": "other",
                "tags": [],
                "importance": -0.5,
                "reasoning": "test"
            }
        ]);

        let memories = MemoryProcessor::parse_extracted_memories(&json).unwrap();

        assert_eq!(memories.len(), 1);
        assert!((memories[0].confidence - 1.0).abs() < 0.01);
        assert!((memories[0].importance - 0.0).abs() < 0.01);
    }

    // Tests for empty content filtering
    #[test]
    fn test_parse_extracted_memories_filters_empty_content() {
        let json = serde_json::json!([
            {
                "content": "",
                "inference_type": "fact",
                "confidence": 0.9,
                "category": "other",
                "tags": [],
                "importance": 0.5,
                "reasoning": "test"
            },
            {
                "content": "valid content",
                "inference_type": "fact",
                "confidence": 0.9,
                "category": "other",
                "tags": [],
                "importance": 0.5,
                "reasoning": "test"
            }
        ]);

        let memories = MemoryProcessor::parse_extracted_memories(&json).unwrap();

        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].content, "valid content");
    }
}
