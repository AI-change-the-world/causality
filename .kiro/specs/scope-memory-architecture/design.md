# Design Document: Scope-Memory Architecture Refactoring

## Overview

本设计文档描述 Memory Server 的架构重构，核心设计原则：

1. **Event-driven**: 所有 Memory 都由 Event 触发创建或更新
2. **Immutable Content**: Memory 内容不可变，冲突时创建新版本
3. **Version Chain**: 通过物化路径追踪 Memory 演进历史，O(1) 查询
4. **LFU Eviction**: 使用命中率 + 时间衰减管理 Memory 生命周期
5. **Semantic Search**: 检索完全依赖全文索引 + 向量相似度

## Architecture

### Data Model

```rust
// Memory - 核心记忆实体 (内容不可变)
struct Memory {
    id: Uuid,
    owner_id: String,              // 用户定义，不校验语义
    scope_id: Option<String>,      // 用户定义，null = global 记忆
    content: String,               // 记忆内容 (不可变)
    category: Option<String>,      // 层级式分类 (work.code.eslint)
    tags: Option<Vec<String>>,     // 标签
    importance: f32,               // 重要性 0.0-1.0
    confidence: f32,               // 置信度 0.0-1.0
    
    // 版本链 (物化路径方案)
    root_memory_id: Option<Uuid>,  // 版本链的根节点 (第一个版本时为 null 或自身 id)
    version_number: i32,           // 版本号，从 1 开始
    is_current_version: bool,      // 是否为当前版本
    supersedes: Option<Uuid>,      // 取代了哪个 Memory
    superseded_by: Option<Uuid>,   // 被哪个 Memory 取代
    
    // 生命周期
    is_global: bool,               // 是否为全局/长期记忆
    hit_count: i64,                // 命中次数
    last_hit_at: Option<DateTime>, // 最后命中时间
    decay_score: f32,              // 衰减分数 (计算字段)
    
    source_event_id: Option<Uuid>, // 来源 Event
    status: Status,                // Active | Cooldown | Candidate | Superseded | Archived
    embedding_status: EmbeddingStatus,
    created_at: DateTime,
    updated_at: DateTime,
}

// 注意: embedding 向量存储在 Qdrant 中，不在 PostgreSQL

// Event - 原始事件 (完全不可变)
struct Event {
    id: Uuid,
    owner_id: String,
    scope_id: Option<String>,
    content: String,
    context: Option<String>,
    summary: Option<String>,
    source: Option<String>,        // "user_created" | "api" | etc.
    processed: bool,
    event_time: DateTime,
    created_at: DateTime,
}

// EventMemoryRelation - 事件-记忆关系 (不可变)
struct EventMemoryRelation {
    id: Uuid,
    event_id: Uuid,
    memory_id: Uuid,
    relation_type: RelationType,   // CreatedFrom | ReinforcedBy
    created_at: DateTime,
}

// Status 枚举
enum Status {
    Active,      // 活跃状态
    Cooldown,    // 冷却状态 (长时间未命中)
    Candidate,   // 待淘汰候选
    Superseded,  // 被新版本取代
    Archived,    // 已归档
}

// RelationType 枚举
enum RelationType {
    CreatedFrom,   // Memory 从此 Event 创建
    ReinforcedBy,  // Memory 被此 Event 强化
}
```

### Database Schema

