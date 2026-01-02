# Memory Server 整体方案（Final Summary）

> **定位一句话：**
> 一个独立的、可嵌入任意 Agent / LLM 应用的 **Memory Server**，
> 负责 **上下文记忆的结构化存储、检索、演化与治理**，
> 不负责推理、不绑定模型、不侵入业务。

---

## 1. 核心设计原则（定死）

### 1.1 Memory ≠ KB ≠ Chat History

* **Chat History**：原始对话，不进 Memory Server
* **Memory**：结构化上下文 / 结论 / 规则
* **KB**：事实、文档、证据

👉 Memory Server **只管 Memory**

---

### 1.2 三层 Memory（不再讨论）

```text
Session Memory   → 短期上下文（分钟 / 小时）
Task Memory      → 任务级结论（天 / 周）
Long-term Memory → 稳定规则（长期，需确认）
```

---

### 1.3 Server 是“治理层”，不是“智能体”

* 不调用 LLM
* 不自动生成记忆
* 不隐式升级记忆
* 所有行为 **显式 + 可审计**

---

## 2. 总体架构（最终形态）

```
┌────────────────────────┐
│ Agent / LLM Application│
│  - LangChain / 自研     │
└──────────┬─────────────┘
           │ HTTP / gRPC
┌──────────▼─────────────┐
│ Memory Server (Rust)   │
│                        │
│  API Layer             │
│  Memory Guard          │
│  Retrieval Engine      │
│  Lifecycle Manager     │
│                        │
└───────┬────────┬───────┘
        │        │
┌───────▼───┐ ┌──▼────────┐
│PostgreSQL │ │ Vector DB │
│- 状态/层级 │ │(Qdrant)  │
│- Scope     │ │- embedding│
│- TTL/审计  │ │- 相似度   │
└───────────┘ └───────────┘
```

---

## 3. 存储选型（定案）

### 3.1 主存储：PostgreSQL

负责：

* Memory 元数据
* 层级（layer）
* 场景（scene）
* 状态（status）
* 生命周期（TTL / hit / 冷却）

### 3.2 向量存储：Qdrant（优先）

负责：

* embedding
* 相似度检索
* 轻量 payload filter

> 规模小可用 pgvector，但最终推荐 Qdrant

---

## 4. Memory 核心数据模型（统一认知）

### 4.1 Memory 关键维度（不要再加）

| 维度       | 作用         |
| ---------- | ------------ |
| layer      | 能活多久     |
| scope      | 属于谁       |
| scene      | 用在什么场景 |
| status     | 是否参与检索 |
| importance | 业务权重     |
| confidence | 可靠程度     |

---

### 4.2 Memory 表（抽象）

```text
Memory {
  id
  layer        // session / task / long
  scope_type   // user / org / project / task / session
  scope_id
  scene        // work.contract_review
  status       // active / cooldown / ignored
  content_md
  importance
  confidence
  hit_count
  last_hit_at
  ttl
}
```

---

## 5. Scene（场景）体系（固定）

> Scene = “什么时候用这条记忆”

```text
life.*
work.*
meta.*
```

示例：

* `work.contract_review`
* `work.enterprise_research`
* `meta.methodology`

Scene 是 **检索过滤 + 权重调节** 的核心。

---

## 6. Memory 生命周期（定死逻辑）

### 6.1 状态机

```text
candidate
 → active
 → stable
 → long-term (候选)
 → deprecated
 → archived / dropped
```

⚠️ **永不自动写 long-term**

---

### 6.2 冷却 / 摒弃

* 长期未命中 → cooldown
* 明确废弃 → ignored（永不检索）

---

## 7. 检索逻辑（最关键）

### 7.1 标准检索流程（不可跳过）

```
1. PostgreSQL 结构过滤
   - scope
   - layer
   - scene
   - status = active

2. 向量相似度召回
   - embedding similarity

3. 综合打分排序
   score =
     相似度
   + 重要性
   + 新近度
   + 命中次数

4. Top-K 返回
```

---

### 7.2 命中即反馈

每次命中：

* hit_count +1
* last_hit_at 更新

👉 **这是记忆“演化”的唯一驱动力**

---

## 8. 写入 / 更新 / 忽略规则（强约束）

### 8.1 写入

* Agent 只能写：

  * session
  * task
* long-term 需人工 / 业务确认

### 8.2 更新模式（必须显式）

* append
* merge
* supersede

### 8.3 忽略

* ignored 状态的 memory 永不进 prompt

---

## 9. 向量化配置（可替换）

```yaml
embedding:
  provider: openai | local
  model: text-embedding-3-large | bge-large-zh
  dim: 1024 / 3072
```

* Memory Server 只调用 embedding
* 不关心模型语义

---

## 10. 技术实现（最终技术栈）

| 层     | 技术              |
| ------ | ----------------- |
| Server | Rust              |
| Web    | Actix Web         |
| Async  | Tokio             |
| DB     | PostgreSQL + sqlx |
| Vector | Qdrant            |
| Config | YAML              |
| Audit  | PG 表             |

---

## 11. 你这套方案的“定位价值”

### 对 Agent：

* 不用自己管 memory
* 可控、可调、可审计

### 对业务：

* 明确上下文边界
* 不怕“模型乱记”

### 对未来：

* 可演进成企业级
* Knox Chat 同路线

---

## 12. 一句话总结（可以直接对外说）

> **Memory Server 是 LLM 系统的“上下文治理层”，
> 它不让模型变聪明，
> 但让系统变可靠。**


