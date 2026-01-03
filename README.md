# Causality 因果律(Memory Framework)

> **一个事件驱动的大模型/智能体应用记忆治理框架**  
> An Event-Driven Memory Governance Framework for LLM/Agent Applications

[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Tests](https://img.shields.io/badge/tests-171%20passing-green.svg)](#)

## 🎯 核心定位 | Core Positioning

Memory Framework 是一个独立的、可嵌入任意 Agent/LLM 应用的**记忆治理服务**，专注于**上下文记忆的结构化存储、检索、演化与治理**。

- ✅ **不负责推理** - 专注记忆管理，不绑定特定模型
- ✅ **不侵入业务** - 通过 HTTP/gRPC API 提供服务
- ✅ **事件驱动** - 基于应用事件自动管理记忆生命周期
- ✅ **多层设计** - 分层管理不同时效性的记忆

## 🏗️ 架构设计 | Architecture

### 事件驱动架构 | Event-Driven Architecture

```mermaid
graph TB
    A[Agent/LLM Application] -->|Events| B[Memory Server]
    B --> C[Memory Guard]
    B --> D[Retrieval Engine] 
    B --> E[Lifecycle Manager]
    
    C --> F[PostgreSQL]
    D --> G[Vector DB]
    E --> F
    
    F --> H[Memory Metadata]
    G --> I[Embeddings]
    
    subgraph "Event Sources"
        J[User Interactions]
        K[Task Completion]
        L[Conversation Context]
        M[System Events]
    end
    
    J --> A
    K --> A
    L --> A
    M --> A
```

### 多层记忆设计 | Multi-Layer Memory Design

Memory Framework 采用三层记忆架构，根据记忆的时效性和重要性进行分层管理：

```
┌─────────────────────────────────────────────────────────────┐
│                    Long-term Memory                         │
│  ┌─────────────────────────────────────────────────────┐   │
│  │                Task Memory                          │   │
│  │  ┌─────────────────────────────────────────────┐   │   │
│  │  │            Session Memory                   │   │   │
│  │  │                                             │   │   │
│  │  │  • 短期上下文 (分钟/小时级)                  │   │   │
│  │  │  • TTL: 1小时                               │   │   │
│  │  │  • 自动过期                                 │   │   │
│  │  └─────────────────────────────────────────────┘   │   │
│  │                                                     │   │
│  │  • 任务级结论 (天/周级)                             │   │
│  │  • TTL: 7天                                        │   │
│  │  • 任务完成后归档                                   │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
│  • 长期稳定规则                                             │
│  • 无自动过期                                               │
│  • 需要人工确认                                             │
└─────────────────────────────────────────────────────────────┘
```

#### 层级特性 | Layer Characteristics

| 层级          | TTL   | 用途                 | 创建方式 |
| ------------- | ----- | -------------------- | -------- |
| **Session**   | 1小时 | 对话上下文、临时偏好 | 自动创建 |
| **Task**      | 7天   | 任务结论、工作流程   | 自动创建 |
| **Long-term** | 永久  | 用户偏好、业务规则   | 需要确认 |

### 事件驱动的记忆管理 | Event-Driven Memory Management

#### 1. 事件源 | Event Sources

```rust
// 用户交互事件
CreateMemoryInput {
    event_source: Some("button_click:like"),
    event_time: Some(Utc::now()),
    content: "用户喜欢深色主题",
    // ...
}

// 任务完成事件  
CreateMemoryInput {
    event_source: Some("task_completion:contract_review"),
    event_time: Some(Utc::now()),
    content: "合同审查流程偏好使用模板A",
    // ...
}
```

#### 2. 自动生命周期管理 | Automatic Lifecycle Management

```rust
// 基于事件的状态转换
pub enum Status {
    Active,      // 活跃使用中
    Cooldown,    // 冷却期（长期未使用）
    Archived,    // 已归档
}

// 自动冷却机制
impl Memory {
    pub fn should_cooldown(&self, threshold_seconds: i64) -> bool {
        // 基于最后命中时间自动判断是否进入冷却
    }
}
```

#### 3. 智能检索与反馈 | Intelligent Retrieval & Feedback

```rust
// 每次检索命中都会触发反馈事件
pub fn record_hit(&mut self) {
    self.hit_count += 1;
    self.last_hit_at = Some(Utc::now());
    
    // 从冷却状态恢复到活跃状态
    if self.status == Status::Cooldown {
        self.status = Status::Active;
    }
}
```

## 🚀 核心特性 | Key Features

### 1. 双路径记忆创建 | Dual-Path Memory Creation

Memory Framework 提供两种创建记忆的方式，满足不同场景需求：

#### 路径一：直接创建 | Direct Creation

用户完全控制记忆内容，自己负责所有字段（content, category, tags, importance 等）。

```bash
# 直接创建记忆 - 用户指定所有字段
curl -X POST http://localhost:8080/api/v1/memories \
  -H "Content-Type: application/json" \
  -d '{
    "layer": "task",
    "scope_type": "user",
    "scope_id": "user_123",
    "scene": "work.preference",
    "content": "用户偏好使用深色主题",
    "category": "preference",
    "tags": ["ui", "theme", "dark-mode"],
    "importance": 0.8
  }'
```

#### 路径二：事件提取 | Event Extraction

提交原始事件（对话、操作记录等），由 LLM 自动提取和填充字段。

```bash
# 从事件提取记忆 - LLM 自动分析和提取
curl -X POST http://localhost:8080/api/v1/memories/from-event \
  -H "Content-Type: application/json" \
  -d '{
    "content": "用户：我不喜欢这个界面太亮了，能换成深色的吗？\n助手：好的，已为您切换到深色主题。\n用户：这样好多了，以后都用这个。",
    "context": "用户设置对话",
    "mode": "auto",
    "scope_type": "user",
    "scope_id": "user_123",
    "scene": "ui.preference"
  }'
```

#### 两种路径对比 | Path Comparison

| 特性          | 直接创建                | 事件提取                                   |
| ------------- | ----------------------- | ------------------------------------------ |
| **API 端点**  | `POST /api/v1/memories` | `POST /api/v1/memories/from-event`         |
| **内容来源**  | 用户整理好的结构化内容  | 原始事件（对话、日志等）                   |
| **字段填充**  | 用户自己指定            | LLM 自动提取                               |
| **分类/标签** | 用户提供（可选）        | LLM 自动生成                               |
| **推断信息**  | 无                      | 包含 inference_type, confidence, reasoning |
| **适用场景**  | 已知明确的记忆内容      | 需要从原始数据中提取洞察                   |

### 2. 事件处理模式 | Event Processing Modes

事件提取支持三种处理模式：

| 模式         | 说明                                 | 返回结果                                  |
| ------------ | ------------------------------------ | ----------------------------------------- |
| **auto**     | 全自动：提取并创建记忆               | `created_memory_ids` 包含已创建的记忆 ID  |
| **assisted** | 辅助模式（默认）：返回建议，等待确认 | `extracted_memories` 包含建议，不自动创建 |
| **manual**   | 手动模式：仅摘要，不提取             | 仅返回 `event_summary`                    |

```bash
# Assisted 模式 - 返回建议供用户确认
curl -X POST http://localhost:8080/api/v1/memories/from-event \
  -H "Content-Type: application/json" \
  -d '{
    "content": "长对话内容...",
    "mode": "assisted",
    "scope_type": "user",
    "scope_id": "user_123",
    "scene": "conversation"
  }'

# 响应示例
{
  "event_summary": "用户讨论了界面偏好设置",
  "extracted_memories": [
    {
      "content": "用户偏好深色主题",
      "inference_type": "preference",
      "confidence": 0.85,
      "category": "preference",
      "tags": ["ui", "theme"],
      "importance": 0.7,
      "reasoning": "用户明确表示喜欢深色主题并要求以后都使用"
    }
  ],
  "created_memory_ids": null
}
```

### 3. 查询增强 | Query Enhancement

检索时可启用查询增强，LLM 会扩展查询语义，提高检索准确性：

```bash
# 启用查询增强
curl -X POST http://localhost:8080/api/v1/memories/retrieve \
  -H "Content-Type: application/json" \
  -d '{
    "query": "用户界面偏好",
    "scope_type": "user",
    "scope_id": "user_123",
    "enhance_query": true,
    "top_k": 5
  }'
```

### 4. 推断可追溯性 | Inference Traceability

从事件提取的记忆会包含推断信息，便于审计：

- **inference_type**: 推断类型（fact/preference/pattern/rule）
- **inference_confidence**: 置信度（0.0-1.0）
- **inference_reasoning**: 推断理由

```json
{
  "id": "...",
  "content": "用户偏好深色主题",
  "inference_type": "preference",
  "inference_confidence": 0.85,
  "inference_reasoning": "用户在对话中明确表示喜欢深色主题"
}
```

### 5. 事件驱动的记忆创建 | Event-Driven Memory Creation

- **自动捕获**：基于应用事件自动创建记忆
- **上下文感知**：记录事件发生的时间和来源
- **智能分层**：根据事件类型自动分配到合适的记忆层

### 6. 多维度记忆检索 | Multi-Dimensional Memory Retrieval

```rust
// 综合评分算法
score = similarity_score * 0.35     // 语义相似度
      + importance * 0.25           // 重要性权重  
      + recency_score * 0.15        // 时间新近度
      + hit_count_score * 0.1       // 历史命中次数
      + fulltext_score * 0.15       // 全文匹配度
```

### 7. 智能生命周期管理 | Intelligent Lifecycle Management

- **自动过期**：基于 TTL 自动清理过期记忆
- **冷却机制**：长期未使用的记忆进入冷却状态
- **热度恢复**：被重新使用的记忆自动恢复活跃状态

### 8. 多模态 LLM 集成 | Multi-Modal LLM Integration

```rust
// 支持多种 LLM 提供商
pub enum LlmProviderType {
    OpenAI,    // GPT-4, GPT-3.5
    Azure,     // Azure OpenAI
    Local,     // Ollama, vLLM 等本地模型
}

// 自动内容处理
pub struct ProcessMemoryResult {
    pub processed_content: String,  // 压缩后的内容
    pub category: MemoryCategory,   // 自动分类
    pub tags: Vec<String>,          // 提取的标签
}
```

## 📊 使用场景 | Use Cases

### 1. 智能客服系统 | Intelligent Customer Service

```rust
// 记录用户偏好
CreateMemoryInput {
    layer: Layer::Task,
    scope_type: ScopeType::User,
    scope_id: "user_12345",
    scene: "customer_service.preference",
    content: "用户偏好通过邮件接收通知",
    event_source: Some("preference_setting"),
    process_with_llm: true,
}
```

### 2. 代码助手 | Code Assistant

```rust
// 记录编程习惯
CreateMemoryInput {
    layer: Layer::Session,
    scope_type: ScopeType::Project,
    scope_id: "project_abc",
    scene: "coding.style_preference", 
    content: "项目使用 TypeScript 严格模式",
    event_source: Some("code_analysis"),
}
```

### 3. 工作流自动化 | Workflow Automation

```rust
// 记录业务规则
CreateMemoryInput {
    layer: Layer::LongTerm,
    scope_type: ScopeType::Org,
    scope_id: "company_xyz",
    scene: "workflow.approval_process",
    content: "超过10万的合同需要CEO审批",
    event_source: Some("policy_update"),
}
```

## 🛠️ 技术栈 | Tech Stack

| 组件           | 技术选型     | 说明                       |
| -------------- | ------------ | -------------------------- |
| **后端框架**   | Rust + Axum  | 高性能异步 Web 框架        |
| **数据库**     | PostgreSQL   | 记忆元数据存储             |
| **向量数据库** | Qdrant       | 语义相似度检索             |
| **LLM 集成**   | async-openai | 支持 OpenAI/Azure/本地模型 |
| **嵌入模型**   | 多提供商支持 | OpenAI/本地嵌入模型        |
| **配置管理**   | YAML         | 灵活的配置系统             |

## 🚀 快速开始 | Quick Start

### 1. 环境准备 | Prerequisites

```bash
# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 启动依赖服务
docker-compose up -d postgres qdrant
```

### 2. 配置服务 | Configuration

```yaml
# config/config.yaml
server:
  host: "0.0.0.0"
  port: 8080

database:
  url: "postgres://memory:memory@localhost:5432/memory_db"

qdrant:
  url: "http://localhost:6334"

embedding:
  default_provider: "openai"

llm:
  default_provider: "openai"
```

### 3. 启动服务 | Start Service

```bash
cd memory-server
cargo run
```

### 4. 创建记忆 | Create Memory

```bash
# 方式一：直接创建（用户指定所有字段）
curl -X POST http://localhost:8080/api/v1/memories \
  -H "Content-Type: application/json" \
  -d '{
    "layer": "session",
    "scope_type": "user", 
    "scope_id": "user_123",
    "scene": "work.preference",
    "content": "用户偏好使用深色主题",
    "category": "preference",
    "tags": ["ui", "theme"],
    "importance": 0.8
  }'

# 方式二：从事件提取（LLM 自动处理）
curl -X POST http://localhost:8080/api/v1/memories/from-event \
  -H "Content-Type: application/json" \
  -d '{
    "content": "用户点击了深色主题按钮，并表示这样看起来更舒服",
    "context": "UI 设置页面",
    "mode": "auto",
    "scope_type": "user",
    "scope_id": "user_123",
    "scene": "ui.preference"
  }'
```

### 5. 检索记忆 | Retrieve Memory

```bash
# 基础检索
curl -X POST http://localhost:8080/api/v1/memories/retrieve \
  -H "Content-Type: application/json" \
  -d '{
    "query": "用户界面偏好",
    "scope_type": "user",
    "scope_id": "user_123", 
    "scene": "work.preference",
    "top_k": 5
  }'

# 启用查询增强（LLM 扩展语义）
curl -X POST http://localhost:8080/api/v1/memories/retrieve \
  -H "Content-Type: application/json" \
  -d '{
    "query": "用户界面偏好",
    "scope_type": "user",
    "scope_id": "user_123",
    "enhance_query": true,
    "top_k": 5
  }'
```

## 📖 API 文档 | API Documentation

启动服务后访问 Swagger UI：
```
http://localhost:8080/swagger-ui/
```

### 核心 API 端点 | Core API Endpoints

| 端点                           | 方法 | 说明                         |
| ------------------------------ | ---- | ---------------------------- |
| `/api/v1/memories`             | POST | 直接创建记忆（用户指定字段） |
| `/api/v1/memories/from-event`  | POST | 从事件提取记忆（LLM 处理）   |
| `/api/v1/memories/retrieve`    | POST | 检索记忆（支持查询增强）     |
| `/api/v1/memories/{id}`        | GET  | 获取记忆详情                 |
| `/api/v1/memories/{id}`        | PUT  | 更新记忆                     |
| `/api/v1/config/providers`     | POST | 配置嵌入提供商               |
| `/api/v1/config/llm-providers` | POST | 配置 LLM 提供商              |

## 🔧 高级配置 | Advanced Configuration

### 1. 多提供商配置 | Multi-Provider Setup

```bash
# 配置 OpenAI 嵌入提供商
curl -X POST http://localhost:8080/api/v1/config/providers \
  -H "Content-Type: application/json" \
  -d '{
    "name": "openai-embeddings",
    "provider_type": "openai",
    "endpoint": "https://api.openai.com/v1",
    "api_key": "sk-...",
    "model": "text-embedding-3-large"
  }'

# 配置本地 LLM 提供商  
curl -X POST http://localhost:8080/api/v1/config/llm-providers \
  -H "Content-Type: application/json" \
  -d '{
    "name": "local-llm",
    "provider_type": "local", 
    "endpoint": "http://localhost:11434",
    "model": "llama3.2"
  }'
```

### 2. 生命周期调优 | Lifecycle Tuning

```yaml
lifecycle:
  cooldown_check_interval_seconds: 3600
  cooldown_thresholds:
    session: 86400      # 1天后进入冷却
    task: 604800        # 7天后进入冷却  
    long_term: 2592000  # 30天后进入冷却
```

### 3. 检索权重调整 | Retrieval Weight Tuning

```yaml
retrieval:
  score_weights:
    similarity: 0.35    # 语义相似度权重
    importance: 0.25    # 重要性权重
    recency: 0.15       # 时间新近度权重
    hit_count: 0.1      # 命中次数权重
    fulltext: 0.15      # 全文匹配权重
```

## 🧪 测试 | Testing

```bash
# 运行所有测试
cargo test

# 运行特定模块测试
cargo test memory_processor

# 运行集成测试
cargo test --test integration
```

当前测试覆盖率：**171 个测试全部通过** ✅

## 📈 性能指标 | Performance Metrics

- **内存使用**：< 100MB (空载)
- **响应时间**：< 50ms (P95)
- **并发支持**：1000+ 连接
- **存储效率**：向量压缩比 > 80%

## 🤝 贡献指南 | Contributing

1. Fork 项目
2. 创建特性分支 (`git checkout -b feature/amazing-feature`)
3. 提交更改 (`git commit -m 'Add amazing feature'`)
4. 推送到分支 (`git push origin feature/amazing-feature`)
5. 创建 Pull Request

## 📄 许可证 | License

本项目采用 MIT 许可证 - 查看 [LICENSE](LICENSE) 文件了解详情。

## 🙏 致谢 | Acknowledgments

- [Qdrant](https://qdrant.tech/) - 高性能向量数据库
- [async-openai](https://github.com/64bit/async-openai) - Rust OpenAI 客户端
- [Axum](https://github.com/tokio-rs/axum) - 现代 Rust Web 框架

---

**Memory Framework - 让 AI 应用拥有可靠的记忆** 🧠✨