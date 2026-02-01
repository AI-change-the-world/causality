# Design Document: Memory Server

## Overview

Memory Server 是一个独立的上下文记忆治理服务，为 Agent/LLM 应用提供结构化记忆的存储、检索、演化与治理能力。

核心设计理念：
- **治理层定位**：默认不调用 LLM 推理，但提供可选的 LLM 处理管道
- **显式可审计**：所有操作都有明确的触发和记录
- **三引擎检索**：PostgreSQL 结构过滤 + 全文检索 + Qdrant 向量召回
- **三层记忆**：Session（分钟级）→ Task（天级）→ Long-term（长期，需确认）
- **可选智能处理**：通过接口开关控制是否启用 LLM 记忆压缩/提纯/分类

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Agent / LLM Application                   │
│                  (LangChain / 自研 / 其他)                    │
└──────────────────────────┬──────────────────────────────────┘
                           │ HTTP REST API
┌──────────────────────────▼──────────────────────────────────┐
│                     Memory Server (Rust)                     │
│  ┌─────────────────────────────────────────────────────┐    │
│  │                    API Layer                         │    │
│  │  - Memory CRUD    - Retrieval    - Config API       │    │
│  │  - Health Check   - Metrics      - Audit Query      │    │
│  └─────────────────────────┬───────────────────────────┘    │
│                            │                                 │
│  ┌─────────────┬───────────┴───────────┬─────────────┐      │
│  │Memory Guard │  Retrieval Engine     │Config Center│      │
│  │- 写入验证    │  - 结构过滤           │- Provider管理│      │
│  │- 层级约束    │  - 全文检索           │- 热更新      │      │
│  │- 更新模式    │  - 向量召回           │- Rate Limit │      │
│  │             │  - 综合打分           │             │      │
│  └──────┬──────┴───────────┬───────────┴─────────────┘      │
│         │                  │                                 │
│  ┌──────▼──────────────────┴───────────────────────────┐    │
│  │              Memory Processor (可选)                 │    │
│  │  - LLM 压缩提纯  - 自动分类  - 关键词提取  - 标签生成 │    │
│  └─────────────────────────┬───────────────────────────┘    │
│                            │                                 │
│  ┌─────────────────────────┴───────────────────────────┐    │
│  │              Lifecycle Manager                       │    │
│  │  - TTL 管理   - 状态转换   - 冷却/摒弃   - 审计日志  │    │
│  └─────────────────────────────────────────────────────┘    │
└───────────┬─────────────────────────────────┬───────────────┘
            │                                 │
┌───────────▼───────────┐       ┌─────────────▼─────────────┐
│     PostgreSQL        │       │         Qdrant            │
│  - Memory 元数据       │       │  - Embedding 向量          │
│  - 全文索引 (tsvector) │       │  - 相似度检索              │
│  - 配置持久化          │       │  - Payload Filter         │
│  - 审计日志            │       │                           │
│  - Provider 配置       │       │                           │
└───────────────────────┘       └───────────────────────────┘
            │
┌───────────▼───────────┐       ┌───────────────────────────┐
│   Embedding Provider  │       │      LLM Provider         │
│  - Openai API         │       │  - Openai GPT             │
│  - Azure Openai       │       │  - Azure Openai           │
│  - Local (BGE等)      │       │  - Local (Ollama等)       │
└───────────────────────┘       └───────────────────────────┘
```

### 组件职责

| 组件              | 职责                                               |
| ----------------- | -------------------------------------------------- |
| API Layer (Axum)  | HTTP 路由、请求验证、响应序列化、中间件            |
| Memory Guard      | 写入约束检查、层级权限控制、更新模式处理           |
| Retrieval Engine  | 多阶段检索：结构过滤→全文检索→向量召回→综合打分    |
| Memory Processor  | 可选的 LLM 处理：记忆压缩、提纯、分类、关键词提取  |
| Config Center     | Embedding/LLM Provider 管理、热更新、Rate Limiting |
| Lifecycle Manager | 状态机转换、TTL 处理、审计日志记录                 |

### 技术栈

| 层            | 技术                                  |
| ------------- | ------------------------------------- |
| Web Framework | Axum 0.7                              |
| Async Runtime | Tokio                                 |
| Database      | PostgreSQL + sqlx                     |
| Vector DB     | Qdrant                                |
| Serialization | serde + serde_json                    |
| Config        | config-rs + YAML                      |
| Logging       | tracing + tracing-subscriber          |
| Metrics       | metrics + metrics-exporter-prometheus |


## Components and Interfaces

### API Layer

#### Memory CRUD API

```rust
// POST /api/v1/memories
struct CreateMemoryRequest {
    layer: Layer,           // session | task (long-term 禁止直接创建)
    scope_type: ScopeType,  // user | org | project | task | session
    scope_id: String,
    scene: String,          // e.g., "work.contract_review"
    content: String,        // Markdown 格式，可以是原始对话记录
    importance: Option<f32>,    // 0.0 - 1.0, Agent/LLM 根据用户行为上下文决定
    confidence: Option<f32>,    // 0.0 - 1.0
    ttl_seconds: Option<i64>,
    event_source: Option<String>,   // 触发记忆的用户行为，如 "button_click:like", "conversation:preference"
    event_time: Option<DateTime<Utc>>,  // 用户行为发生时间
    embedding_provider: Option<String>,  // 指定 embedding provider，否则用默认
    // 新增：LLM 处理选项
    process_with_llm: Option<bool>,     // 是否启用 LLM 处理，默认 false
    llm_provider: Option<String>,       // 指定 LLM provider，否则用默认
}