```sql
-- memories 表
CREATE TABLE memories (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id VARCHAR(255) NOT NULL,
    scope_id VARCHAR(255),  -- null = global memory
    content TEXT NOT NULL,
    category VARCHAR(500),
    tags TEXT[],
    importance REAL NOT NULL DEFAULT 0.5,
    confidence REAL NOT NULL DEFAULT 1.0,
    
    -- 版本链
    root_memory_id UUID,  -- 指向版本链的根节点
    version_number INTEGER NOT NULL DEFAULT 1,
    is_current_version BOOLEAN NOT NULL DEFAULT true,
    supersedes UUID REFERENCES memories(id),
    superseded_by UUID REFERENCES memories(id),
    
    -- 生命周期
    is_global BOOLEAN NOT NULL DEFAULT false,
    hit_count BIGINT NOT NULL DEFAULT 0,
    last_hit_at TIMESTAMP WITH TIME ZONE,
    decay_score REAL NOT NULL DEFAULT 1.0,
    
    source_event_id UUID REFERENCES events(id),
    status VARCHAR(20) NOT NULL DEFAULT 'active',
    embedding_status VARCHAR(20) NOT NULL DEFAULT 'pending',
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- 版本链查询索引 (核心优化)
CREATE INDEX idx_memories_root ON memories(root_memory_id);
CREATE INDEX idx_memories_current ON memories(root_memory_id, is_current_version) 
    WHERE is_current_version = true;

-- 检索索引
CREATE INDEX idx_memories_owner ON memories(owner_id);
CREATE INDEX idx_memories_owner_scope ON memories(owner_id, scope_id);
CREATE INDEX idx_memories_status ON memories(status);
CREATE INDEX idx_memories_global ON memories(owner_id, is_global) WHERE is_global = true;
CREATE INDEX idx_memories_category ON memories USING btree(category);

-- 全文搜索索引
CREATE INDEX idx_memories_content_fts ON memories USING gin(to_tsvector('english', content));

-- events 表
CREATE TABLE events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id VARCHAR(255) NOT NULL,
    scope_id VARCHAR(255),
    content TEXT NOT NULL,
    context TEXT,
    summary TEXT,
    source VARCHAR(50),
    processed BOOLEAN NOT NULL DEFAULT false,
    event_time TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_events_owner ON events(owner_id);
CREATE INDEX idx_events_processed ON events(processed) WHERE processed = false;

-- event_memory_relations 表
CREATE TABLE event_memory_relations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id UUID NOT NULL REFERENCES events(id),
    memory_id UUID NOT NULL REFERENCES memories(id),
    relation_type VARCHAR(20) NOT NULL,  -- 'created_from' | 'reinforced_by'
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    
    UNIQUE(event_id, memory_id)
);

CREATE INDEX idx_emr_event ON event_memory_relations(event_id);
CREATE INDEX idx_emr_memory ON event_memory_relations(memory_id);
```

### Category 检索逻辑

```rust
// Category 查询类型
enum CategoryQuery {
    Exact(String),           // category = 'work.code.eslint'
    Prefix(String),          // category LIKE 'work.code.%'
    Contains(String),        // category LIKE '%eslint%' (需要 trigram 索引)
}

impl CategoryQuery {
    fn to_sql(&self) -> (String, String) {
        match self {
            CategoryQuery::Exact(cat) => ("category = $1".to_string(), cat.clone()),
            CategoryQuery::Prefix(prefix) => {
                let pattern = if prefix.ends_with('.') {
                    format!("{}%", prefix)
                } else {
                    format!("{}.%", prefix)
                };
                ("category LIKE $1 OR category = $2".to_string(), pattern)
            }
            CategoryQuery::Contains(term) => {
                (format!("category LIKE '%{}%'", term), term.clone())
            }
        }
    }
}
```

## Memory Version Chain (核心设计)

### 版本演进流程图

```
时间线 ──────────────────────────────────────────────────────────────────────►

Event_1 (t1)                    Event_2 (t2)                    Event_3 (t3)
"用户说喜欢深色主题"              "用户说更喜欢浅色主题"            "用户确认用浅色主题"
     │                              │                              │
     ▼                              ▼                              ▼
┌─────────────────┐          ┌─────────────────┐          ┌─────────────────┐
│ Memory_v1       │ supersede│ Memory_v2       │ reinforce│ Memory_v2       │
│ id: A           │ ───────► │ id: B           │ ◄─────── │ confidence ↑    │
│ content: 深色   │          │ content: 浅色   │          │ hit_count ↑     │
│ root: A         │          │ root: A         │          │                 │
│ version: 1      │          │ version: 2      │          │                 │
│ is_current: ✗   │          │ is_current: ✓   │          │                 │
│ superseded_by: B│          │ supersedes: A   │          │                 │
│ status: Superseded         │ status: Active  │          │                 │
└─────────────────┘          └─────────────────┘          └─────────────────┘
         │                           ▲
         │      superseded_by        │
         └───────────────────────────┘
```

### 版本链数据结构

```
Memory_v1 (根节点):
  id: "aaa-111"
  root_memory_id: "aaa-111"  (指向自己)
  version_number: 1
  is_current_version: false
  supersedes: null
  superseded_by: "bbb-222"
  status: Superseded

Memory_v2:
  id: "bbb-222"
  root_memory_id: "aaa-111"  (指向根节点)
  version_number: 2
  is_current_version: true
  supersedes: "aaa-111"
  superseded_by: null
  status: Active

Memory_v3 (如果再次冲突):
  id: "ccc-333"
  root_memory_id: "aaa-111"  (仍然指向根节点)
  version_number: 3
  is_current_version: true
  supersedes: "bbb-222"
  superseded_by: null
  status: Active
```

