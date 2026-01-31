# Design Document: System Profile

## Overview

SystemProfile 是 Memory System 的核心配置实体，定义系统的业务边界、目标对象和行为规范。本设计包含：

1. **SystemProfile 数据模型** - 存储系统配置的实体
2. **数据库迁移** - 新增 system_profile 表
3. **Repository 层** - 数据持久化
4. **Service 层** - 业务逻辑（含 LLM 解析）
5. **API 层** - REST 接口
6. **Provider 简化** - 环境变量配置 LLM/Embedding

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         API Layer                                │
│  POST /api/v1/system/init  GET /api/v1/system  PUT /api/v1/system│
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                       Service Layer                              │
│                     ProfileService                               │
│  - initialize(description) → LLM parse → create                  │
│  - get() → return profile                                        │
│  - update(fields) → partial update / LLM re-parse                │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Repository Layer                             │
│                    ProfileRepository                             │
│  - create(profile) → INSERT                                      │
│  - get() → SELECT (singleton)                                    │
│  - update(profile) → UPDATE                                      │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Database Layer                              │
│                   system_profile table                           │
└─────────────────────────────────────────────────────────────────┘
```

## Components and Interfaces

### 1. Domain Entity: SystemProfile

```rust
/// 系统画像实体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemProfile {
    /// 唯一标识符
    pub id: Uuid,
    /// 系统名称 (max 100 chars)
    pub name: String,
    /// 原始用户描述
    pub description: String,
    /// 系统用途 (LLM 解析)
    pub purpose: String,
    /// 业务领域 (LLM 解析)
    pub domain: String,
    /// 目标对象 (LLM 解析)
    pub target_audience: String,
    /// 事件分类列表 (LLM 解析)
    pub event_categories: Vec<String>,
    /// 关注的记忆类型 (LLM 解析)
    pub memory_focus: Vec<String>,
    /// 系统边界/不处理的内容 (LLM 解析)
    pub boundaries: Vec<String>,
    /// 事件提取 prompt (LLM 生成，用于指导六要素提取)
    pub extraction_prompt: String,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 更新时间
    pub updated_at: DateTime<Utc>,
}
```

### 2. Domain Entity: StructuredEvent (六要素模型)

```rust
/// 结构化事件 - 六要素 + 两辅助
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredEvent {
    /// 原始事件 ID
    pub event_id: Uuid,
    /// 时间：事件发生的具体时间点
    pub time: Option<String>,
    /// 地点：设备、页面、内容位置
    pub location: Option<String>,
    /// 人物：操作用户标识
    pub actor: String,
    /// 起因：事件触发原因（基于 scope 推断）
    pub cause: Option<String>,
    /// 经过：事件过程描述
    pub process: Option<String>,
    /// 结果：事件结果/影响
    pub result: Option<String>,
    /// 背景（辅助）：用户当前场景
    pub background: Option<String>,
    /// 细节（辅助）：关键细节信息
    pub details: Option<String>,
    /// 事件分类（基于 SystemProfile.event_categories）
    pub category: Option<String>,
}
```

### 3. Service: EventHandler

```rust
/// 事件处理器 - 负责将原始输入结构化
pub struct EventHandler {
    llm_provider: Arc<dyn LlmProvider>,
    profile_service: Arc<ProfileService>,
}

impl EventHandler {
    /// 将原始事件内容结构化为六要素格式
    pub async fn structure_event(
        &self,
        event: &Event,
    ) -> AppResult<StructuredEvent>;
    
    /// 使用 SystemProfile 的 extraction_strategy 构建 prompt
    fn build_extraction_prompt(
        &self,
        content: &str,
        strategy: &ExtractionStrategy,
    ) -> String;
}
```

### 2. Repository: ProfileRepository

```rust
pub struct ProfileRepository {
    pool: PgPool,
}

impl ProfileRepository {
    /// 创建系统画像 (仅允许一条记录)
    pub async fn create(&self, profile: &SystemProfile) -> AppResult<SystemProfile>;
    
    /// 获取系统画像 (singleton)
    pub async fn get(&self) -> AppResult<Option<SystemProfile>>;
    