struct CreateMemoryResponse {
    id: Uuid,
    created_at: DateTime<Utc>,
    embedding_status: EmbeddingStatus,  // pending | completed | failed
    processing_status: Option<ProcessingStatus>,  // pending | completed | failed | skipped
    category: Option<MemoryCategory>,   // LLM 自动分类结果
}

// GET /api/v1/memories/{id}
struct GetMemoryResponse {
    id: Uuid,
    layer: Layer,
    scope_type: ScopeType,
    scope_id: String,
    scene: String,
    status: Status,
    content: String,                    // 处理后的内容（如果启用了 LLM 处理）
    raw_content: Option<String>,        // 原始内容（如果启用了 LLM 处理）
    category: Option<MemoryCategory>,   // 记忆分类
    tags: Option<Vec<String>>,          // 提取的标签/关键词
    importance: f32,
    confidence: f32,
    hit_count: i64,
    last_hit_at: Option<DateTime<Utc>>,
    ttl: Option<i64>,
    event_source: Option<String>,
    event_time: Option<DateTime<Utc>>,
    processing_status: Option<ProcessingStatus>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

// PUT /api/v1/memories/{id}
struct UpdateMemoryRequest {
    mode: UpdateMode,       // append | merge | supersede
    content: Option<String>,
    importance: Option<f32>,
    confidence: Option<f32>,
    ttl_seconds: Option<i64>,
    process_with_llm: Option<bool>,     // 是否对更新内容启用 LLM 处理
}

// DELETE /api/v1/memories/{id}
// 软删除，标记为 archived

// 记忆分类枚举
enum MemoryCategory {
    UserPreference,     // 用户偏好（如：喜欢深色主题）
    BehaviorPattern,    // 行为模式（如：习惯在早上处理邮件）
    BusinessRule,       // 业务规则（如：合同审批需要三级签字）
    FactualKnowledge,   // 事实知识（如：项目截止日期是 X）
    Other,              // 其他
}

// 处理状态枚举
enum ProcessingStatus {
    Pending,    // 等待处理
    Completed,  // 处理完成
    Failed,     // 处理失败
    Skipped,    // 跳过（未启用 LLM 处理）
}
```

#### Retrieval API

```rust
// POST /api/v1/memories/retrieve
struct RetrieveRequest {
    query: String,                      // 检索文本（用于向量检索和全文检索）
    scope_type: Option<ScopeType>,
    scope_id: Option<String>,
    layers: Option<Vec<Layer>>,         // 过滤层级
    scenes: Option<Vec<String>>,        // 过滤场景
    categories: Option<Vec<MemoryCategory>>,  // 按分类过滤
    tags: Option<Vec<String>>,          // 按标签过滤
    event_source_prefix: Option<String>, // 按事件来源前缀过滤
    top_k: Option<usize>,               // 默认 10
    min_score: Option<f32>,             // 最低分数阈值
    // 检索模式控制
    use_fulltext: Option<bool>,         // 是否启用全文检索，默认 true
    use_vector: Option<bool>,           // 是否启用向量检索，默认 true
    fulltext_weight: Option<f32>,       // 全文检索权重，默认 0.3
    highlight: Option<bool>,            // 是否返回高亮片段，默认 false
}

struct RetrieveResponse {
    memories: Vec<RetrievedMemory>,
    total_candidates: usize,            // 结构过滤后的候选数
}

struct RetrievedMemory {
    memory: GetMemoryResponse,
    score: f32,                         // 综合得分
    similarity: f32,                    // 向量相似度
    text_match_score: Option<f32>,      // 全文匹配得分
    highlights: Option<Vec<String>>,    // 高亮片段
}
```

#### Config API

```rust
// GET /api/v1/config/providers
struct ListProvidersResponse {
    providers: Vec<ProviderInfo>,
    default_provider: String,
}

struct ProviderInfo {
    name: String,
    provider_type: ProviderType,  // openai | azure | local
    enabled: bool,
    model: String,
    dimension: usize,
    rate_limit: RateLimitConfig,
}

// POST /api/v1/config/providers
struct CreateProviderRequest {
    name: String,
    provider_type: ProviderType,
    endpoint: String,
    api_key: Option<String>,      // 敏感信息，存储时加密
    model: String,
    dimension: usize,
    rate_limit: Option<RateLimitConfig>,
}

// PUT /api/v1/config/providers/{name}
struct UpdateProviderRequest {
    endpoint: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    enabled: Option<bool>,
    rate_limit: Option<RateLimitConfig>,
}

// PUT /api/v1/config/default-provider
struct SetDefaultProviderRequest {
    provider_name: String,
}

struct RateLimitConfig {
    requests_per_minute: Option<u32>,
    tokens_per_minute: Option<u32>,
}
```

#### Health & Metrics API

```rust
// GET /health
struct HealthResponse {
    status: HealthStatus,       // healthy | degraded | unhealthy
    postgres: ComponentHealth,
    qdrant: ComponentHealth,
    embedding: ComponentHealth,
    version: String,
    uptime_seconds: u64,
}

struct ComponentHealth {
    status: HealthStatus,
    latency_ms: Option<u64>,
    error: Option<String>,
}

// GET /metrics
// Prometheus 格式输出
```

#### Audit API

```rust
// GET /api/v1/audit
struct AuditQueryRequest {
    memory_id: Option<Uuid>,
    start_time: Option<DateTime<Utc>>,
    end_time: Option<DateTime<Utc>>,
    operation: Option<AuditOperation>,
    limit: Option<usize>,
    offset: Option<usize>,
}

struct AuditLogEntry {
    id: Uuid,
    memory_id: Uuid,
    operation: AuditOperation,  // create | update | delete | status_change
    actor_id: Option<String>,
    changes: serde_json::Value,
    created_at: DateTime<Utc>,
}
```


## Data Models

### PostgreSQL Schema

```sql
-- 枚举类型
CREATE TYPE layer AS ENUM ('session', 'task', 'long_term');
CREATE TYPE scope_type AS ENUM ('user', 'org', 'project', 'task', 'session');
CREATE TYPE status AS ENUM ('candidate', 'active', 'stable', 'cooldown', 'ignored', 'archived');
CREATE TYPE embedding_status AS ENUM ('pending', 'completed', 'failed');
CREATE TYPE processing_status AS ENUM ('pending', 'completed', 'failed', 'skipped');
CREATE TYPE update_mode AS ENUM ('append', 'merge', 'supersede');
CREATE TYPE provider_type AS ENUM ('openai', 'azure', 'local');
CREATE TYPE memory_category AS ENUM ('user_preference', 'behavior_pattern', 'business_rule', 'factual_knowledge', 'other');

-- Memory 主表
CREATE TABLE memories (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    layer layer NOT NULL,
    scope_type scope_type NOT NULL,
    scope_id VARCHAR(255) NOT NULL,
    scene VARCHAR(255) NOT NULL,
    status status NOT NULL DEFAULT 'active',
    -- 内容字段
    content TEXT NOT NULL,                      -- 用于检索的内容（处理后或原始）
    raw_content TEXT,                           -- 原始内容（如果启用了 LLM 处理）
    -- 分类和标签
    category memory_category,                   -- LLM 自动分类
    tags TEXT[],                                -- 提取的标签/关键词
    -- 全文检索
    content_tsv TSVECTOR,                       -- 全文检索向量
    -- 元数据
    importance REAL NOT NULL DEFAULT 0.5,
    confidence REAL NOT NULL DEFAULT 1.0,
    hit_count BIGINT NOT NULL DEFAULT 0,
    last_hit_at TIMESTAMPTZ,
    ttl_seconds BIGINT,
    expires_at TIMESTAMPTZ,
    event_source VARCHAR(255),
    event_time TIMESTAMPTZ,
    -- 处理状态
    embedding_status embedding_status NOT NULL DEFAULT 'pending',
    embedding_provider VARCHAR(100),
    processing_status processing_status NOT NULL DEFAULT 'skipped',
    llm_provider VARCHAR(100),
    -- 时间戳
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 索引
CREATE INDEX idx_memories_scope ON memories(scope_type, scope_id);
CREATE INDEX idx_memories_scene ON memories(scene);
CREATE INDEX idx_memories_layer ON memories(layer);
CREATE INDEX idx_memories_status ON memories(status);
CREATE INDEX idx_memories_category ON memories(category) WHERE category IS NOT NULL;
CREATE INDEX idx_memories_tags ON memories USING GIN(tags) WHERE tags IS NOT NULL;
CREATE INDEX idx_memories_expires_at ON memories(expires_at) WHERE expires_at IS NOT NULL;
CREATE INDEX idx_memories_last_hit_at ON memories(last_hit_at);
CREATE INDEX idx_memories_event_source ON memories(event_source) WHERE event_source IS NOT NULL;
-- 全文检索索引
CREATE INDEX idx_memories_content_tsv ON memories USING GIN(content_tsv);

-- 自动更新 tsvector 的触发器
CREATE OR REPLACE FUNCTION memories_update_tsv() RETURNS trigger AS $$
BEGIN
    -- 使用 'simple' 配置，或者如果安装了中文分词插件可以使用 'zhparser' 或 'jiebacfg'
    NEW.content_tsv := to_tsvector('simple', COALESCE(NEW.content, ''));
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER memories_tsv_trigger
    BEFORE INSERT OR UPDATE OF content ON memories
    FOR EACH ROW EXECUTE FUNCTION memories_update_tsv();

-- Embedding Provider 配置表
CREATE TABLE embedding_providers (
    name VARCHAR(100) PRIMARY KEY,
    provider_type provider_type NOT NULL,
    endpoint VARCHAR(500) NOT NULL,
    api_key_encrypted BYTEA,
    model VARCHAR(200) NOT NULL,
    dimension INTEGER NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    is_default BOOLEAN NOT NULL DEFAULT false,
    rpm_limit INTEGER,
    tpm_limit INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- LLM Provider 配置表（用于记忆处理）
CREATE TABLE llm_providers (
    name VARCHAR(100) PRIMARY KEY,
    provider_type provider_type NOT NULL,
    endpoint VARCHAR(500) NOT NULL,
    api_key_encrypted BYTEA,
    model VARCHAR(200) NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    is_default BOOLEAN NOT NULL DEFAULT false,
    rpm_limit INTEGER,
    tpm_limit INTEGER,
    -- 处理配置
    compression_prompt TEXT,            -- 压缩提纯的 prompt 模板
    classification_prompt TEXT,         -- 分类的 prompt 模板
    max_input_tokens INTEGER DEFAULT 4000,
    max_output_tokens INTEGER DEFAULT 1000,
    temperature REAL DEFAULT 0.3,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 审计日志表
CREATE TABLE audit_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    memory_id UUID NOT NULL REFERENCES memories(id),
    operation VARCHAR(50) NOT NULL,
    actor_id VARCHAR(255),
    old_value JSONB,
    new_value JSONB,
    reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_logs_memory_id ON audit_logs(memory_id);
CREATE INDEX idx_audit_logs_created_at ON audit_logs(created_at);

-- 生命周期配置表
CREATE TABLE lifecycle_config (
    key VARCHAR(100) PRIMARY KEY,
    value JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 默认生命周期配置
INSERT INTO lifecycle_config (key, value) VALUES
    ('cooldown_threshold_days', '{"session": 1, "task": 7, "long_term": 30}'),
    ('default_ttl_seconds', '{"session": 3600, "task": 604800, "long_term": null}');

-- 默认 LLM 处理 prompt 模板
INSERT INTO lifecycle_config (key, value) VALUES
    ('compression_prompt', '"你是一个记忆压缩助手。请从以下对话/操作记录中提取关键信息，生成简洁的结构化记忆。\n\n要求：\n1. 保留核心事实和用户偏好\n2. 去除冗余和无关信息\n3. 使用简洁的陈述句\n4. 保持原意不变\n\n原始内容：\n{content}\n\n压缩后的记忆："'),
    ('classification_prompt', '"请将以下记忆分类到最合适的类别：\n- user_preference: 用户偏好（如喜好、习惯设置）\n- behavior_pattern: 行为模式（如工作习惯、操作方式）\n- business_rule: 业务规则（如流程、规定）\n- factual_knowledge: 事实知识（如日期、数据）\n- other: 其他\n\n记忆内容：\n{content}\n\n请只返回类别名称："');
```

### Qdrant Collection Schema

```json
{
  "collection_name": "memories",
  "vectors": {
    "size": 1536,
    "distance": "Cosine"
  },
  "payload_schema": {
    "memory_id": "uuid",
    "layer": "keyword",
    "scope_type": "keyword",
    "scope_id": "keyword",
    "scene": "keyword",
    "status": "keyword",
    "importance": "float",
    "confidence": "float",
    "event_source": "keyword",
    "created_at": "datetime"
  }
}
```

### Rust Domain Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "layer", rename_all = "snake_case")]
pub enum Layer {
    Session,
    Task,
    LongTerm,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "scope_type", rename_all = "snake_case")]
pub enum ScopeType {
    User,
    Org,
    Project,
    Task,
    Session,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "status", rename_all = "snake_case")]
pub enum Status {
    Candidate,
    Active,
    Stable,
    Cooldown,
    Ignored,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "memory_category", rename_all = "snake_case")]
pub enum MemoryCategory {
    UserPreference,
    BehaviorPattern,
    BusinessRule,
    FactualKnowledge,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "processing_status", rename_all = "snake_case")]
pub enum ProcessingStatus {
    Pending,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct Memory {
    pub id: Uuid,
    pub layer: Layer,
    pub scope_type: ScopeType,
    pub scope_id: String,
    pub scene: String,
    pub status: Status,
    pub content: String,                        // 用于检索的内容
    pub raw_content: Option<String>,            // 原始内容
    pub category: Option<MemoryCategory>,       // 记忆分类
    pub tags: Option<Vec<String>>,              // 标签/关键词
    pub importance: f32,
    pub confidence: f32,
    pub hit_count: i64,
    pub last_hit_at: Option<DateTime<Utc>>,
    pub ttl_seconds: Option<i64>,
    pub expires_at: Option<DateTime<Utc>>,
    pub event_source: Option<String>,
    pub event_time: Option<DateTime<Utc>>,
    pub embedding_status: EmbeddingStatus,
    pub embedding_provider: Option<String>,
    pub processing_status: ProcessingStatus,
    pub llm_provider: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

### Memory Processor 接口

```rust
/// LLM 处理请求
struct ProcessMemoryRequest {
    content: String,                    // 原始内容
    context: Option<String>,            // 可选的上下文信息
}

/// LLM 处理结果
struct ProcessMemoryResult {
    processed_content: String,          // 压缩提纯后的内容
    category: MemoryCategory,           // 自动分类
    tags: Vec<String>,                  // 提取的标签
    confidence: f32,                    // 处理置信度
}

/// Memory Processor trait
#[async_trait]
pub trait MemoryProcessor: Send + Sync {
    /// 处理原始内容，返回压缩提纯后的结果
    async fn process(&self, request: ProcessMemoryRequest) -> Result<ProcessMemoryResult, ProcessingError>;
    
    /// 仅分类
    async fn classify(&self, content: &str) -> Result<MemoryCategory, ProcessingError>;
    
    /// 仅提取标签
    async fn extract_tags(&self, content: &str) -> Result<Vec<String>, ProcessingError>;
}
```


## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system—essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property 1: Memory CRUD Round-Trip

*For any* valid memory creation request (with layer ∈ {session, task}), creating the memory and then querying it by ID should return a memory with identical layer, scope_type, scope_id, scene, content, event_source, and event_time fields.

**Validates: Requirements 1.1, 2.1**

### Property 2: Long-Term Layer Rejection

*For any* memory creation request with layer set to "long_term", the Memory_Server should reject the request with a validation error, regardless of other field values.

**Validates: Requirements 1.5**

### Property 3: Update Mode Correctness

*For any* existing memory and any update request:
- If mode is "append", the resulting content should contain both original and new content
- If mode is "supersede", the resulting content should equal only the new content
- If mode is "merge", the resulting content should contain both original and new content

**Validates: Requirements 2.2, 2.3, 2.4**

### Property 4: Soft Delete Preservation

*For any* memory that is deleted, querying it by ID should still return the memory with status "archived", and the content should be preserved.

**Validates: Requirements 2.6**

### Property 5: Invalid Input Rejection

*For any* create or update request with an invalid scope_type value, the Memory_Server should return a validation error and not persist any changes.

**Validates: Requirements 2.7**

### Property 6: Retrieval Result Ordering

*For any* retrieval request returning multiple memories, the results should be sorted in descending order by composite score.

**Validates: Requirements 3.4**

### Property 7: Hit Count Increment

*For any* memory returned in retrieval results, its hit_count should be incremented by 1 and last_hit_at should be updated to a timestamp >= the retrieval request time.

**Validates: Requirements 3.5**

### Property 8: Status-Based Exclusion

*For any* retrieval request, memories with status "ignored" or "archived" should never appear in the results, regardless of their similarity score or other attributes.

**Validates: Requirements 3.6**

### Property 9: Cooldown Score Penalty

*For any* two memories with identical content, importance, and recency, if one has status "active" and the other has status "cooldown", the active memory should have a higher composite score.

**Validates: Requirements 3.7**

### Property 10: Cooldown Recovery

*For any* memory with status "cooldown", if it is hit (returned in retrieval results), its status should transition to "active".

**Validates: Requirements 4.3**

### Property 11: Audit Log Completeness

*For any* memory operation (create, update, delete, status_change), an audit log entry should be created containing: timestamp, operation type, memory_id, and the change details (old_value and new_value for updates).

**Validates: Requirements 4.5, 8.1, 8.2**

### Property 12: State Transition Audit

*For any* status transition of a memory, the audit log should record both the old status and new status.

**Validates: Requirements 4.4, 4.5**

### Property 13: Event Source Traceability

*For any* memory created with event_source and event_time, querying the memory should return the same event_source and event_time values.

**Validates: Requirements 1.2, 1.3, 1.6**

### Property 14: Event Source Prefix Filtering

*For any* retrieval request with event_source_prefix filter, all returned memories should have event_source starting with the specified prefix.

**Validates: Requirements 3.9**

### Property 15: Full-Text Search Relevance

*For any* retrieval request with a text query, memories containing the exact query terms should have a higher text_match_score than memories not containing those terms.

**Validates: Requirements 13.3, 13.4**

### Property 16: LLM Processing Preservation

*For any* memory created with `process_with_llm=true`, the raw_content field should contain the original input, and the content field should contain the processed result.

**Validates: Requirements 12.4**

### Property 17: Category Classification Consistency

*For any* memory processed with LLM, the category field should be one of the valid MemoryCategory values (user_preference, behavior_pattern, business_rule, factual_knowledge, other).

**Validates: Requirements 12.3**

### Property 18: Processing Fallback

*For any* memory where LLM processing fails, the content field should contain the original raw content, and processing_status should be "failed".

**Validates: Requirements 12.7**

### Property 19: Category Filter Correctness

*For any* retrieval request with category filter, all returned memories should have a category matching one of the specified categories.

**Validates: Requirements 3.10**


## Error Handling

### Error Response Format

```rust
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub code: String,           // 机器可读错误码
    pub message: String,        // 人类可读描述
    pub details: Option<serde_json::Value>,
    pub request_id: String,
}
```

### Error Categories

| 错误码                | HTTP Status | 场景                          |
| --------------------- | ----------- | ----------------------------- |
| `VALIDATION_ERROR`    | 400         | 请求参数验证失败              |
| `INVALID_LAYER`       | 400         | 尝试直接创建 long-term memory |
| `INVALID_SCOPE_TYPE`  | 400         | 无效的 scope_type             |
| `INVALID_UPDATE_MODE` | 400         | 无效的更新模式                |
| `MEMORY_NOT_FOUND`    | 404         | Memory ID 不存在              |
| `PROVIDER_NOT_FOUND`  | 404         | Embedding provider 不存在     |
| `PROVIDER_DISABLED`   | 400         | 指定的 provider 已禁用        |
| `NO_DEFAULT_PROVIDER` | 500         | 没有可用的默认 provider       |
| `EMBEDDING_FAILED`    | 500         | Embedding 生成失败（重试后）  |
| `RATE_LIMIT_EXCEEDED` | 429         | Provider rate limit 超限      |
| `DATABASE_ERROR`      | 500         | PostgreSQL 操作失败           |
| `VECTOR_DB_ERROR`     | 500         | Qdrant 操作失败               |
| `CONFIG_ERROR`        | 500         | 配置加载/验证失败             |

### Retry Strategy

```rust
pub struct RetryConfig {
    pub max_retries: u32,           // 默认 3
    pub initial_delay_ms: u64,      // 默认 100
    pub max_delay_ms: u64,          // 默认 5000
    pub multiplier: f64,            // 默认 2.0
}
```

对于 Embedding 调用：
1. 首次失败后等待 100ms 重试
2. 第二次失败后等待 200ms 重试
3. 第三次失败后等待 400ms 重试
4. 三次都失败则标记 `embedding_status = failed`

### Graceful Degradation

- PostgreSQL 不可用：服务不可用，返回 503
- Qdrant 不可用：降级为仅结构化检索，返回结果但标记 `degraded: true`
- Embedding Provider 不可用：Memory 创建成功但 `embedding_status = pending`，后台重试

## Testing Strategy

### 测试框架选择

- **单元测试**: Rust 内置 `#[test]`
- **Property-Based Testing**: `proptest` crate
- **集成测试**: `tokio-test` + `testcontainers`

### Property-Based Testing 配置

```rust
// proptest.toml
[default]
cases = 100
max_shrink_iters = 1000
```

### 测试分层

| 层级        | 覆盖范围                   | 工具             |
| ----------- | -------------------------- | ---------------- |
| Unit        | 纯函数、数据转换、验证逻辑 | `#[test]`        |
| Property    | 核心不变量、CRUD 正确性    | `proptest`       |
| Integration | API 端到端、数据库交互     | `testcontainers` |

### Property Test 示例

```rust
use proptest::prelude::*;

// Property 1: Memory CRUD Round-Trip
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    
    /// Feature: memory-server, Property 1: Memory CRUD Round-Trip
    /// Validates: Requirements 1.1, 1.3
    #[test]
    fn prop_memory_crud_roundtrip(
        layer in prop_oneof![Just(Layer::Session), Just(Layer::Task)],
        scope_type in any::<ScopeType>(),
        scope_id in "[a-z0-9]{8,32}",
        scene in "[a-z]+\\.[a-z_]+",
        content in ".{1,1000}",
    ) {
        // Create memory
        let request = CreateMemoryRequest { layer, scope_type, scope_id, scene, content };
        let response = create_memory(request.clone());
        
        // Query memory
        let memory = get_memory(response.id);
        
        // Verify round-trip
        prop_assert_eq!(memory.layer, request.layer);
        prop_assert_eq!(memory.scope_type, request.scope_type);
        prop_assert_eq!(memory.scope_id, request.scope_id);
        prop_assert_eq!(memory.scene, request.scene);
        prop_assert_eq!(memory.content, request.content);
    }
}

// Property 2: Long-Term Layer Rejection
proptest! {
    /// Feature: memory-server, Property 2: Long-Term Layer Rejection
    /// Validates: Requirements 1.2
    #[test]
    fn prop_longterm_rejection(
        scope_type in any::<ScopeType>(),
        scope_id in "[a-z0-9]{8,32}",
        scene in "[a-z]+\\.[a-z_]+",
        content in ".{1,1000}",
    ) {
        let request = CreateMemoryRequest {
            layer: Layer::LongTerm,
            scope_type,
            scope_id,
            scene,
            content,
        };
        let result = create_memory(request);
        prop_assert!(result.is_err());
        prop_assert!(matches!(result.unwrap_err().code, "INVALID_LAYER"));
    }
}
```

### 单元测试覆盖

- 输入验证函数
- Score 计算逻辑
- 状态机转换规则
- 配置解析

### 集成测试覆盖

- API 端到端流程
- PostgreSQL 事务正确性
- Qdrant 向量存储/检索
- Embedding Provider 调用（Mock）


## Deployment Architecture

### Docker Compose 配置

```yaml
# docker-compose.yml
version: '3.8'

services:
  memory-server:
    build:
      context: .
      dockerfile: Dockerfile
    ports:
      - "${MEMORY_SERVER_PORT:-8080}:8080"
    environment:
      - DATABASE_URL=postgres://memory:memory@postgres:5432/memory_db
      - QDRANT_URL=http://qdrant:6334
      - CONFIG_PATH=/app/config/config.yaml
      - RUST_LOG=info
    volumes:
      - ./config:/app/config:ro
    depends_on:
      postgres:
        condition: service_healthy
      qdrant:
        condition: service_healthy
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 10s
      timeout: 5s
      retries: 3
      start_period: 10s
    restart: unless-stopped

  postgres:
    image: postgres:16-alpine
    environment:
      - POSTGRES_USER=memory
      - POSTGRES_PASSWORD=memory
      - POSTGRES_DB=memory_db
    volumes:
      - postgres_data:/var/lib/postgresql/data
      - ./migrations:/docker-entrypoint-initdb.d:ro
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U memory -d memory_db"]
      interval: 5s
      timeout: 5s
      retries: 5
    restart: unless-stopped

  qdrant:
    image: qdrant/qdrant:v1.7.4
    ports:
      - "${QDRANT_PORT:-6333}:6333"
      - "${QDRANT_GRPC_PORT:-6334}:6334"
    volumes:
      - qdrant_data:/qdrant/storage
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:6333/health"]
      interval: 5s
      timeout: 5s
      retries: 5
    restart: unless-stopped

volumes:
  postgres_data:
  qdrant_data:
```

### Dockerfile

```dockerfile
# Dockerfile
FROM rust:1.75-slim as builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN apt-get update && apt-get install -y pkg-config libssl-dev && \
    cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/memory-server /app/memory-server

EXPOSE 8080
CMD ["/app/memory-server"]
```

### 配置文件示例

```yaml
# config/config.yaml
server:
  host: "0.0.0.0"
  port: 8080

database:
  url: "${DATABASE_URL}"
  max_connections: 10
  min_connections: 2

qdrant:
  url: "${QDRANT_URL}"
  collection_name: "memories"

embedding:
  default_provider: "openai"
  providers:
    - name: "openai"
      type: "openai"
      endpoint: "https://api.openai.com/v1"
      api_key: "${OPENAI_API_KEY}"
      model: "text-embedding-3-small"
      dimension: 1536
      rate_limit:
        requests_per_minute: 500
        tokens_per_minute: 1000000
    
    - name: "local-bge"
      type: "local"
      endpoint: "http://embedding-service:8000/v1"
      model: "bge-large-zh-v1.5"
      dimension: 1024
      rate_limit:
        requests_per_minute: 1000

# LLM 处理配置（可选）
llm:
  default_provider: "openai-gpt4"
  providers:
    - name: "openai-gpt4"
      type: "openai"
      endpoint: "https://api.openai.com/v1"
      api_key: "${OPENAI_API_KEY}"
      model: "gpt-4o-mini"
      max_input_tokens: 4000
      max_output_tokens: 1000
      temperature: 0.3
      rate_limit:
        requests_per_minute: 100
        tokens_per_minute: 100000
    
    - name: "local-ollama"
      type: "local"
      endpoint: "http://ollama:11434/v1"
      model: "qwen2.5:7b"
      max_input_tokens: 8000
      max_output_tokens: 2000
      temperature: 0.3

lifecycle:
  cooldown_check_interval_seconds: 3600
  cooldown_thresholds:
    session: 86400      # 1 day
    task: 604800        # 7 days
    long_term: 2592000  # 30 days
  default_ttl:
    session: 3600       # 1 hour
    task: 604800        # 7 days
    long_term: null     # no expiry

retrieval:
  default_top_k: 10
  max_top_k: 100
  cooldown_penalty: 0.5
  score_weights:
    similarity: 0.3
    text_match: 0.2     # 全文检索权重
    importance: 0.25
    recency: 0.15
    hit_count: 0.1
  fulltext:
    enabled: true
    language: "simple"  # 或 "zhparser" / "jiebacfg" 用于中文

audit:
  enabled: true
  retention_days: 90
```

### 项目目录结构

```
memory-server/
├── Cargo.toml
├── Cargo.lock
├── Dockerfile
├── docker-compose.yml
├── config/
│   └── config.yaml
├── migrations/
│   ├── 001_init.sql
│   └── 002_fulltext_and_processing.sql
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── config/
│   │   └── mod.rs
│   ├── api/
│   │   ├── mod.rs
│   │   ├── memory.rs
│   │   ├── retrieval.rs
│   │   ├── config.rs
│   │   ├── audit.rs
│   │   └── health.rs
│   ├── domain/
│   │   ├── mod.rs
│   │   ├── memory.rs
│   │   ├── layer.rs
│   │   ├── scope.rs
│   │   ├── status.rs
│   │   └── category.rs
│   ├── service/
│   │   ├── mod.rs
│   │   ├── memory_guard.rs
│   │   ├── memory_processor.rs      # LLM 处理服务
│   │   ├── retrieval_engine.rs
│   │   ├── lifecycle_manager.rs
│   │   └── config_center.rs
│   ├── repository/
│   │   ├── mod.rs
│   │   ├── memory_repo.rs
│   │   ├── audit_repo.rs
│   │   └── config_repo.rs
│   ├── embedding/
│   │   ├── mod.rs
│   │   ├── provider.rs
│   │   ├── openai.rs
│   │   └── local.rs
│   ├── llm/                          # LLM 处理模块
│   │   ├── mod.rs
│   │   ├── provider.rs
│   │   ├── openai.rs
│   │   └── local.rs
│   └── error.rs
└── tests/
    ├── common/
    │   └── mod.rs
    ├── property_tests/
    │   ├── mod.rs
    │   ├── memory_crud.rs
    │   ├── retrieval.rs
    │   └── processing.rs
    └── integration/
        ├── mod.rs
        └── api_tests.rs
```