### 版本链查询 (O(1) 复杂度)

```sql
-- 1. 获取当前活跃版本 (最常用)
SELECT * FROM memories 
WHERE owner_id = $owner_id 
  AND is_current_version = true
  AND status = 'active';

-- 2. 获取某个 Memory 的完整版本历史 (无需递归!)
SELECT * FROM memories 
WHERE root_memory_id = $root_id 
ORDER BY version_number;

-- 3. 获取版本历史 + 关联的 Events
SELECT 
    m.*,
    e.content as source_event_content,
    e.event_time as source_event_time,
    emr.relation_type
FROM memories m
LEFT JOIN event_memory_relations emr 
    ON m.id = emr.memory_id AND emr.relation_type = 'created_from'
LEFT JOIN events e ON emr.event_id = e.id
WHERE m.root_memory_id = $root_id
ORDER BY m.version_number;

-- 4. 查找被取代的旧版本
SELECT * FROM memories 
WHERE superseded_by = $current_memory_id;
```

### 创建新版本的逻辑

```rust
async fn create_superseding_memory(
    &self,
    old_memory: &Memory,
    new_content: String,
    source_event: &Event,
    extracted: &ExtractedMemory,
) -> Result<Memory> {
    // 1. 确定 root_memory_id
    let root_id = old_memory.root_memory_id.unwrap_or(old_memory.id);
    let new_version = old_memory.version_number + 1;
    
    // 2. 创建新版本 Memory
    let new_memory = Memory {
        id: Uuid::new_v4(),
        owner_id: old_memory.owner_id.clone(),
        scope_id: old_memory.scope_id.clone(),
        content: new_content,
        category: extracted.category.clone(),
        tags: extracted.tags.clone(),
        importance: extracted.importance,
        confidence: extracted.confidence,
        
        // 版本链
        root_memory_id: Some(root_id),
        version_number: new_version,
        is_current_version: true,
        supersedes: Some(old_memory.id),
        superseded_by: None,
        
        // 生命周期
        is_global: old_memory.is_global,  // 继承 global 状态
        hit_count: 0,
        last_hit_at: None,
        decay_score: 1.0,
        
        source_event_id: Some(source_event.id),
        status: Status::Active,
        // ...
    };
    
    // 3. 更新旧版本 (原子操作)
    sqlx::query!(
        r#"
        UPDATE memories SET
            superseded_by = $1,
            is_current_version = false,
            status = 'superseded',
            updated_at = NOW()
        WHERE id = $2
        "#,
        new_memory.id,
        old_memory.id
    ).execute(&self.pool).await?;
    
    // 4. 插入新版本
    self.insert_memory(&new_memory).await?;
    
    // 5. 创建关系记录
    self.create_relation(source_event.id, new_memory.id, RelationType::CreatedFrom).await?;
    
    Ok(new_memory)
}
```

## Data Immutability Rules

| 实体                | 创建 | 修改         | 删除         |
| ------------------- | ---- | ------------ | ------------ |
| Event               | ✅    | ❌ 完全不可变 | ❌            |
| Memory              | ✅    | ⚠️ 仅限元数据 | ❌ 只标记状态 |
| EventMemoryRelation | ✅    | ❌            | ❌            |

### Memory 可修改的字段 (元数据)

```rust
// 这些字段可以更新，不影响语义
struct MemoryMutableFields {
    confidence: f32,        // 被强化时增加
    hit_count: i64,         // 被检索时 +1
    last_hit_at: DateTime,  // 被检索时更新
    decay_score: f32,       // 后台定期计算
    status: Status,         // 状态流转
    superseded_by: Uuid,    // 被取代时设置
    is_current_version: bool, // 被取代时设为 false
    is_global: bool,        // 提升为全局时设为 true
    updated_at: DateTime,   // 任何更新时刷新
}
```

### Memory 不可修改的字段 (核心内容)

