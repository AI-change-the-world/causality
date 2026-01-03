# Requirements Document

## Introduction

重构 Memory Server 架构，实现 Event-driven 的 Memory 管理系统。核心设计原则：
- 所有 Memory 都由 Event 触发创建或更新
- Memory 内容不可变，冲突时创建新版本
- 通过版本链追踪 Memory 演进历史
- 使用 LFU + 衰减机制管理 Memory 生命周期
- 检索完全依赖全文索引 + 向量相似度

## Glossary

- **Owner**: 记忆所有者的唯一标识符，由外部系统定义
- **Scope**: 记忆的作用域标识符，由用户自定义（如 project:123, session:abc）
- **Memory**: 结构化的记忆实体，存储提炼后的知识/偏好/规则，内容不可变
- **Event**: 原始事件记录，作为 Memory 的证据来源，完全不可变
- **Version Chain**: Memory 的版本演进链，通过 supersedes/superseded_by 关联
- **Global Memory**: 跨 Scope 有效的长期记忆
- **Decay Score**: 基于命中率和时间的衰减分数，用于淘汰决策

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         Event (不可变)                           │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ id: UUID                                                 │   │
│  │ owner_id: String                                         │   │
│  │ scope_id: Option<String>                                 │   │
│  │ content: Text                                            │   │
│  │ context: Option<Text>                                    │   │
│  │ summary: Option<Text>                                    │   │
│  │ processed: bool                                          │   │
│  │ event_time: DateTime                                     │   │
│  │ created_at: DateTime                                     │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                                │
                                │ 1:N (event_memory_relations)
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│                       Memory (内容不可变)                        │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ id: UUID                                                 │   │
│  │ owner_id: String                                         │   │
│  │ scope_id: Option<String>  (null = global memory)         │   │
│  │ content: Text                                            │   │
│  │ category: Option<String>  (层级式: work.code.eslint)     │   │
│  │ tags: Option<Vec<String>>                                │   │
│  │ importance: f32                                          │   │
│  │ confidence: f32                                          │   │
│  │                                                          │   │
│  │ // 版本链                                                 │   │
│  │ root_memory_id: Option<UUID>                             │   │
│  │ version_number: i32                                      │   │
│  │ is_current_version: bool                                 │   │
│  │ supersedes: Option<UUID>                                 │   │
│  │ superseded_by: Option<UUID>                              │   │
│  │                                                          │   │
│  │ // 生命周期                                               │   │
│  │ is_global: bool                                          │   │
│  │ hit_count: i64                                           │   │
│  │ last_hit_at: Option<DateTime>                            │   │
│  │ decay_score: f32                                         │   │
│  │                                                          │   │
│  │ status: Status                                           │   │
│  │ source_event_id: Option<UUID>                            │   │
│  │ created_at, updated_at                                   │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                   EventMemoryRelation                            │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ event_id: UUID                                           │   │
│  │ memory_id: UUID                                          │   │
│  │ relation_type: created_from | reinforced_by              │   │
│  │ created_at: DateTime                                     │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## Requirements

### Requirement 1: Event 存储

**User Story:** As a system, I want to store Events as immutable records, so that I have a complete audit trail of all inputs.

#### Acceptance Criteria

1. THE Event SHALL be completely immutable after creation
2. WHEN an Event is created, THE System SHALL accept owner_id and scope_id without validation
3. THE Event SHALL store original content and optional context
4. WHEN an Event is processed, THE System SHALL generate a summary
5. THE System SHALL NOT delete or modify Event records

### Requirement 2: Memory 创建与版本管理

**User Story:** As a system, I want to create Memories from Events with version tracking, so that I can trace the evolution of knowledge over time.

#### Acceptance Criteria

1. WHEN an Event is processed, THE System SHALL extract potential Memory using LLM
2. THE Memory content SHALL be immutable after creation
3. WHEN a new Memory is created, THE System SHALL set root_memory_id to its own id and version_number to 1
4. WHEN a Memory supersedes another, THE System SHALL:
   - Set new Memory's supersedes to old Memory's id
   - Set new Memory's root_memory_id to old Memory's root_memory_id
   - Set new Memory's version_number to old Memory's version_number + 1
   - Set old Memory's superseded_by to new Memory's id
   - Set old Memory's is_current_version to false
   - Set old Memory's status to Superseded
5. THE System SHALL create EventMemoryRelation with relation_type="created_from"

### Requirement 3: Memory 匹配与强化

**User Story:** As a system, I want to match extracted memories against existing ones, so that I can reinforce existing knowledge instead of creating duplicates.

#### Acceptance Criteria