    /// 更新系统画像
    pub async fn update(&self, profile: &SystemProfile) -> AppResult<SystemProfile>;
    
    /// 检查是否已存在
    pub async fn exists(&self) -> AppResult<bool>;
}
```

### 3. Service: ProfileService

```rust
pub struct ProfileService {
    repo: ProfileRepository,
    llm_provider: Arc<dyn LlmProvider>,
}

impl ProfileService {
    /// 初始化系统 - 使用 LLM 解析描述
    pub async fn initialize(&self, request: InitializeRequest) -> AppResult<SystemProfile>;
    
    /// 获取当前系统画像
    pub async fn get(&self) -> AppResult<SystemProfile>;
    
    /// 更新系统画像
    pub async fn update(&self, request: UpdateProfileRequest) -> AppResult<SystemProfile>;
    
    /// 使用 LLM 解析描述
    async fn parse_description(&self, description: &str) -> AppResult<ParsedProfile>;
}
```

### 4. API Endpoints

| Method | Path                | Description    |
| ------ | ------------------- | -------------- |
| POST   | /api/v1/system/init | 初始化系统画像 |
| GET    | /api/v1/system      | 获取系统画像   |
| PUT    | /api/v1/system      | 更新系统画像   |

#### POST /api/v1/system/init

Request:
```json
{
  "name": "房产推荐系统",
  "description": "这是一个房产推荐系统，帮助购房者找到合适的房源，主要服务于有购房需求的客户"
}
```

Response (201):
```json
{
  "id": "uuid",
  "name": "房产推荐系统",
  "description": "这是一个房产推荐系统...",
  "purpose": "房产推荐",
  "domain": "房地产",
  "target_audience": "购房者",
  "event_categories": ["咨询", "看房", "比价", "投诉", "成交"],
  "memory_focus": ["购房偏好", "预算范围", "区域偏好", "户型需求"],
  "boundaries": ["不处理租房", "不处理商业地产"],
  "created_at": "2024-01-01T00:00:00Z",
  "updated_at": "2024-01-01T00:00:00Z"
}
```

#### PUT /api/v1/system

Request (partial update):
```json
{
  "name": "新名称",
  "event_categories": ["咨询", "看房", "成交"]
}
```

Request (re-parse with new description):
```json
{
  "description": "新的系统描述...",
  "reparse": true
}
```

### 5. Provider Configuration (config.yaml)

完全通过 `config.yaml` 配置 LLM 和 Embedding provider，不再使用数据库存储：

```yaml
# config.yaml
llm:
  provider_type: openai  # openai | azure | local
  endpoint: "https://api.openai.com/v1"
  api_key: "sk-xxx"  # 或通过环境变量 MEMORY_SERVER__LLM__API_KEY
  model: "gpt-4o-mini"

embedding:
  provider_type: openai  # openai | azure | local
  endpoint: "https://api.openai.com/v1"
  api_key: "sk-xxx"  # 或通过环境变量 MEMORY_SERVER__EMBEDDING__API_KEY
  model: "text-embedding-3-small"
  dimension: 1536
```

**变更：**
- 删除 `embedding_providers` 和 `llm_providers` 数据库表
- 删除 provider 管理 API（POST/PUT/DELETE /api/v1/config/providers）
- Event API 不再需要传 `llm_provider` 和 `embedding_provider` 参数
- 系统启动时从 config.yaml 加载 provider 配置

**敏感信息处理：**
- api_key 可以直接写在 config.yaml 中（开发环境）
- 也可以通过环境变量覆盖：`MEMORY_SERVER__LLM__API_KEY`

### 8. AppState 初始化

在应用启动时初始化全局 provider 实例：

```rust
/// 应用状态 - 包含全局 provider 实例
pub struct AppState {
    // ... 其他字段
    /// 全局 LLM provider 实例
    pub llm_provider: Arc<dyn LlmProvider>,
    /// 全局 Embedding provider 实例
    pub embedding_provider: Arc<dyn EmbeddingProvider>,
    /// Embedding provider 名称（用于 Qdrant collection）
    pub embedding_provider_name: String,
}