```rust
// 这些字段创建后不可变，冲突时创建新版本
struct MemoryImmutableFields {
    id: Uuid,
    owner_id: String,
    scope_id: Option<String>,  // 提升为 global 时变为 null，但这是特殊情况
    content: String,           // 核心内容不可变!
    category: Option<String>,
    tags: Option<Vec<String>>,
    importance: f32,
    root_memory_id: Option<Uuid>,
    version_number: i32,
    supersedes: Option<Uuid>,
    source_event_id: Option<Uuid>,
    created_at: DateTime,
}
```

## Components and Interfaces

### 1. EventProcessor

负责处理 Event 并提取 Memory。

```rust
trait EventProcessor {
    /// 处理单个 Event，提取 Memory
    async fn process_event(&self, event: &Event) -> Result<Vec<ExtractedMemory>>;
    
    /// 批量处理 Events
    async fn process_batch(&self, events: Vec<Event>) -> Result<ProcessBatchResult>;
}

struct ExtractedMemory {
    content: String,
    category: Option<String>,
    tags: Option<Vec<String>>,
    importance: f32,
    confidence: f32,
}
```

### 2. MemoryMatcher

负责匹配新提取的 Memory 与现有 Memory。

```rust
trait MemoryMatcher {
    /// 查找相似的现有 Memory (只搜索 is_current_version = true)
    async fn find_similar(
        &self,
        owner_id: &str,
        scope_id: Option<&str>,
        content: &str,
        embedding: &[f32],
        threshold: f32,  // 默认 0.85
    ) -> Result<Vec<MatchResult>>;
}

struct MatchResult {
    memory: Memory,
    similarity_score: f32,
}
```

### 3. MemoryReconciler

负责决定如何处理匹配结果。

```rust
trait MemoryReconciler {
    /// 协调新提取的 Memory 与现有 Memory
    async fn reconcile(
        &self,
        event: &Event,
        extracted: &ExtractedMemory,
        matches: Vec<MatchResult>,
    ) -> Result<ReconcileOutcome>;
}

enum ReconcileOutcome {
    /// 无匹配，创建新 Memory
    CreateNew {
        memory: Memory,
        relation: EventMemoryRelation,
    },
    /// 有匹配且内容一致，强化现有 Memory
    Reinforce {
        memory_id: Uuid,
        confidence_delta: f32,
        relation: EventMemoryRelation,
    },
    /// 有匹配但内容冲突，创建新版本
    Supersede {
        new_memory: Memory,
        superseded_id: Uuid,
        relation: EventMemoryRelation,
    },
}
```

### 4. DecayCalculator

负责计算 Memory 的衰减分数。

```rust
trait DecayCalculator {
    /// 计算单个 Memory 的衰减分数
    fn calculate(&self, memory: &Memory, config: &DecayConfig) -> f32;
    
    /// 批量更新衰减分数
    async fn update_batch(&self, owner_id: &str) -> Result<usize>;
}

struct DecayConfig {
    decay_half_life_days: f32,  // 衰减半衰期 (默认 7 天)
    hit_boost_factor: f32,      // 命中加成因子 (默认 0.1)
    global_boost: f32,          // global memory 加成 (默认 2.0)
}

// 衰减分数计算公式
// decay_score = (1 + log(hit_count + 1) * hit_boost_factor) 
//             * exp(-days_since_last_activity / decay_half_life_days)
//             * (is_global ? global_boost : 1.0)
```

### 5. GlobalPromoter

负责将跨 Scope 强化的 Memory 提升为 Global Memory。

```rust
trait GlobalPromoter {
    /// 检查 Memory 是否满足提升条件
    async fn check_eligibility(&self, memory_id: Uuid) -> Result<PromotionCheck>;
    
    /// 执行提升
    async fn promote(&self, memory_id: Uuid, reason: &str) -> Result<Memory>;
}

struct PromotionCheck {
    eligible: bool,
    scope_count: i32,           // 被多少个不同 scope 强化
    total_reinforcements: i32,  // 总强化次数
    min_confidence: f32,        // 最低置信度要求
    reason: Option<String>,
}

struct PromotionCriteria {
    min_scope_diversity: i32,   // 最少跨越的 scope 数量 (默认 2)
    min_reinforcements: i32,    // 最少强化次数 (默认 3)
    min_confidence: f32,        // 最低置信度 (默认 0.7)
    min_age_hours: i64,         // 最小年龄 (默认 24 小时)
}
```

### 6. EvictionManager

负责管理 Memory 的淘汰。

