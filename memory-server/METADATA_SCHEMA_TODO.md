# Metadata Schema Driven Memory TODO

## 背景

当前系统在记忆覆盖、链路归并、历史偏好演变识别上，过度依赖向量相似度和自然语言内容本身。

这会导致几个核心问题：

1. 很难精准找到“之前相关的言论 / memory”
2. 偏好变化、立场变化、技术路线变化不容易触发 supersede
3. 多条历史记忆很难被一个新的阶段性结论统一收束
4. 检索对业务侧不够可控，用户侧无法稳定参与 query planning

因此需要引入一套 **profile 驱动的 metadata schema**，让 memory 的写入、检索、覆盖、链路归并都建立在可控的结构化语义之上。

---

## 核心目标

### 1. 让 profile 不只是描述信息

当前 profile 中的 `purpose`、`domain` 等字段，不应该只是文本说明。

目标是让 profile 在初始化完成后，返回一份可供业务侧使用的 schema 定义，例如：

- 有哪些实体类型
- 哪些 metadata 字段是稳定可过滤的
- 哪些字段是枚举值
- 哪些字段会参与覆盖判断
- 哪些字段会参与链路归并

换句话说：

> profile 应该定义 memory system 的结构化语义契约，而不是只定义一个 namespace。

---

## 需要设计的能力

### 2. Profile 初始化返回 metadata schema

初始化 profile 后，服务端应返回一份结构化 schema，供用户侧后续构建 memory 和 query。

建议至少包含：

- `entity_types`
- `filterable_fields`
- `hierarchical_fields`
- `enum_fields`
- `time_fields`
- `conflict_fields`
- `lineage_group_fields`
- `retrieval_defaults`

示例方向：

```json
{
  "entity_types": {
    "preference": {
      "subject": {
        "type": "enum",
        "values": ["programming_language", "framework", "work_style"]
      },
      "time_scope": {
        "type": "enum",
        "values": ["past", "current"]
      },
      "polarity": {
        "type": "enum",
        "values": ["like", "dislike", "neutral"]
      },
      "strength": {
        "type": "float"
      }
    }
  }
}
```

---

### 3. Memory 写入时允许附带 metadata

除了自然语言 `content`，memory 写入时还应允许用户侧显式提供结构化 metadata。

例如：

- `entity_type = preference`
- `subject = programming_language`
- `object = rust`
- `time_scope = current`
- `polarity = like`

这样用户说：

- 去年喜欢 C++
- 去年喜欢 LangChain
- 今年不喜欢了

系统就不需要只靠 embedding 猜“是不是相关”，而是能先从 metadata 找出：

- subject 接近
- 同类 preference
- 时间上可能被覆盖

---

### 4. Query 支持 where/filter schema

需要定义一套稳定的结构化检索条件，而不是只有自然语言 query。

参考目标：

```json
{
  "where": [
    {
      "subject": {
        "in": ["programming_language", "framework"]
      }
    },
    {
      "time_scope": {
        "eq": "current"
      }
    }
  ]
}
```

建议支持：

- `eq`
- `neq`
- `in`
- `nin`
- `exists`
- `prefix`
- `contains`
- `gt/gte/lt/lte`
- `and/or/not`

---

## 覆盖与链路归并

### 5. 覆盖候选不应只依赖向量相似度

当前 supersede 触发路径是：

1. 先向量命中候选
2. 再做 consistency/conflict check
3. 再决定 supersede

问题是：

- 同主题但表述不同的记忆，可能进不了候选集
- 偏好变化、技术路线变化、阶段性结论变化都很容易漏掉

目标改造：

1. 先按 metadata 找“同主题候选集”
2. 再结合 embedding / fulltext / LLM 做冲突判断
3. 再决定 reinforce / supersede / merge

---

### 6. 去重升级为“链路归并”

去重不应只是删掉重复结果。

更合理的目标是：

- 识别哪些 memory 属于同一演变主题
- 将其归并到当前有效结论
- 同时保留历史链路，用于解释和后续分析

当前已经做了部分检索阶段的 lineage collapse，但还不够。

后续需要补：

1. 写入阶段的 lineage-aware merge
2. 检索阶段的 schema-aware lineage collapse
3. 历史解释层的 lineage 展示

---

### 7. 支持“多旧合一新”的覆盖模型

当前 schema 只能表达单链：

- `supersedes: Option<Uuid>`
- `superseded_by: Option<Uuid>`

但真实业务会出现：

- 多条旧 memory
- 被一条新的阶段性结论统一覆盖 / 收束

例如：

- 去年喜欢 C++
- 去年喜欢 LangChain
- 今年转向另一种技术路线

目标是支持：

- 一个新 memory 覆盖多个旧 memory
- 检索时默认返回当前结论
- 历史链路保留被覆盖的旧结论集合

这意味着后续需要从“版本链”升级为“记忆演变关系”。

---

## 用户侧职责

### 8. Query Planning 放在业务侧

这个系统不应该承担重型 query planning。

建议边界：

- 服务端负责：存储、更新、检索、反馈、链路归并
- 用户侧负责：根据 profile schema 构建 metadata、生成 where/filter 条件、决定查询意图

换句话说：

> 服务端做 memory engine，业务侧做 schema-aware query planner。

---

### 9. Profile schema 返回后，用户侧需要有 SDK/文档

如果 profile 真的返回 schema，用户侧必须知道怎么用。

至少需要：

1. memory 写入 payload 示例
2. query where/filter 示例
3. supersede / merge 相关 metadata 建议
4. 常见 entity_type 设计参考

---

## 实现优先级建议

### Phase 1

1. 给 profile 增加 metadata schema 定义能力
2. profile 初始化返回 schema
3. memory 写入支持 metadata 字段
4. retrieve 支持 where/filter 基础语法

### Phase 2

1. 用 metadata 缩小 supersede 候选集
2. 用 metadata + semantic rerank 做 conflict check
3. 将“best match only”升级为“same-topic candidate set”

### Phase 3

1. 从单链 supersede 升级为记忆演变关系模型
2. 支持多旧合一新
3. 支持更完整的 lineage collapse / explanation / analytics

---

## 成功标准

做到下面这些，才算这条路线真正跑通：

1. 用户侧可以根据 profile 返回的 schema 来构建 metadata
2. 检索支持结构化 where/filter，而不只是 query string
3. 偏好变化类场景可以稳定找到“之前相关的记忆”
4. supersede 不再只依赖 embedding 相似度
5. 检索默认返回当前有效结论，同时保留历史链路
6. 后续可以对用户偏好变化、路线变化做分析，而不是只看孤立 memory
