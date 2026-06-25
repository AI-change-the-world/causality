//! ProfileService implementation
//!
//! Provides business logic for SystemProfile management including:
//! - System initialization with LLM-based description parsing
//! - Profile retrieval
//! - Partial and full profile updates

use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::domain::{
    CreateProfileInput, ParsedProfile, ProfileValidation, SystemProfile, UpdateProfileInput,
};
use crate::error::{AppError, AppResult};
use crate::llm::{ChatRequest, LlmProvider};
use crate::repository::ProfileRepository;
use uuid::Uuid;

/// Default prompt template for parsing system description into structured profile
const PARSE_PROFILE_PROMPT: &str = r#"你是一个系统配置助手。请根据用户提供的系统描述，为一个独立业务系统生成 profile 配置和 metadata schema proposal。

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
  "extraction_prompt": "一段完整的 prompt，用于指导 LLM 将原始事件解析为该业务系统专属的 JSON 分析结果。输出应偏向 event analysis payload，而不是固定六要素。prompt 里要明确提取哪些事件字段、如何归纳意图、对象、状态变化、风险、结论等，并且要能识别业务内高价值的隐含变化信号。",
  "metadata_schema": {
    "version": 1,
    "entity_types": {
      "example_entity": {
        "description": "该类型 memory 的定义",
        "fields": {
          "memory_type": { "type": "string", "required": true, "filterable": true },
          "example_field": { "type": "string", "required": false, "filterable": true, "enum": ["example_a", "example_b"] }
        },
        "candidate_match_fields": ["memory_type"],
        "conflict_fields": ["memory_type"],
        "lineage_group_fields": ["memory_type"]
      }
    },
    "filterable_fields": ["memory_type", "example_field"],
    "retrieval_defaults": {
      "use_vector": true,
      "use_fulltext": true,
      "collapse_lineage": true
    }
  },
  "schema_generation_prompt": "一段完整 prompt，用于指导后续 LLM/调用方按照 metadata_schema 生成 memory metadata JSON。这个 prompt 必须清楚说明允许的字段、字段含义、必填字段、枚举范围、禁止杜撰未知字段。"
}
```

注意：
- event_categories 应该是该业务场景下常见的用户行为/事件类型
- memory_focus 应该是对该业务有价值的用户信息类型
- boundaries 应该明确系统的边界，避免功能发散
- extraction_prompt 应该是一个完整的、可直接使用的 prompt 模板，输出事件分析 JSON
- extraction_prompt 不要只是泛化摘要器，而要像该业务领域的分析引擎
- extraction_prompt 必须明确：什么信息即使是隐含表达，也应该被识别为“条件变化/预算变化/目标变化/决策状态变化”
- extraction_prompt 必须优先服务后续记忆抽取、记忆更新、候选缩圈和冲突判断，而不是只做对话总结
- extraction_prompt 必须告诉下游模型：如果一句话反映了用户资源能力变化、约束放宽/收紧、偏好强化/反转、决策推进/停滞，这属于高价值事件，不应轻易输出空对象
- 如果业务描述中存在“推荐、选购、筛选、决策、咨询、偏好、约束、预算、风险、阶段”这类语义，extraction_prompt 应显式覆盖这些维度
- 对带有状态变化的业务（如荐房、导购、教育规划、旅行决策），extraction_prompt 应能识别“以前/现在”“原本/后来”“预算紧/预算放宽”“不接受/现在可考虑”这类变化表达
- metadata_schema 必须是业务专属的 schema proposal，不要使用固定通用字段凑数
- metadata_schema 中的字段要以记忆检索、候选缩圈、冲突判断为目标
- metadata_schema 的字段设计要和 extraction_prompt 对齐：如果 extraction_prompt 会要求识别预算变化、约束变更、决策阶段、偏好对象，那么 schema 应提供相应的 filterable 字段或分组字段
- 对荐房、导购、筛选这类“同一类别下会出现多个具体选项”的业务，不要只给粗粒度 category 字段；应额外提供能够区分具体对象/目标的字段，如 preference_target / object / location_value / style_value，避免“湖边”和“海边”都坍缩成同一个 location 偏好
- candidate_match_fields / conflict_fields / lineage_group_fields 在这类业务中应优先包含“具体对象/目标字段”，否则系统无法正确判断是同一偏好被反转，还是新增了另一个并存偏好
- 如果某些字段存在稳定值域（如 memory_type、preference_category、sentiment、decision_stage、event_type），应尽量在 metadata_schema.fields.<field>.enum 中显式给出枚举值，方便调用方和后端做一致校验
- schema_generation_prompt 必须能直接指导下游生成符合 schema 的 metadata JSON
- schema_generation_prompt 必须引用这些 canonical 枚举值，明确要求下游不要使用自然语言近义词替代 schema 里的标准值
- schema_generation_prompt 还应明确：当用户表达的是某个具体偏好对象（如湖边、海边、学区、朝南、法式风格）时，必须同时保留“类别”和“具体对象值”，不要只保留粗粒度 category
- 如果用户描述的是垂直业务，请优先生成贴近该领域的 event_type / object / preference / constraint / stage / risk 结构，不要退回通用空泛字段
- 不要输出 markdown，不要附加解释，只返回 JSON"#;

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

        // Parse description using LLM
        let parsed = self.parse_description(&input.description).await?;

        // Create the profile
        let profile = SystemProfile::new(input, parsed);

        // Persist to database
        let created = self.repo.create(&profile).await?;

        info!(id = %created.id, name = %created.name, "System profile initialized");

        Ok(created)
    }

    /// Get the first system profile.
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

    /// Get a profile by ID.
    pub async fn get_by_id(&self, profile_id: Uuid) -> AppResult<SystemProfile> {
        self.repo.get_by_id(profile_id).await?.ok_or_else(|| {
            AppError::Validation(format!("System profile not found: {}", profile_id))
        })
    }

    /// List all system profiles.
    pub async fn list(&self) -> AppResult<Vec<SystemProfile>> {
        self.repo.list().await
    }

    /// Get a profile by name.
    pub async fn get_by_name(&self, name: &str) -> AppResult<Option<SystemProfile>> {
        self.repo.get_by_name(name).await
    }

    /// Update a specific system profile
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
    pub async fn update(
        &self,
        profile_id: Uuid,
        input: UpdateProfileInput,
    ) -> AppResult<SystemProfile> {
        debug!(reparse = input.reparse, "Updating system profile");

        // Validate input
        ProfileValidation::validate_update(&input)?;

        // Get existing profile
        let mut profile = self.get_by_id(profile_id).await?;

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

    /// Confirm the current schema proposal for a profile.
    pub async fn confirm_schema(&self, profile_id: Uuid) -> AppResult<SystemProfile> {
        debug!(%profile_id, "Confirming profile schema");

        let mut profile = self.get_by_id(profile_id).await?;
        profile.confirm_schema();

        let updated = self.repo.update(&profile).await?;
        info!(
            id = %updated.id,
            schema_version = updated.schema_version,
            confirmed_at = ?updated.schema_confirmed_at,
            "Profile schema confirmed"
        );

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
        let metadata_schema = parsed["metadata_schema"].clone();
        let schema_generation_prompt = parsed["schema_generation_prompt"]
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

        if extraction_prompt.is_empty() {
            return Err(AppError::Internal(
                "LLM response missing required field: extraction_prompt".to_string(),
            ));
        }

        Self::validate_metadata_schema(&metadata_schema)?;

        if schema_generation_prompt.is_empty() {
            return Err(AppError::Internal(
                "LLM response missing required field: schema_generation_prompt".to_string(),
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
            metadata_schema,
            schema_generation_prompt,
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

    fn validate_metadata_schema(schema: &Value) -> AppResult<()> {
        let schema_obj = schema.as_object().ok_or_else(|| {
            AppError::Internal("LLM response metadata_schema must be a JSON object".to_string())
        })?;

        if !schema_obj.contains_key("entity_types") {
            return Err(AppError::Internal(
                "LLM response metadata_schema missing required field: entity_types".to_string(),
            ));
        }

        if !schema_obj["entity_types"].is_object() {
            return Err(AppError::Internal(
                "LLM response metadata_schema.entity_types must be an object".to_string(),
            ));
        }

        if !schema_obj.contains_key("filterable_fields")
            || !schema_obj["filterable_fields"].is_array()
        {
            return Err(AppError::Internal(
                "LLM response metadata_schema.filterable_fields must be an array".to_string(),
            ));
        }

        if !schema_obj.contains_key("retrieval_defaults")
            || !schema_obj["retrieval_defaults"].is_object()
        {
            return Err(AppError::Internal(
                "LLM response metadata_schema.retrieval_defaults must be an object".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

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
  "extraction_prompt": "提取prompt",
  "metadata_schema": {
    "version": 1,
    "entity_types": {
      "house_preference": {
        "fields": {
          "memory_type": { "type": "string", "required": true, "filterable": true }
        },
        "candidate_match_fields": ["memory_type"],
        "conflict_fields": ["memory_type"],
        "lineage_group_fields": ["memory_type"]
      }
    },
    "filterable_fields": ["memory_type"],
    "retrieval_defaults": {
      "use_vector": true,
      "use_fulltext": true,
      "collapse_lineage": true
    }
  },
  "schema_generation_prompt": "生成 memory metadata"
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
            "extraction_prompt": "请将用户行为解析为结构化格式...",
            "metadata_schema": {
                "version": 1,
                "entity_types": {
                    "house_preference": {
                        "fields": {
                            "memory_type": { "type": "string", "required": true, "filterable": true },
                            "region": { "type": "string", "required": false, "filterable": true }
                        },
                        "candidate_match_fields": ["memory_type", "region"],
                        "conflict_fields": ["memory_type", "region"],
                        "lineage_group_fields": ["memory_type", "region"]
                    }
                },
                "filterable_fields": ["memory_type", "region"],
                "retrieval_defaults": {
                    "use_vector": true,
                    "use_fulltext": true,
                    "collapse_lineage": true
                }
            },
            "schema_generation_prompt": "请基于 house_preference schema 输出 metadata JSON"
        }"#;

        let result = ProfileService::parse_llm_response(response).unwrap();

        assert_eq!(result.purpose, "房产推荐");
        assert_eq!(result.domain, "房地产");
        assert_eq!(result.target_audience, "购房者");
        assert_eq!(result.event_categories, vec!["咨询", "看房", "成交"]);
        assert_eq!(result.memory_focus, vec!["购房偏好", "预算范围"]);
        assert_eq!(result.boundaries, vec!["不处理租房"]);
        assert!(!result.extraction_prompt.is_empty());
        assert!(result.metadata_schema.is_object());
        assert_eq!(
            result.metadata_schema["entity_types"]["house_preference"]["fields"]["region"]["type"],
            "string"
        );
        assert_eq!(
            result.schema_generation_prompt,
            "请基于 house_preference schema 输出 metadata JSON"
        );
    }

    #[test]
    fn test_parse_llm_response_missing_purpose() {
        let response = r#"{
            "domain": "房地产",
            "target_audience": "购房者",
            "metadata_schema": {},
            "schema_generation_prompt": "生成 metadata"
        }"#;

        let result = ProfileService::parse_llm_response(response);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_llm_response_missing_domain() {
        let response = r#"{
            "purpose": "房产推荐",
            "target_audience": "购房者",
            "metadata_schema": {},
            "schema_generation_prompt": "生成 metadata"
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
    "extraction_prompt": "提取prompt",
    "metadata_schema": {
        "version": 1,
        "entity_types": {
            "product_preference": {
                "fields": {
                    "memory_type": { "type": "string", "required": true, "filterable": true }
                },
                "candidate_match_fields": ["memory_type"],
                "conflict_fields": ["memory_type"],
                "lineage_group_fields": ["memory_type"]
            }
        },
        "filterable_fields": ["memory_type"],
        "retrieval_defaults": {
            "use_vector": true,
            "use_fulltext": true,
            "collapse_lineage": true
        }
    },
    "schema_generation_prompt": "提取商品偏好 metadata"
}
```"#;

        let result = ProfileService::parse_llm_response(response).unwrap();
        assert_eq!(result.purpose, "电商推荐");
        assert_eq!(result.domain, "电子商务");
    }

    #[test]
    fn test_parse_llm_response_missing_metadata_schema() {
        let response = r#"{
            "purpose": "房产推荐",
            "domain": "房地产",
            "target_audience": "购房者",
            "extraction_prompt": "提取prompt",
            "schema_generation_prompt": "生成 metadata"
        }"#;

        let result = ProfileService::parse_llm_response(response);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_llm_response_invalid_metadata_schema() {
        let response = r#"{
            "purpose": "房产推荐",
            "domain": "房地产",
            "target_audience": "购房者",
            "extraction_prompt": "提取prompt",
            "metadata_schema": {
                "version": 1,
                "entity_types": []
            },
            "schema_generation_prompt": "生成 metadata"
        }"#;

        let result = ProfileService::parse_llm_response(response);
        assert!(result.is_err());
    }

    #[test]
    fn test_confirm_schema_is_idempotent_on_entity() {
        let mut profile = SystemProfile {
            id: uuid::Uuid::new_v4(),
            name: "system".to_string(),
            description: "desc".to_string(),
            purpose: "purpose".to_string(),
            domain: "domain".to_string(),
            target_audience: "audience".to_string(),
            event_categories: vec![],
            memory_focus: vec![],
            boundaries: vec![],
            extraction_prompt: "prompt".to_string(),
            metadata_schema: serde_json::json!({
                "version": 1,
                "entity_types": {},
                "filterable_fields": [],
                "retrieval_defaults": {}
            }),
            schema_status: crate::domain::SchemaStatus::Confirmed,
            schema_version: 3,
            schema_confirmed_at: Some(Utc::now()),
            schema_generation_prompt: Some("schema prompt".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let confirmed_at = profile.schema_confirmed_at;
        profile.confirm_schema();

        assert_eq!(
            profile.schema_status,
            crate::domain::SchemaStatus::Confirmed
        );
        assert_eq!(profile.schema_version, 3);
        assert_eq!(profile.schema_confirmed_at, confirmed_at);
    }

    #[test]
    fn test_default_prompt_contains_placeholder() {
        assert!(PARSE_PROFILE_PROMPT.contains("{description}"));
    }
}