impl AppState {
    pub async fn new(config: AppConfig) -> Result<Self, AppError> {
        // 1. 初始化数据库连接
        let pool = create_db_pool(&config.database).await?;
        
        // 2. 初始化 Qdrant
        let qdrant_repo = QdrantRepository::new(&config.qdrant).await?;
        
        // 3. 初始化全局 LLM provider
        let llm_provider = create_llm_provider(&config.llm)?;
        
        // 4. 初始化全局 Embedding provider
        let (embedding_provider, embedding_provider_name) = 
            create_embedding_provider(&config.embedding)?;
        
        // 5. 确保 Qdrant collection 存在
        qdrant_repo.ensure_collection(
            &embedding_provider_name,
            config.embedding.dimension,
        ).await?;
        
        Ok(Self {
            // ...
            llm_provider,
            embedding_provider,
            embedding_provider_name,
        })
    }
}

/// 根据配置创建 LLM provider
fn create_llm_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, AppError> {
    match config.provider_type {
        ProviderType::OpenAI | ProviderType::Azure => {
            Ok(Arc::new(OpenAILlmProvider::new(config)?))
        }
        ProviderType::Local => {
            Ok(Arc::new(LocalLlmProvider::new(config)?))
        }
    }
}

/// 根据配置创建 Embedding provider
fn create_embedding_provider(config: &EmbeddingConfig) 
    -> Result<(Arc<dyn EmbeddingProvider>, String), AppError> 
{
    let name = format!("{}-{}", config.provider_type, config.model);
    let provider: Arc<dyn EmbeddingProvider> = match config.provider_type {
        ProviderType::OpenAI | ProviderType::Azure => {
            Arc::new(OpenAIProvider::new(config)?)
        }
        ProviderType::Local => {
            Arc::new(LocalProvider::new(config)?)
        }
    };
    Ok((provider, name))
}
```

**启动流程：**
1. 加载 config.yaml
2. 验证 LLM 和 Embedding 配置完整性
3. 创建全局 LLM provider 实例
4. 创建全局 Embedding provider 实例
5. 确保 Qdrant collection 存在（使用 embedding 配置的 dimension）
6. 将 provider 实例存入 AppState

### 6. LLM Prompt for Parsing SystemProfile

```
你是一个系统配置助手。请根据用户提供的系统描述，提取以下结构化信息：

用户描述：
{description}

请以 JSON 格式返回以下字段：
{
  "purpose": "系统的主要用途（简短描述）",
  "domain": "业务领域（如：房地产、电商、金融等）",
  "target_audience": "目标用户群体",
  "event_categories": ["可能的事件类型列表"],
  "memory_focus": ["需要关注的记忆类型"],
  "boundaries": ["系统不处理的内容"],
  "extraction_prompt": "一段完整的 prompt，用于指导 LLM 将原始事件解析为六要素格式（时间、地点、人物、起因、经过、结果、背景、细节）。这个 prompt 应该针对该业务领域定制，包含具体的提取指导和示例。"
}

注意：
- event_categories 应该是该业务场景下常见的用户行为/事件类型
- memory_focus 应该是对该业务有价值的用户信息类型
- boundaries 应该明确系统的边界，避免功能发散
- extraction_prompt 应该是一个完整的、可直接使用的 prompt 模板，包含该领域的具体提取指导
```

### 7. extraction_prompt 示例（房产推荐系统）

LLM 生成的 extraction_prompt 示例：

```
你是一个房产推荐系统的事件分析助手。请将用户行为解析为结构化的六要素格式。

原始事件内容：
{content}

请提取以下要素：
1. 时间：用户操作的具体时间点
2. 地点：用户使用的设备类型、所在页面（如：房源列表页、详情页、收藏页）
3. 人物：用户标识
4. 起因：用户行为的动机（如：对房源不满意、想了解更多、价格超预算）
5. 经过：用户的具体操作（如：点击不喜欢、收藏房源、提交咨询）
6. 结果：操作的结果（如：房源被标记、推荐策略调整、咨询已提交）
7. 背景：用户当前的浏览场景和目的
8. 细节：其他关键信息（如：是否选择了不喜欢原因、咨询的具体问题）

