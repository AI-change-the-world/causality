//! MemoryProcessor service
//!
//! Responsible for LLM-driven memory processing: compression, classification,
//! and tag extraction. This is an optional processing pipeline that can be
//! enabled per-memory via the `process_with_llm` flag.

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

/// Default prompt template for memory compression
const DEFAULT_COMPRESSION_PROMPT: &str = r#"你是一个记忆压缩助手。请从以下对话/操作记录中提取关键信息，生成简洁的结构化记忆。

要求：
1. 保留核心事实和用户偏好
2. 去除冗余和无关信息
3. 使用简洁的陈述句
4. 保持原意不变
5. 输出应该是一段简洁的文字，不要使用列表格式

原始内容：
{content}

压缩后的记忆："#;

/// Default prompt template for memory classification
const DEFAULT_CLASSIFICATION_PROMPT: &str = r#"请将以下记忆分类到最合适的类别：
- user_preference: 用户偏好（如喜好、习惯设置）
- behavior_pattern: 行为模式（如工作习惯、操作方式）
- business_rule: 业务规则（如流程、规定）
- factual_knowledge: 事实知识（如日期、数据）
- other: 其他

记忆内容：
{content}

请只返回类别名称（如 user_preference），不要返回其他内容："#;

/// Default prompt template for tag extraction
const DEFAULT_TAG_EXTRACTION_PROMPT: &str = r#"请从以下记忆内容中提取3-5个关键词/标签，用于后续检索。

要求：
1. 标签应该是名词或名词短语
2. 标签应该能够代表记忆的核心主题
3. 每个标签用逗号分隔
4. 只返回标签，不要返回其他内容

记忆内容：
{content}

标签："#;

/// MemoryProcessor service for LLM-driven memory processing
pub struct MemoryProcessor {
    /// LLM provider for processing
    llm_provider: Arc<dyn LlmProvider>,
    /// Compression prompt template
    compression_prompt: String,
    /// Classification prompt template
    classification_prompt: String,
    /// Tag extraction prompt template
    tag_extraction_prompt: String,
}

impl MemoryProcessor {
    /// Create a new MemoryProcessor with the given LLM provider
    pub fn new(llm_provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            llm_provider,
            compression_prompt: DEFAULT_COMPRESSION_PROMPT.to_string(),
            classification_prompt: DEFAULT_CLASSIFICATION_PROMPT.to_string(),
            tag_extraction_prompt: DEFAULT_TAG_EXTRACTION_PROMPT.to_string(),
        }
    }

    /// Create a new MemoryProcessor with custom prompts
    pub fn with_prompts(
        llm_provider: Arc<dyn LlmProvider>,
        compression_prompt: Option<String>,
        classification_prompt: Option<String>,
        tag_extraction_prompt: Option<String>,
    ) -> Self {
        Self {
            llm_provider,
            compression_prompt: compression_prompt
                .unwrap_or_else(|| DEFAULT_COMPRESSION_PROMPT.to_string()),
            classification_prompt: classification_prompt
                .unwrap_or_else(|| DEFAULT_CLASSIFICATION_PROMPT.to_string()),
            tag_extraction_prompt: tag_extraction_prompt
                .unwrap_or_else(|| DEFAULT_TAG_EXTRACTION_PROMPT.to_string()),
        }
    }

    /// Process raw content: compress, classify, and extract tags
    pub async fn process(
        &self,
        request: ProcessMemoryRequest,
    ) -> Result<ProcessMemoryResult, ProcessingError> {
        debug!(
            content_len = request.content.len(),
            has_context = request.context.is_some(),
            "Processing memory content"
        );

        // Step 1: Compress/refine the content
        let processed_content = self.compress(&request.content).await?;

        // Step 2: Classify the memory
        let category = self.classify(&processed_content).await?;

        // Step 3: Extract tags
        let tags = self.extract_tags(&processed_content).await?;

        info!(
            original_len = request.content.len(),
            processed_len = processed_content.len(),
            category = %category,
            tag_count = tags.len(),
            "Memory processing completed"
        );

        Ok(ProcessMemoryResult {
            processed_content,
            category,
            tags,
            confidence: 0.9, // Default confidence for successful processing
        })
    }

    /// Compress/refine raw content
    pub async fn compress(&self, content: &str) -> Result<String, ProcessingError> {
        let prompt = self.compression_prompt.replace("{content}", content);
        let request = ChatRequest::new(prompt)
            .with_temperature(0.3)
            .with_max_tokens(500);

        let response = self.llm_provider.chat(request).await?;
        let compressed = response.content.trim().to_string();

        debug!(
            original_len = content.len(),
            compressed_len = compressed.len(),
            "Content compressed"
        );

        Ok(compressed)
    }

    /// Classify memory into a category
    pub async fn classify(&self, content: &str) -> Result<MemoryCategory, ProcessingError> {
        let prompt = self.classification_prompt.replace("{content}", content);
        let request = ChatRequest::new(prompt)
            .with_temperature(0.1)
            .with_max_tokens(50);

        let response = self.llm_provider.chat(request).await?;
        let category_str = response.content.trim().to_lowercase();

        // Parse the category from the response
        let category = Self::parse_category(&category_str);

        debug!(
            raw_response = %category_str,
            category = %category,
            "Memory classified"
        );

        Ok(category)
    }

    /// Extract tags/keywords from content
    pub async fn extract_tags(&self, content: &str) -> Result<Vec<String>, ProcessingError> {
        let prompt = self.tag_extraction_prompt.replace("{content}", content);
        let request = ChatRequest::new(prompt)
            .with_temperature(0.3)
            .with_max_tokens(100);

        let response = self.llm_provider.chat(request).await?;
        let tags = Self::parse_tags(&response.content);

        debug!(
            tag_count = tags.len(),
            tags = ?tags,
            "Tags extracted"
        );

        Ok(tags)
    }

    /// Parse category from LLM response
    fn parse_category(response: &str) -> MemoryCategory {
        // Try to find a known category in the response
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
            // Default to Other if we can't parse
            warn!(
                response = %response,
                "Could not parse category, defaulting to Other"
            );
            MemoryCategory::Other
        }
    }

    /// Parse tags from LLM response
    fn parse_tags(response: &str) -> Vec<String> {
        response
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.len() <= 50) // Filter out empty and overly long tags
            .take(10) // Limit to 10 tags
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
    fn test_default_prompts_contain_placeholder() {
        assert!(DEFAULT_COMPRESSION_PROMPT.contains("{content}"));
        assert!(DEFAULT_CLASSIFICATION_PROMPT.contains("{content}"));
        assert!(DEFAULT_TAG_EXTRACTION_PROMPT.contains("{content}"));
    }
}
