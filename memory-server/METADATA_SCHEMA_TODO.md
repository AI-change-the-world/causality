# Metadata Schema Driven Memory TODO

## 背景

当前 memory server 的结构化能力分散在三处：

- `system_profiles` 提供文本化业务描述
- `memories.category/tags` 提供弱结构化分类
- `structured_events` 提供固定六要素事件解析

这套设计在早期可用，但已经暴露出几个问题：

1. profile 只描述“系统是什么”，没有定义“记忆应该如何结构化”
2. memory 只有 `category/tags`，无法支撑业务专属 metadata 检索和覆盖判断
3. `structured_events` 采用固定列式 schema，不适合多业务系统扩展
4. supersede / lineage / query planning 仍然过度依赖 embedding 和自然语言内容

因此需要将系统升级为：

> `profile schema -> event analysis payload -> memory metadata -> schema-aware retrieve/reconcile`

核心思路是：

- profile 初始化时，由 LLM 产出一份结构化 `metadata_schema`
- 业务侧确认并冻结 schema version
- memory 写入和检索围绕 `metadata JSONB` 展开
- 事件级分析保留，但改为 `analysis_payload JSONB`，不再保留固定六要素表

---

## 核心设计原则

### 1. Profile 不只是 namespace，而是语义契约

`profile_id` 仍然表示业务系统命名空间。

但初始化完成后，profile 还必须携带一份稳定的结构化语义契约：

- 有哪些 metadata 字段
- 字段类型是什么
- 哪些字段可过滤
- 哪些字段参与 conflict candidate selection
- 哪些字段参与 lineage grouping
- 检索默认策略是什么

也就是说：

> profile 定义的不只是“系统边界”，还定义“当前系统如何理解 memory metadata”。

---

### 2. Profile schema 必须是 proposal -> confirm -> version

不能直接让 LLM 生成一份 schema 后立即长期生效。

应分成两个阶段：

1. `initialize profile`
   返回 `metadata_schema_proposal`
2. `confirm profile schema`
   将 proposal 冻结为 `schema_version = 1`

后续如需调整 schema，应升级版本，而不是直接覆盖旧版本。

否则不同时间写入的 memories 会使用不同语义结构，导致检索、覆盖和 lineage 判断失稳。

---

### 3. Memory metadata 采用 JSONB，而不是继续堆固定字段

当前 `memories.category` 和 `memories.tags` 应合并为：

- `metadata JSONB`

业务字段由当前 profile schema 决定，可多可少，不做全局硬编码。

例如房产系统可能定义：

```json
{
  "memory_type": "house_preference",
  "region": "浦东",
  "budget_range": "500w-700w",
  "layout": "3br",
  "school_priority": true,
  "time_scope": "current",
  "polarity": "prefer"
}
```

而另一个系统可以完全不是这套字段。

---

### 4. Event 分析结果也应采用 JSONB

当前 `structured_events` 表以固定列承载六要素：

- `time_element`
- `location_element`
- `actor_element`
- `cause_element`
- `process_element`
- `result_element`
- `background_element`
- `details_element`

这张表对“房产对话 / 电商行为 / CRM 跟进 / 工单处理”这种多系统形态不够通用。

需要保留“事件级结构化分析”这个能力，但不保留当前固定列式表。

建议升级为：

- `events.analysis_payload JSONB`
- `events.analysis_version`
- `events.analysis_schema_version`

注意：

- event analysis payload 和 memory metadata 不是一回事
- 一个 event 可能产出多条 memory
- event payload 是事件理解层
- memory metadata 是记忆语义层

---

## 数据模型目标

### 5. SystemProfile 增加 schema 能力

建议为 `system_profiles` 增加：

- `metadata_schema JSONB`
- `schema_status VARCHAR`
  - `draft`
  - `confirmed`
- `schema_version INTEGER`
- `schema_confirmed_at TIMESTAMPTZ`
- `schema_generation_prompt TEXT`
  - 可选，用于追踪 schema proposal 生成依据

`metadata_schema` 建议结构：

```json
{
  "version": 1,
  "entity_types": {
    "house_preference": {
      "fields": {
        "region": { "type": "string", "filterable": true },
        "budget_range": { "type": "enum", "values": ["0-300w", "300w-500w", "500w-700w"] },
        "time_scope": { "type": "enum", "values": ["past", "current"] },
        "polarity": { "type": "enum", "values": ["prefer", "avoid", "neutral"] }
      },
      "candidate_match_fields": ["memory_type", "region"],
      "conflict_fields": ["memory_type", "region", "time_scope"],
      "lineage_group_fields": ["memory_type", "region"]
    }
  },
  "filterable_fields": ["memory_type", "region", "budget_range", "time_scope", "polarity"],
  "retrieval_defaults": {
    "use_vector": true,
    "use_fulltext": true,
    "collapse_lineage": true
  }
}
```