1. WHEN a Memory is extracted from Event, THE System SHALL search for similar existing Memories within same owner_id
2. THE System SHALL use vector similarity for matching (threshold configurable, default 0.85)
3. WHEN matching Memory is found AND content is semantically consistent (LLM判断):
   - THE System SHALL NOT create new Memory
   - THE System SHALL update existing Memory's confidence (weighted average)
   - THE System SHALL increment hit_count
   - THE System SHALL create EventMemoryRelation with relation_type="reinforced_by"
4. WHEN matching Memory is found AND content conflicts (LLM判断):
   - THE System SHALL create new Memory version (supersedes old one)
   - THE System SHALL create EventMemoryRelation with relation_type="created_from"

### Requirement 4: 层级式 Category

**User Story:** As a system, I want to categorize Memories using hierarchical paths, so that I can organize and retrieve them efficiently.

#### Acceptance Criteria

1. THE category field SHALL support hierarchical paths (e.g., "work.code.eslint", "personal.food.chinese")
2. WHEN extracting Memory, THE LLM SHALL generate appropriate category path
3. THE System SHALL NOT validate category format (LLM自由生成)
4. WHEN retrieving, THE System SHALL support prefix matching on category
5. THE System SHALL support category-based aggregation queries

### Requirement 5: Global Memory Promotion

**User Story:** As a system, I want to promote frequently reinforced Memories to global status, so that cross-scope knowledge is preserved.

#### Acceptance Criteria

1. THE System SHALL evaluate Memories for global promotion based on:
   - Reinforcement count across different scopes (min_scope_diversity)
   - Confidence score (min_confidence)
   - Age (min_age_hours)
2. WHEN a Memory is promoted to global, THE System SHALL:
   - Set is_global = true
   - Set scope_id = null
3. THE Global Memory SHALL be retrievable from any scope of the same owner
4. THE System SHALL run promotion evaluation periodically (configurable interval)

### Requirement 6: LFU 淘汰机制

**User Story:** As a system, I want to automatically retire low-value Memories, so that storage is used efficiently.

#### Acceptance Criteria

1. THE System SHALL calculate decay_score based on: hit_count, last_hit_at, created_at
2. THE decay_score formula SHALL be: `hit_count / (1 + days_since_last_hit * decay_factor)`
3. WHEN Memory is not hit for extended period, THE System SHALL transition status to Cooldown
4. WHEN Memory in Cooldown is hit, THE System SHALL transition back to Active
5. WHEN decay_score drops below threshold, THE System SHALL mark status as Candidate
6. WHEN storage limit is reached, THE System SHALL archive Memories with lowest decay_score
7. THE System SHALL NOT delete Memories, only archive them

### Requirement 7: 版本链查询

**User Story:** As a developer, I want to query the complete version history of a Memory, so that I can understand how knowledge evolved.

#### Acceptance Criteria

1. THE System SHALL support querying all versions of a Memory by root_memory_id
2. THE System SHALL return versions ordered by version_number
3. THE System SHALL include source Event information for each version
4. THE query SHALL be O(1) using root_memory_id index (no recursive CTE needed)

### Requirement 8: 检索

**User Story:** As a user, I want to retrieve relevant Memories using semantic search, so that I can find information efficiently.

#### Acceptance Criteria

1. THE System SHALL support vector similarity search on Memory content
2. THE System SHALL support full-text search on Memory content
3. THE System SHALL support hybrid search (vector + full-text)
4. THE default retrieval SHALL only return is_current_version = true Memories
5. THE System SHALL support filtering by owner_id, scope_id, category prefix
6. THE System SHALL support optional include_evidence parameter to return source Events
7. WHEN scope_id is provided, THE System SHALL also include global Memories (is_global = true)

### Requirement 9: API

**User Story:** As a developer, I want clear APIs to interact with the Memory system.

#### Acceptance Criteria

1. THE System SHALL provide Event creation endpoint
2. THE System SHALL provide Memory retrieval endpoint with search parameters
3. THE System SHALL provide Memory version history endpoint
4. THE System SHALL provide Memory statistics endpoint (hit_count, decay_score, etc.)
5. THE System SHALL provide manual promotion endpoint for administrators

## Deleted Features (Compared to Previous Design)

以下功能已从设计中移除以简化系统：

- ❌ Scope 独立实体表 - 改用 scope_id 字符串
- ❌ ScopeType 枚举 - 不需要预定义类型
- ❌ Layer 枚举 (session/task/long_term) - 改用 is_global + scope_id
- ❌ Scene / SceneDefinition - 合并到 category
- ❌ TTL / expires_at - 改用 LFU 淘汰
- ❌ contradicted_by 关系类型 - 冲突时直接创建新版本
- ❌ 三种更新策略 (append/merge/supersede) - 内容不可变，只有版本替换