```rust
trait EvictionManager {
    /// 执行淘汰检查
    async fn run_eviction(&self, owner_id: &str, config: &EvictionConfig) -> Result<EvictionResult>;
    
    /// 获取淘汰候选
    async fn get_candidates(&self, owner_id: &str, limit: usize) -> Result<Vec<Memory>>;
}

struct EvictionConfig {
    cooldown_threshold_days: i32,   // 进入 Cooldown 的阈值 (默认 14 天)
    candidate_threshold_days: i32,  // 进入 Candidate 的阈值 (默认 30 天)
    archive_threshold_days: i32,    // 归档的阈值 (默认 90 天)
    max_memories_per_owner: i64,    // 每个 owner 的最大 Memory 数量
    decay_score_threshold: f32,     // 衰减分数阈值 (默认 0.1)
}

struct EvictionResult {
    cooldown_count: usize,
    candidate_count: usize,
    archived_count: usize,
}
```

## Data Models

### Complete Event Processing Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Event 处理完整流程                                   │
└─────────────────────────────────────────────────────────────────────────────┘

1. Event 进入系统
   │
   ▼
2. 存储 Event (不可变)
   │
   ▼
3. EventProcessor.process_event()
   │  - LLM 提取 Memory 候选
   │  - 生成 content, category, tags, importance, confidence
   │
   ▼
4. 生成 embedding 向量
   │
   ▼
5. MemoryMatcher.find_similar()
   │  - 搜索范围: owner_id 相同 AND (scope_id 相同 OR is_global = true)
   │  - 只搜索 is_current_version = true 的 Memory
   │  - 阈值: similarity >= 0.85
   │
   ├─► 情况 A: 无匹配 (similarity < 0.85)
   │   │
   │   └─► MemoryReconciler → CreateNew
   │       - 创建新 Memory
   │       - root_memory_id = 自身 id
   │       - version_number = 1
   │       - is_current_version = true
   │       - relation: (Event, Memory, "created_from")
   │
   ├─► 情况 B: 有匹配 + LLM 判断内容一致
   │   │
   │   └─► MemoryReconciler → Reinforce
   │       - 更新 confidence (加权平均)
   │       - 更新 hit_count += 1
   │       - 更新 last_hit_at = now
   │       - relation: (Event, Memory, "reinforced_by")
   │       - Memory 内容不变!
   │
   └─► 情况 C: 有匹配 + LLM 判断内容冲突
       │
       └─► MemoryReconciler → Supersede
           - 创建新 Memory (新版本)
           - 新 Memory.root_memory_id = 旧 Memory.root_memory_id
           - 新 Memory.version_number = 旧 Memory.version_number + 1
           - 新 Memory.supersedes = 旧 Memory.id
           - 新 Memory.is_current_version = true
           - 旧 Memory.superseded_by = 新 Memory.id
           - 旧 Memory.is_current_version = false
           - 旧 Memory.status = Superseded
           - relation: (Event, 新Memory, "created_from")
   │
   ▼
6. GlobalPromoter.check_eligibility() (异步)
   │  - 检查是否满足提升条件
   │
   ├─► 不满足条件 → 结束
   │
   └─► 满足条件 → GlobalPromoter.promote()
       - is_global = true
       - scope_id = null
```

### Memory Lifecycle State Machine

```
                                    ┌─────────────┐
                                    │   Created   │
                                    └──────┬──────┘
                                           │
                                           ▼
                              ┌────────────────────────┐
                              │        Active          │
                              │  (is_current_version)  │
                              └────────────┬───────────┘
                                           │
                    ┌──────────────────────┼──────────────────────┐
                    │                      │                      │
                    ▼                      ▼                      ▼
           ┌───────────────┐      ┌───────────────┐      ┌───────────────┐
           │   Cooldown    │      │  Superseded   │      │   Promoted    │
           │ (未命中 14天) │      │  (被新版本    │      │ (is_global    │
           │               │      │   取代)       │      │   = true)     │
           └───────┬───────┘      └───────────────┘      └───────────────┘
                   │                                              │
        ┌──────────┴──────────┐                                   │
        │                     │                                   │
        ▼                     ▼                                   │
┌───────────────┐    ┌───────────────┐                           │
│    Active     │    │   Candidate   │                           │
│  (再次命中)   │    │ (未命中 30天) │                           │
└───────────────┘    └───────┬───────┘                           │
                             │                                    │
                             ▼                                    │
                    ┌───────────────┐                            │
                    │   Archived    │ ◄──────────────────────────┘
                    │ (未命中 90天  │   (global memory 也可能被归档)
                    │  或容量超限)  │
                    └───────────────┘
