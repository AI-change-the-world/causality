//! EventHandler service
//!
//! Responsible for analyzing raw events into profile-aware JSON payloads and
//! performing relevance checks before memory extraction.

use std::sync::Arc;

use serde_json::Value;
use tracing::{debug, info, warn};

use crate::domain::{Event, Memory, SystemProfile};
use crate::error::{AppError, AppResult};
use crate::llm::{ChatRequest, LlmProvider};
use crate::service::ProfileService;

/// Default extraction prompt when no SystemProfile exists.
const DEFAULT_EXTRACTION_PROMPT: &str = r#"你是一个事件分析助手。请将以下事件内容解析为结构化事件分析 JSON。

事件内容：
{content}

请尽量提取以下信息：
1. 事件类型 (event_type)
2. 行为主体 (actor)
3. 意图/动机 (intent)
4. 涉及对象 (entities)
5. 关键状态变化 (state_changes)
6. 结果/结论 (result)
7. 风险或限制 (risk_flags)
8. 上下文补充 (context_notes)
9. 分类 (category)

请以 JSON 格式返回，无法确定的字段返回 null：
```json
{
  "event_type": "事件类型或null",
  "actor": "主体标识或null",
  "intent": "意图或null",
  "entities": ["对象1", "对象2"],
  "state_changes": ["变化1", "变化2"],
  "result": "结果或null",
  "risk_flags": ["风险1", "风险2"],
  "context_notes": "补充上下文或null",
  "category": "分类或null"
}
```"#;

/// Prompt template for checking event relevance against SystemProfile.
const RELEVANCE_CHECK_PROMPT: &str = r#"你是一个事件相关性判断助手。请判断以下事件内容是否与系统的业务领域相关，或者是否与用户的历史记忆有关联。

## 系统信息
- 系统名称: {system_name}
- 系统用途: {purpose}
- 业务领域: {domain}
- 目标用户: {target_audience}
- 事件类型: {event_categories}
- 关注的记忆类型: {memory_focus}
- 系统边界（不处理的内容）: {boundaries}

## 用户历史记忆（上下文）
{context_memories}

## 新事件内容
{content}

## 判断标准
1. 事件内容是否与业务领域直接相关
2. 事件是否属于系统定义的事件类型范围
3. 事件是否在系统边界之内（不在 boundaries 列表中）
4. 事件是否与用户的历史记忆有关联
5. 事件是否可能产生对目标用户有价值的记忆

## 请以 JSON 格式返回判断结果：
```json
{
  "is_relevant": true或false,
  "relevance_score": 0.0到1.0之间的相关性分数,
  "reason": "判断理由的简短说明",
  "matched_categories": ["匹配的事件类型列表，如果有的话"],
  "related_memory_indices": [与哪些历史记忆相关的索引号，从1开始]
}
```"#;