请以 JSON 格式返回，无法确定的字段返回 null。
```

## Data Models

### Database Schema

```sql
-- 添加到 001_init.sql

-- ============================================================================
-- SYSTEM PROFILE TABLE (Singleton)
-- ============================================================================

CREATE TABLE system_profile (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL,
    description TEXT NOT NULL,
    purpose TEXT NOT NULL,
    domain VARCHAR(100) NOT NULL,
    target_audience VARCHAR(255) NOT NULL,
    event_categories TEXT[] NOT NULL DEFAULT '{}',
    memory_focus TEXT[] NOT NULL DEFAULT '{}',
    boundaries TEXT[] NOT NULL DEFAULT '{}',
    -- 事件提取 prompt (完整的 prompt 模板)
    extraction_prompt TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Ensure singleton (only one row allowed)
CREATE UNIQUE INDEX idx_system_profile_singleton ON system_profile ((true));

-- Trigger for updated_at
CREATE TRIGGER update_system_profile_updated_at
    BEFORE UPDATE ON system_profile
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- ============================================================================
-- STRUCTURED EVENTS TABLE
-- ============================================================================

CREATE TABLE structured_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id UUID NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    -- 六要素
    time_element TEXT,
    location_element TEXT,
    actor_element VARCHAR(255) NOT NULL,
    cause_element TEXT,
    process_element TEXT,
    result_element TEXT,
    -- 两辅助
    background_element TEXT,
    details_element TEXT,
    -- 分类
    category VARCHAR(100),
    -- 时间戳
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- 一个事件只有一个结构化记录
    UNIQUE(event_id)
);

CREATE INDEX idx_structured_events_event ON structured_events(event_id);
CREATE INDEX idx_structured_events_category ON structured_events(category);
CREATE INDEX idx_structured_events_actor ON structured_events(actor_element);
```

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system-essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property 1: Round-trip Serialization

*For any* valid SystemProfile, serializing to JSON and deserializing back SHALL produce an equivalent object with all fields preserved.

**Validates: Requirements 1.1-1.10, 7.1, 7.2, 7.3**

### Property 2: Singleton Constraint

*For any* system instance, attempting to create a second SystemProfile when one already exists SHALL return an error, and the original profile SHALL remain unchanged.

**Validates: Requirements 1.11, 2.3**

### Property 3: Partial Update Preservation

*For any* SystemProfile and any partial update request, fields not included in the update request SHALL remain unchanged, and the updated_at timestamp SHALL be updated.

**Validates: Requirements 4.1, 4.3**

### Property 4: Name Length Validation

*For any* string with length greater than 100 characters, attempting to create or update a SystemProfile with that string as the name SHALL be rejected with a validation error.

**Validates: Requirements 1.2**

## Error Handling

| Error Case                 | HTTP Status     | Error Code          | Message                                        |
| -------------------------- | --------------- | ------------------- | ---------------------------------------------- |
| System already initialized | 409 Conflict    | ALREADY_INITIALIZED | "System profile already exists"                |
| System not initialized     | 404 Not Found   | NOT_INITIALIZED     | "System profile not found"                     |
| LLM parsing failed         | 500 Internal    | LLM_PARSE_ERROR     | "Failed to parse description"                  |
| Name too long              | 400 Bad Request | VALIDATION_ERROR    | "Name must be 100 characters or less"          |
| Missing env config         | 500 Internal    | CONFIG_ERROR        | "Required environment variable not set: {var}" |

## Testing Strategy

### Unit Tests

- ProfileRepository CRUD operations (with test database)
- SystemProfile validation logic
- LLM prompt generation
- JSON serialization/deserialization

### Property-Based Tests

使用 `proptest` 库进行属性测试：

1. **Round-trip serialization** - 生成随机 SystemProfile，验证序列化往返
2. **Singleton constraint** - 验证重复创建被拒绝
3. **Partial update** - 生成随机更新请求，验证未更新字段保持不变
4. **Name validation** - 生成随机长度字符串，验证 >100 被拒绝

### Integration Tests

- 完整初始化流程（含 LLM mock）
- API 端点测试
- 环境变量配置加载测试