```

### Retrieval Flow

```
Query 进入 (owner_id, scope_id?, query_text, filters?)
    │
    ▼
生成 query embedding
    │
    ▼
确定搜索范围:
├─ scope_id 有值: WHERE (scope_id = $scope OR is_global = true)
└─ scope_id 为空: WHERE is_global = true (仅全局记忆)
    │
    ▼
基础过滤:
├─ owner_id = $owner_id
├─ is_current_version = true (默认)
└─ status NOT IN ('superseded', 'archived') (默认)
    │
    ▼
执行混合搜索:
├─ Vector similarity search (Qdrant)
└─ Full-text search (PostgreSQL)
    │
    ▼
合并结果，计算 combined_score:
  combined_score = α * vector_score + (1-α) * text_score
  (α 默认 0.7)
    │
    ▼
应用额外过滤器:
├─ category prefix (可选)
├─ tags contains (可选)
└─ min_confidence (可选)
    │
    ▼
按 combined_score DESC 排序
    │
    ▼
可选: include_evidence = true 时加载关联 Events
    │
    ▼
可选: include_history = true 时加载版本历史
    │
    ▼
返回结果
```

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system—essentially, a formal statement about what the system should do.*

### Property 1: Event Immutability
*For any* Event after creation, all fields SHALL remain unchanged for the lifetime of the record.
**Validates: Requirements 1.1, 1.5**

### Property 2: Memory Content Immutability
*For any* Memory after creation, the content, category, tags, importance, root_memory_id, version_number, supersedes, and source_event_id fields SHALL remain unchanged.
**Validates: Requirements 2.2**

### Property 3: Version Chain Integrity
*For any* Memory M with supersedes = S:
- M.root_memory_id = S.root_memory_id (or S.id if S is root)
- M.version_number = S.version_number + 1
- S.superseded_by = M.id
- S.is_current_version = false
- S.status = Superseded
**Validates: Requirements 2.4**

### Property 4: Single Current Version
*For any* root_memory_id R, there SHALL be exactly one Memory where root_memory_id = R AND is_current_version = true.
**Validates: Requirements 2.4, 7.1**

### Property 5: Version Chain Query O(1)
*For any* version history query by root_memory_id, the query SHALL NOT use recursive CTE and SHALL complete in O(1) index lookups.
**Validates: Requirements 7.4**

### Property 6: Reinforcement Preserves Content
*For any* Reinforce operation, the Memory's content, category, tags, and importance fields SHALL remain unchanged.
**Validates: Requirements 3.3**

### Property 7: Supersede Creates New Version
*For any* Supersede operation, a new Memory record SHALL be created (not updated in place).
**Validates: Requirements 3.4**

### Property 8: Global Memory Scope
*For any* Memory with is_global = true, scope_id SHALL be null.
**Validates: Requirements 5.2**

### Property 9: Decay Score Monotonicity
*For any* Memory M, if hit_count increases, decay_score SHALL increase (all else equal).
**Validates: Requirements 6.1, 6.2**

### Property 10: Status Transition Validity
*For any* Memory, status transitions SHALL only follow:
- Active → Cooldown (no hit for cooldown_threshold)
- Active → Superseded (new version created)
- Cooldown → Active (hit received)
- Cooldown → Candidate (no hit for candidate_threshold)
- Candidate → Archived (no hit for archive_threshold OR capacity limit)
**Validates: Requirements 6.3, 6.4, 6.5, 6.6**

### Property 11: Retrieval Excludes Non-Current
*For any* default retrieval query, results SHALL NOT include Memories where is_current_version = false OR status IN ('superseded', 'archived').
**Validates: Requirements 8.4**

### Property 12: Scope + Global Retrieval
*For any* retrieval with scope_id = S, results SHALL include Memories where (scope_id = S OR is_global = true).
**Validates: Requirements 8.7**

### Property 13: EventMemoryRelation Completeness
*For any* Memory M, there SHALL exist at least one EventMemoryRelation with memory_id = M.id and relation_type = 'created_from'.
**Validates: Requirements 2.5**

## Error Handling

| Error Case         | Handling Strategy                                |
| ------------------ | ------------------------------------------------ |
| owner_id 格式无效  | 返回 400 Bad Request，说明格式要求 (长度 <= 255) |
| scope_id 格式无效  | 返回 400 Bad Request，说明格式要求 (长度 <= 255) |
| LLM 提取失败       | 标记 Event.processed = false，记录错误，支持重试 |
| Embedding 生成失败 | 标记 embedding_status = Failed，降级为纯文本搜索 |
| 匹配过程超时       | 返回部分结果，记录警告                           |
| 版本链创建失败     | 回滚事务，保持旧版本不变                         |
| 容量超限           | 触发异步归档任务，不阻塞写入                     |
| 提升冲突           | 记录冲突，人工审核                               |

## Testing Strategy

### Unit Tests
- DecayCalculator: 验证衰减公式计算正确性
- MemoryReconciler: 验证各种匹配场景的处理逻辑
- Category 前缀匹配逻辑
- Version chain 创建逻辑

### Property-Based Tests
使用 `proptest` 库：

```rust
// Property 3: Version Chain Integrity
proptest! {
    #[test]
    fn test_version_chain_integrity(
        old_memory in arbitrary_memory(),
        new_content in ".*",
    ) {
        let new_memory = create_superseding_memory(&old_memory, new_content);
        
        // 验证版本链完整性
        prop_assert_eq!(new_memory.root_memory_id, old_memory.root_memory_id.or(Some(old_memory.id)));
        prop_assert_eq!(new_memory.version_number, old_memory.version_number + 1);
        prop_assert_eq!(new_memory.supersedes, Some(old_memory.id));
        prop_assert!(new_memory.is_current_version);
    }
}