/// Prompt template for checking event relevance without context memories.
const RELEVANCE_CHECK_PROMPT_NO_CONTEXT: &str = r#"你是一个事件相关性判断助手。请判断以下事件内容是否与系统的业务领域相关。

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
```"#;

/// Result of relevance check.
#[derive(Debug, Clone)]
pub struct RelevanceCheckResult {
    pub is_relevant: bool,
    pub relevance_score: f32,
    pub reason: String,
    pub matched_categories: Vec<String>,
    pub related_memory_indices: Vec<usize>,
}

/// Profile-aware event analysis result.
#[derive(Debug, Clone)]
pub struct EventAnalysis {
    pub payload: Value,
    pub schema_version: Option<i32>,
}

/// EventHandler service for analyzing raw events.
#[derive(Clone)]
pub struct EventHandler {
    llm_provider: Arc<dyn LlmProvider>,
    profile_service: Option<ProfileService>,
}

impl EventHandler {
    pub fn new(llm_provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            llm_provider,
            profile_service: None,
        }
    }

    pub fn with_profile_service(
        llm_provider: Arc<dyn LlmProvider>,
        profile_service: ProfileService,
    ) -> Self {
        Self {
            llm_provider,
            profile_service: Some(profile_service),
        }
    }

    /// Analyze a raw event into a profile-aware JSON payload.
    pub async fn structure_event(&self, event: &Event) -> AppResult<EventAnalysis> {
        debug!(
            event_id = %event.id,
            owner_id = %event.owner_id,
            content_len = event.content.len(),
            "Analyzing event into structured payload"
        );

        let (extraction_prompt, schema_version) =
            self.get_extraction_config(event.profile_id).await;
        let prompt = self.build_extraction_prompt(&event.content, &extraction_prompt);

        let chat_request = ChatRequest::new(prompt)
            .with_temperature(0.3)
            .with_json_response();

        let response = self.llm_provider.chat(chat_request).await.map_err(|e| {
            warn!(error = %e, event_id = %event.id, "LLM parsing failed for event");
            AppError::Internal(format!("Failed to structure event: {}", e))
        })?;

        let payload = self.parse_llm_response(&response.content)?;

        info!(
            event_id = %event.id,
            schema_version = ?schema_version,
            top_level_keys = payload.as_object().map(|obj| obj.len()).unwrap_or(0),
            "Event analyzed successfully"
        );

        Ok(EventAnalysis {
            payload,
            schema_version,
        })
    }

    pub async fn check_relevance(
        &self,
        content: &str,
        profile: &SystemProfile,
        context_memories: Option<&[Memory]>,
    ) -> AppResult<RelevanceCheckResult> {
        debug!(
            profile_id = %profile.id,
            content_len = content.len(),
            context_memory_count = context_memories.map(|m| m.len()).unwrap_or(0),
            "Checking event relevance against SystemProfile"
        );

        let prompt = if let Some(memories) = context_memories {
            if memories.is_empty() {
                RELEVANCE_CHECK_PROMPT_NO_CONTEXT
                    .replace("{system_name}", &profile.name)
                    .replace("{purpose}", &profile.purpose)
                    .replace("{domain}", &profile.domain)
                    .replace("{target_audience}", &profile.target_audience)
                    .replace("{event_categories}", &profile.event_categories.join(", "))
                    .replace("{memory_focus}", &profile.memory_focus.join(", "))
                    .replace("{boundaries}", &profile.boundaries.join(", "))
                    .replace("{content}", content)
            } else {
                RELEVANCE_CHECK_PROMPT
                    .replace("{system_name}", &profile.name)
                    .replace("{purpose}", &profile.purpose)
                    .replace("{domain}", &profile.domain)
                    .replace("{target_audience}", &profile.target_audience)
                    .replace("{event_categories}", &profile.event_categories.join(", "))
                    .replace("{memory_focus}", &profile.memory_focus.join(", "))
                    .replace("{boundaries}", &profile.boundaries.join(", "))
                    .replace(
                        "{context_memories}",
                        &Self::format_context_memories(memories),
                    )
                    .replace("{content}", content)
            }
        } else {
            RELEVANCE_CHECK_PROMPT_NO_CONTEXT
                .replace("{system_name}", &profile.name)
                .replace("{purpose}", &profile.purpose)
                .replace("{domain}", &profile.domain)
                .replace("{target_audience}", &profile.target_audience)
                .replace("{event_categories}", &profile.event_categories.join(", "))
                .replace("{memory_focus}", &profile.memory_focus.join(", "))
                .replace("{boundaries}", &profile.boundaries.join(", "))
                .replace("{content}", content)
        };

        let chat_request = ChatRequest::new(prompt)
            .with_temperature(0.1)
            .with_json_response();

        let response = self.llm_provider.chat(chat_request).await.map_err(|e| {
            warn!(error = %e, "LLM relevance check failed");
            AppError::Internal(format!("Failed to check event relevance: {}", e))
        })?;

        let result = self.parse_relevance_response(&response.content)?;

        info!(
            is_relevant = result.is_relevant,
            relevance_score = result.relevance_score,
            reason = %result.reason,
            related_memory_count = result.related_memory_indices.len(),
            "Event relevance check completed"
        );

        Ok(result)
    }

    pub async fn check_relevance_with_profile(
        &self,
        profile_id: uuid::Uuid,
        content: &str,
        context_memories: Option<&[Memory]>,
    ) -> AppResult<RelevanceCheckResult> {
        if let Some(ref profile_service) = self.profile_service {
            match profile_service.get_by_id(profile_id).await {
                Ok(profile) => {
                    return self
                        .check_relevance(content, &profile, context_memories)
                        .await;
                }
                Err(e) => {
                    debug!(error = %e, "No SystemProfile found, skipping relevance check");
                }
            }
        }

        Ok(RelevanceCheckResult {
            is_relevant: true,
            relevance_score: 1.0,
            reason: "No SystemProfile configured, accepting all events".to_string(),
            matched_categories: vec![],
            related_memory_indices: vec![],
        })
    }

    fn format_context_memories(memories: &[Memory]) -> String {
        if memories.is_empty() {
            return "（无历史记忆）".to_string();
        }

        memories
            .iter()
            .enumerate()
            .map(|(idx, memory)| {
                let metadata_preview =
                    if memory.metadata.is_null() || memory.metadata == serde_json::json!({}) {
                        "metadata={}".to_string()
                    } else {
                        format!("metadata={}", memory.metadata)
                    };
                let global_marker = if memory.is_global {
                    " [长期记忆]"
                } else {
                    ""
                };
                format!(
                    "{}. [{}{}] {}",
                    idx + 1,
                    metadata_preview,
                    global_marker,
                    memory.content
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn get_extraction_config(&self, profile_id: uuid::Uuid) -> (String, Option<i32>) {
        if let Some(ref profile_service) = self.profile_service {
            match profile_service.get_by_id(profile_id).await {
                Ok(profile) => {
                    if !profile.extraction_prompt.is_empty() {
                        debug!("Using extraction prompt from SystemProfile");
                        return (profile.extraction_prompt, Some(profile.schema_version));
                    }
                }
                Err(e) => {
                    debug!(error = %e, "No SystemProfile found, using default extraction prompt");
                }
            }
        }

        debug!("Using default extraction prompt");
        (DEFAULT_EXTRACTION_PROMPT.to_string(), None)
    }

    fn build_extraction_prompt(&self, content: &str, prompt_template: &str) -> String {
        prompt_template.replace("{content}", content)
    }

    fn parse_llm_response(&self, response: &str) -> AppResult<Value> {
        let json_str = Self::extract_json(response);

        let parsed: Value = serde_json::from_str(&json_str).map_err(|e| {
            warn!(response = %response, error = %e, "Failed to parse LLM response JSON");
            AppError::Internal(format!("Invalid JSON response from LLM: {}", e))
        })?;

        if !parsed.is_object() {
            return Err(AppError::Internal(
                "Invalid JSON response from LLM: expected top-level object".to_string(),
            ));
        }

        Ok(parsed)
    }

    fn parse_relevance_response(&self, response: &str) -> AppResult<RelevanceCheckResult> {
        let json_str = Self::extract_json(response);

        let parsed: Value = serde_json::from_str(&json_str).map_err(|e| {
            warn!(response = %response, error = %e, "Failed to parse relevance check response");
            AppError::Internal(format!("Invalid JSON response from LLM: {}", e))
        })?;

        Ok(RelevanceCheckResult {
            is_relevant: parsed["is_relevant"].as_bool().unwrap_or(true),
            relevance_score: parsed["relevance_score"]
                .as_f64()
                .map(|f| f as f32)
                .unwrap_or(if parsed["is_relevant"].as_bool().unwrap_or(true) {
                    1.0
                } else {
                    0.0
                }),
            reason: parsed["reason"]
                .as_str()
                .unwrap_or("No reason provided")
                .to_string(),
            matched_categories: parsed["matched_categories"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default(),
            related_memory_indices: parsed["related_memory_indices"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_u64())
                        .map(|n| n as usize)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    fn extract_json(response: &str) -> String {
        let response = response.trim();

        if let Some(start) = response.find("```json") {
            if let Some(end) = response[start..]
                .find("```\n")
                .or(response[start..].rfind("```"))
            {
                let json_start = start + 7;
                let json_end = start + end;
                if json_start < json_end {
                    return response[json_start..json_end].trim().to_string();
                }
            }
        }

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

        if let Some(start) = response.find('{') {
            if let Some(end) = response.rfind('}') {
                if start < end {
                    return response[start..=end].to_string();
                }
            }
        }

        response.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_from_code_block() {
        let response = r#"```json
{
  "event_type": "feedback",
  "actor": "user123",
  "intent": "reject_listing",
  "entities": ["listing_001"],
  "state_changes": ["listing_marked_disliked"],
  "result": "房源被标记为不喜欢",
  "risk_flags": [],
  "context_notes": "用户正在浏览推荐房源",
  "category": "反馈"
}
```"#;
        let json = EventHandler::extract_json(response);
        assert!(json.contains("user123"));
        assert!(json.contains("reject_listing"));
    }

    #[test]
    fn test_extract_json_raw() {
        let response = r#"{"event_type":"browse","actor":"user123"}"#;
        let json = EventHandler::extract_json(response);
        assert_eq!(json, response);
    }

    #[test]
    fn test_default_extraction_prompt_contains_placeholder() {
        assert!(DEFAULT_EXTRACTION_PROMPT.contains("{content}"));
    }

    #[test]
    fn test_parse_llm_response_full() {
        let response = r#"{
            "event_type": "feedback",
            "actor": "user123",
            "intent": "reject_listing",
            "entities": ["listing_001"],
            "state_changes": ["listing_marked_disliked"],
            "result": "房源被标记为不喜欢",
            "context_notes": "用户正在浏览推荐房源",
            "category": "反馈"
        }"#;

        let result = parse_llm_response_test(response).unwrap();

        assert_eq!(result["event_type"], "feedback");
        assert_eq!(result["actor"], "user123");
        assert_eq!(result["intent"], "reject_listing");
        assert_eq!(result["category"], "反馈");
    }

    #[test]
    fn test_parse_llm_response_with_code_block() {
        let response = r#"```json
{
    "event_type": "browse",
    "actor": "user456",
    "intent": "view_page",
    "entities": ["首页"],
    "state_changes": [],
    "result": null,
    "risk_flags": [],
    "context_notes": null,
    "category": "浏览"
}
```"#;

        let result = parse_llm_response_test(response).unwrap();
        assert_eq!(result["event_type"], "browse");
        assert_eq!(result["actor"], "user456");
        assert_eq!(result["category"], "浏览");
    }

    fn parse_llm_response_test(response: &str) -> crate::error::AppResult<Value> {
        let json_str = EventHandler::extract_json(response);
        let parsed: Value = serde_json::from_str(&json_str).map_err(|e| {
            crate::error::AppError::Internal(format!("Invalid JSON response from LLM: {}", e))
        })?;

        if !parsed.is_object() {
            return Err(crate::error::AppError::Internal(
                "expected object".to_string(),
            ));
        }

        Ok(parsed)
    }

    fn parse_relevance_response_test(
        response: &str,
    ) -> crate::error::AppResult<RelevanceCheckResult> {
        let json_str = EventHandler::extract_json(response);
        let parsed: Value = serde_json::from_str(&json_str).map_err(|e| {
            crate::error::AppError::Internal(format!("Invalid JSON response from LLM: {}", e))
        })?;

        Ok(RelevanceCheckResult {
            is_relevant: parsed["is_relevant"].as_bool().unwrap_or(true),
            relevance_score: parsed["relevance_score"]
                .as_f64()
                .map(|f| f as f32)
                .unwrap_or(if parsed["is_relevant"].as_bool().unwrap_or(true) {
                    1.0
                } else {
                    0.0
                }),
            reason: parsed["reason"]
                .as_str()
                .unwrap_or("No reason provided")
                .to_string(),
            matched_categories: parsed["matched_categories"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default(),
            related_memory_indices: parsed["related_memory_indices"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_u64())
                        .map(|n| n as usize)
                        .collect()
                })
                .unwrap_or_default(),
        })
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
    }

    #[test]
    fn test_format_context_memories_empty() {
        let memories: Vec<Memory> = vec![];
        let result = EventHandler::format_context_memories(&memories);
        assert_eq!(result, "（无历史记忆）");
    }

    #[test]
    fn test_format_context_memories_single() {
        let memory = create_test_memory(
            "我想买一套海景房，但是价钱太贵了买不起",
            serde_json::json!({
                "memory_type": "house_budget",
                "status": "insufficient_budget"
            }),
            false,
        );
        let result = EventHandler::format_context_memories(&[memory]);
        assert!(result.contains("\"memory_type\":\"house_budget\""));
        assert!(result.contains("买不起"));
    }

    fn create_test_memory(content: &str, metadata: Value, is_global: bool) -> Memory {
        Memory {
            id: uuid::Uuid::new_v4(),
            profile_id: uuid::Uuid::new_v4(),
            owner_id: "owner123".to_string(),
            scope_id: if is_global {
                None
            } else {
                Some("scope123".to_string())
            },
            content: content.to_string(),
            metadata,
            schema_version: 1,
            importance: 0.8,
            confidence: 0.8,
            root_memory_id: None,
            version_number: 1,
            is_current_version: true,
            supersedes: None,
            superseded_by: None,
            is_global,
            hit_count: 0,
            last_hit_at: None,
            reinforcement_count: 0,
            last_reinforced_at: None,
            decay_score: 1.0,
            source_event_id: None,
            status: crate::domain::Status::Active,
            embedding_status: crate::domain::EmbeddingStatus::Pending,
            embedding_provider: None,
            processing_status: crate::domain::ProcessingStatus::Pending,
            llm_provider: None,
            inference_type: None,
            inference_confidence: None,
            inference_reasoning: None,
            conflict_reason: None,
            consistency_confidence: None,
            promoted_at: None,
            promotion_reason: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }
}