---

### 6. Memories 采用 metadata JSONB

建议调整 `memories`：

- 删除：
  - `category`
  - `tags`
- 新增：
  - `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`
  - `schema_version INTEGER`
  - `lineage_group_key VARCHAR(255)` 或后续再决定是否做派生字段

说明：

- `lineage_group_key` 可以是派生优化字段，不必要求业务侧显式写
- `conflict_key` 也不建议做全局固定字段，更适合按 schema 动态计算

---

### 7. Events 采用 analysis_payload JSONB

建议调整 `events`：

- 新增：
  - `analysis_payload JSONB`
  - `analysis_status processing_status`
  - `analysis_schema_version INTEGER`
- 删除独立 `structured_events` 表

这样 profile 可以决定：

- 有的系统分析“六要素”
- 有的系统分析“意图/对象/结果/风险”
- 有的系统分析“会话摘要/用户状态/操作反馈”

事件分析格式不应在数据库层写死。

---

## API 与职责边界

### 8. Profile 初始化必须返回 schema proposal

初始化 profile 不再只是返回：

- purpose
- domain
- target_audience

还必须返回：

- `metadata_schema`
- `schema_status`
- `schema_version`

建议流程：

1. `POST /api/v1/systems`
   返回 draft schema proposal
2. `POST /api/v1/systems/{profile_id}/schema/confirm`
   冻结 schema

---

### 9. 调用方负责 schema-aware query planning

业务侧拿到 profile schema 后，应自行决定：

- 如何构造 memory metadata
- 如何构造 where/filter
- 需要查询什么意图

服务端不承担重型 query planner。

服务端负责：

- schema 存储
- metadata 校验
- memory 写入
- retrieve / reconcile / lineage collapse

调用方不直接检索 PG，而是调用 memory service 的 schema-aware API。

---

### 10. Retrieve API 支持 where/filter

需要从当前的：

- `query`
- `category_prefix`
- `tags`

升级为：

- `query`
- `where`
- `collapse_lineage`
- `include_history`

建议 where 语法支持：

- `eq`
- `neq`
- `in`
- `nin`
- `exists`
- `contains`
- `prefix`
- `gt/gte/lt/lte`
- `and/or/not`

并支持 JSONB 路径过滤。

---

## 覆盖与链路归并

### 11. Supersede 候选先按 metadata 缩圈

当前路径是：

1. 先向量召回
2. 再 consistency check
3. 再决定 reinforce / supersede

未来应改为：

1. 根据 profile schema 的 `candidate_match_fields` 找同主题候选
2. 再结合 embedding / fulltext / LLM 做 rerank 和冲突判断
3. 再决定 reinforce / supersede / merge

这样能稳定处理：

- 偏好变化
- 立场变化
- 路线变化
- 阶段性总结覆盖多条旧结论

---

### 12. 版本链模型后续升级为演变关系模型

当前仍保留：

- `supersedes`
- `superseded_by`

作为 Phase 1 的兼容实现。

但目标不是长期停在单链，而是后续升级为：

- `memory_relations`
- 支持 `supersedes_many`
- 支持 `summarizes`
- 支持 `supports`
- 支持 `contradicts`

这样才能真正支撑“多旧合一新”。

---

## 实施优先级

### Phase 1：打底

1. profile 增加 `metadata_schema/schema_status/schema_version`
2. profile 初始化返回 schema proposal
3. memory 增加 `metadata JSONB`
4. event 增加 `analysis_payload JSONB`
5. retrieve 增加基础 where/filter

### Phase 2：候选与冲突治理

1. supersede 候选按 metadata 缩圈
2. 冲突判断结合 metadata + semantic rerank
3. lineage collapse 改成 schema-aware

### Phase 3：关系模型升级

1. 从单链 supersede 升级为演变关系图
2. 支持多旧合一新
3. 支持 explanation / analytics / preference evolution

---

## 成功标准

做到下面这些，才算这条路线跑通：

1. profile 初始化会返回可冻结的 metadata schema proposal
2. memory 写入可以携带 JSONB metadata，而不是只靠 category/tags
3. retrieve 支持 where/filter，而不只是 query string
4. supersede 候选不再只依赖 embedding 相似度
5. event analysis 不再依赖固定六要素表结构
6. 不同业务系统可以拥有完全不同的 metadata 结构，但共享同一 memory engine