// Property 4: Single Current Version
proptest! {
    #[test]
    fn test_single_current_version(
        memories in vec(arbitrary_memory_in_chain(), 1..10),
    ) {
        let root_id = memories[0].root_memory_id.unwrap_or(memories[0].id);
        let current_count = memories.iter()
            .filter(|m| m.root_memory_id == Some(root_id) && m.is_current_version)
            .count();
        
        prop_assert_eq!(current_count, 1);
    }
}

// Property 9: Decay Score Monotonicity
proptest! {
    #[test]
    fn test_decay_score_increases_with_hits(
        memory in arbitrary_memory(),
        additional_hits in 1..100i64,
    ) {
        let config = DecayConfig::default();
        let score_before = calculate_decay_score(&memory, &config);
        
        let mut memory_after = memory.clone();
        memory_after.hit_count += additional_hits;
        let score_after = calculate_decay_score(&memory_after, &config);
        
        prop_assert!(score_after > score_before);
    }
}
```

### Integration Tests
- Event → Memory 提取完整流程
- 版本链创建和查询
- 跨 Scope 强化 → Global 提升流程
- 检索 + 版本过滤流程
- LFU 淘汰流程

## API Endpoints

### Event API

```
POST /api/v1/events
  - 创建新 Event，触发 Memory 提取流程
  - Body: { owner_id, scope_id?, content, context?, source? }
  - Response: { event_id, processed, memories_created, memories_reinforced }

GET /api/v1/events/{event_id}
  - 获取 Event 详情
  - Response: { event, related_memories }
```

### Memory API

```
GET /api/v1/memories/search
  - 语义搜索 Memory
  - Query: owner_id, scope_id?, query, category_prefix?, min_confidence?, limit?
  - Query: include_evidence?, include_history?
  - Response: { memories: [...], total }

GET /api/v1/memories/{memory_id}
  - 获取 Memory 详情
  - Query: include_evidence?, include_history?
  - Response: { memory, evidence?, history? }

GET /api/v1/memories/{memory_id}/history
  - 获取 Memory 版本历史
  - Response: { versions: [...], current_version }

POST /api/v1/memories/{memory_id}/promote
  - 手动提升为 Global Memory
  - Body: { reason }
  - Response: { memory }

GET /api/v1/memories/stats
  - 获取 Memory 统计信息
  - Query: owner_id
  - Response: { total, by_status, by_scope, avg_decay_score }
```

### Admin API

```
POST /api/v1/admin/eviction
  - 手动触发淘汰检查
  - Body: { owner_id?, dry_run? }
  - Response: { cooldown_count, candidate_count, archived_count }

POST /api/v1/admin/decay-update
  - 手动更新衰减分数
  - Body: { owner_id? }
  - Response: { updated_count }
```
