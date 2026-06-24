# Memory Server 下一版优化 TODO

## 核心边界

这一版优化先明确一个原则：

Memory Server 不负责业务侧 Query Planning。

业务侧应该根据自己的任务、场景、用户意图和产品流程决定：

- 本轮需要哪些记忆类型
- 查当前 scope、父级 scope、全局记忆还是历史证据
- 不同记忆 bucket 的 top_k 和权重
- 是否需要解释、证据链或历史版本

Memory Server 只负责提供稳定、可治理的基础能力：

- 记忆存储
- 事件驱动更新
- create / reinforce / supersede
- profile / owner / scope 隔离
- 检索原语
- 命中反馈
- 生命周期治理
- 审计和可恢复性

因此，后续优化不在服务端默认引入 LLM Query Planner。服务端可以支持业务侧传入 `intent`、`scene`、`categories`、`scope_policy`、`bucket` 等结构化 hint，但不替业务做规划。

---

## 当前实现进度

已落地：

- 修复 PostgreSQL 检索中 `scope_id = null` 的跨 scope 召回问题。
- 修复 full-text / content similarity 路径的 `scope_id = null` 语义，使其与普通检索一致。
- 修复 decay score 更新 SQL 参数错位。
- 增加 `processing` 事件状态和原子 claim，避免同一个 pending event 被并发处理。
- retry 现在会先把 failed event reset 为 pending，再走 claim 处理。
- 事件处理中的 embedding / Qdrant 写失败不再让整条 event 处理失败，而是把 memory 标记为 `embedding_status = failed`。
- promote / archive / supersede 的关键路径已开始同步 Qdrant payload 或删除旧 vector。
- Qdrant vector filter 在没有 `scope_id` 时默认只预召回 global，避免候选槽位被其他 scope 污染。
- 增加同步小批量修复接口：`POST /api/v1/systems/{profile_id}/admin/embeddings/rebuild`。

仍需继续：

- 事件处理的 PostgreSQL 写入还没有完整事务化，version chain 并发保护仍需单独做。
- Qdrant 同步目前是同步补偿 + rebuild，不是完整 outbox / background worker。
- lifecycle batch transition 后的 Qdrant 状态同步仍需覆盖。
- bucket retrieval、memory pack、candidate memory、README 更新仍未实现。

---

## P0：修复会影响业务正确性的 bug

### 1. 修复 `scope_id = null` 检索语义

当前问题：

- API 注释说 `scope_id = null` 默认只查 global memories。
- 但 `find_for_retrieval_by_profile` 中的 SQL 条件：

```sql
AND (
  $3::text IS NULL
  OR scope_id = $3
  OR ($4 = true AND is_global = true)
)
```

会导致 `scope_id = null` 时返回该 owner 下所有 scope 的当前记忆。

影响：

- 跨 scope 召回。
- 当前会话可能被其他任务、其他上下文污染。
- 用户记忆解释不可信。

建议修复：

- 当 `scope_id IS NULL` 时，只返回 `is_global = true`。
- 当 `scope_id IS NOT NULL` 时，返回 `scope_id = $scope_id`，并根据 `include_global` 决定是否包含 global。

验收标准：

- `scope_id = null` 只返回 `is_global = true` 的记忆。
- `scope_id = "s1"` 且 `include_global = true` 返回 `s1 + global`。
- `scope_id = "s1"` 且 `include_global = false` 只返回 `s1`。
- 增加 repository 层测试覆盖三个分支。

### 2. 修复 decay score 更新 SQL 参数错位

当前问题：

`update_decay_scores` SQL 中 `$2` 同时被用于 `hit_boost_factor` 和 `owner_id`：

```sql
SET decay_score = (1.0 + ln(hit_count + 1) * $2)
...
WHERE profile_id = $1
  AND owner_id = $2
```

但代码 bind 顺序是：

```rust
.bind(profile_id)
.bind(owner_id)
.bind(hit_boost_factor)
.bind(decay_half_life_days)
.bind(global_boost)
```

影响：

- decay 更新接口不可用或计算错误。
- 生命周期治理无法可靠运行。

建议修复：

- 重新编号 SQL 参数，例如：

```sql
SET decay_score = (1.0 + ln(hit_count + 1) * $3)
                * exp(... / $4)
                * CASE WHEN is_global THEN $5 ELSE 1.0 END
WHERE profile_id = $1
  AND owner_id = $2
```

验收标准：

- admin decay update 能正常执行。
- 只更新指定 `(profile_id, owner_id)`。
- 增加集成测试或 repository 测试。

### 3. 增加事件处理的原子领取机制

当前问题：

- Event 状态只有 `pending | completed | failed | skipped`。
- `process_existing_event` 先读 event，再判断 terminal，然后直接处理。
- 多个 worker 或重复请求可以同时处理同一个 pending event。

影响：

- 重复创建 memory。
- 重复 reinforce。
- supersede 版本链可能分叉。

建议修复：

- 增加 `processing` 状态。
- 增加 repository 方法：

```text
claim_for_processing(profile_id, event_id)
```

语义：

```sql
UPDATE events
SET processing_status = 'processing'
WHERE id = $event_id
  AND profile_id = $profile_id
  AND processing_status IN ('pending', 'failed')
RETURNING *
```

验收标准：

- 同一 event 只能被一个 worker claim 成功。
- 未 claim 成功的请求返回可解释错误。
- `retry` 必须先 reset/claim，再处理。

### 4. 明确事件处理的幂等和补偿策略

当前问题：

事件处理链路会先写 PostgreSQL memory / relation，再写 Qdrant，最后标记 event completed。中间失败会留下部分副作用。

影响：

- event failed 但 memory 已创建。
- retry 可能重复创建 memory。
- Qdrant 与 PostgreSQL 不一致。

建议方案：

- 第一阶段先保证 PostgreSQL 内部事务一致：
  - memory create
  - reinforce
  - supersede
  - event-memory relation
  - event status update
- Qdrant 写入不放在同一个事务里，而是改成可恢复的异步/补偿机制：
  - memory.embedding_status = pending
  - background job 写 Qdrant
  - 成功后标记 completed
  - 失败标记 failed，并支持 rebuild

验收标准：

- DB 写入失败不会留下半条 version chain。
- Qdrant 失败不会导致 event 业务处理整体重复。
- 支持按 profile rebuild embedding。

### 5. 同步 Qdrant payload 与 memory 状态

当前问题：

以下操作只更新 PostgreSQL，没有同步 Qdrant payload：

- promote to global
- archive/delete
- supersede old memory
- status transition

影响：

- Qdrant 召回候选越来越脏。
- global promotion 后向量 payload 仍可能是旧 scope。
- archived / superseded 记忆仍参与向量候选。

建议修复：

- 短期：所有状态变更后更新或删除对应 Qdrant point。
- 中期：增加 embedding outbox / rebuild job。
- 长期：Qdrant 只做召回，PostgreSQL 做最终过滤，但仍保持 payload 尽量同步以保证性能和解释性。

验收标准：

- promote 后 payload 中 `is_global=true` 且 `scope_id=null`。
- archive/supersede 后 vector 不再以 active 状态召回。
- 提供 `/admin/embeddings/rebuild`。

---

## P1：收敛业务语义

### 1. 统一 `scope_id = null` 与 `is_global`

当前问题：

- `scope_id = null` 在注释中表示 global context。
- `is_global` 又独立表示跨 scope 可用。
- 当前允许 `scope_id = null && is_global = false`，语义不清。

建议决策二选一：

方案 A：`is_global` 是唯一全局标记。

- `scope_id = null` 只表示“不绑定具体 scope”。
- 但检索 global 必须看 `is_global=true`。
- 需要允许 null non-global，但要明确它的用途。

方案 B：`scope_id = null` 等价 global。

- DB 约束：`scope_id IS NULL` 必须 `is_global = true`。
- 写入事件如果 `scope_id = null`，抽取出的 memory 默认 global。
- 语义简单，但可能过度放大全局记忆。

建议采用方案 A，但对 API 文档写清楚：

- 写事件时 `scope_id = null` 表示事件不属于短期上下文。
- 创建 memory 时 `is_global=true` 才表示跨 scope 可用。
- 检索时 `scope_id = null` 默认只查 `is_global=true`。

验收标准：

- 文档、代码注释、检索实现一致。
- API 对 `scope_id=null && is_global=false` 的含义有明确说明，或直接禁止。

### 2. Memory Server 不做默认 LLM Query Planning

设计决策：

- 不在普通 recall 中同步调用 LLM 做 planning。
- 业务侧负责决定本轮需要哪些记忆。
- Memory Server 接收结构化 hint，并提供可靠检索原语。

建议新增可选检索字段：

```json
{
  "query": "给用户推荐房子",
  "owner_id": "user-123",
  "scope_id": "session-456",
  "options": {
    "scope_policy": "current_scope_plus_global",
    "categories": ["constraint", "preference", "recent_intent"],
    "tags": ["budget", "location"],
    "include_evidence": false,
    "include_history": false
  }
}
```

注意：

- `scope_policy` 是业务 hint，不是 LLM plan。
- Memory Server 不解释业务意图，只执行过滤和排序。

验收标准：

- 默认检索不增加 LLM 调用。
- 业务侧可以通过结构化参数控制检索范围。
- 检索延迟稳定可控。

### 3. 将 `SystemProfile` 真正用于 memory extraction 策略

当前问题：

- `SystemProfile.extraction_prompt` 主要用于 structure event。
- 记忆抽取仍然使用通用 prompt。

影响：

- 不同业务系统虽然数据隔离，但“什么值得记”不够业务化。

建议优化：

- Profile 中增加或复用 memory extraction 指导：
  - valuable memory types
  - ignore rules
  - category taxonomy
  - confidence policy
  - sensitive inference policy
- `MemoryProcessor.extract_from_event` 接收 profile-derived extraction config。

验收标准：

- 房产系统、客服系统、研发助手可以抽取出不同风格的 memory。
- profile boundary 能影响“是否抽取”和“抽取什么”。

---

## P1：优化检索作为基础原语

### 1. 支持 bucket retrieval，而不是单一 top-k

当前问题：

单一 top-k 会把不同类型记忆混在一起排序，例如：

- 当前会话意图
- 稳定偏好
- 硬约束
- 历史事实
- 业务规则

建议新增服务端原语：

```json
{
  "buckets": [
    {
      "name": "current_scope",
      "scope_policy": "current_only",
      "top_k": 5
    },
    {
      "name": "global_preferences",
      "scope_policy": "global_only",
      "category_prefix": "preference",
      "top_k": 5
    },
    {
      "name": "constraints",
      "scope_policy": "current_scope_plus_global",
      "category_prefix": "constraint",
      "top_k": 3
    }
  ]
}
```

说明：

- bucket 由业务侧指定。
- Memory Server 只执行 bucket 检索。
- 返回结果保留 bucket 分组，方便业务侧组装 prompt。

验收标准：

- 支持同一次请求中多个 bucket。
- 每个 bucket 可以有独立 top_k / scope_policy / category / min_confidence。
- 命中反馈仍能正确记录。

### 2. 返回 Memory Pack，而不是只返回平铺 memories

当前普通返回可以保持轻量：

```json
{
  "memories": [
    {
      "id": "...",
      "content": "...",
      "score": 0.82,
      "confidence": 0.9,
      "category": "preference.location",
      "tags": ["quiet", "subway"]
    }
  ]
}
```

高级返回可以支持：

```json
{
  "buckets": [
    {
      "name": "global_preferences",
      "memories": []
    }
  ],
  "pack_text": "- 用户偏好安静房源\n- 用户预算约 300 万"
}
```

注意：

- `pack_text` 只是格式化，不做 LLM 总结。
- 是否将 pack_text 放入 Agent prompt 由业务侧决定。

验收标准：

- 默认轻量返回。
- 高级模式可返回 bucket 分组和可直接拼 prompt 的文本。

### 3. 调整 score 语义

当前风险：

- `hit_count` 容易形成强者恒强。
- 没有向量命中的候选如果默认 similarity=0.5，会被错误抬高。
- global memory 是否加权不明确。

建议：

- vector 未命中时 similarity 设为 0，而不是 0.5。
- 对 hit_count 使用更弱的饱和函数，并限制最大贡献。
- global boost / penalty 配置化。
- scope-local memory 和 global memory 分开排序或分桶，不强行混排。

验收标准：

- score explain 能说明各项贡献。
- global 记忆不会无条件压过 current scope。
- 纯全文命中和向量命中有可解释区分。

---

## P1：记忆写入和演化策略

### 1. 引入 candidate memory

当前问题：

事件抽取出的 memory 直接 active，容易把一次性表达变成稳定用户记忆。

建议：

- 低置信度、单次出现、推断型 memory 进入 candidate。
- 明确事实或业务侧手动写入可 active。
- 多次 reinforce 后 candidate -> active。

可参考规则：

```text
fact confidence >= 0.9 -> active
preference confidence >= 0.8 and importance >= 0.6 -> active
preference/pattern low confidence -> candidate
rule -> active or needs confirmation,由 profile 决定
```

验收标准：

- candidate 默认不参与普通检索，或仅高级检索可查。
- reinforce candidate 到阈值后自动 active。
- 审计记录状态变化原因。

### 2. 强化 version chain 并发保护

当前问题：

supersede 使用 `old.version_number + 1`，没有事务锁和唯一约束时可能并发生成多个 current version。

建议：

- supersede 在 DB transaction 内执行。
- 对旧 current memory `SELECT FOR UPDATE`。
- 增加唯一约束，保证同一 root 只有一个 current version。

验收标准：

- 并发 supersede 不会出现两个 current version。
- version history 始终线性可解释。

### 3. 明确 reinforce 与 hit 的区别

当前 reinforce 会增加 confidence、hit_count、reinforcement_count。

建议：

- `hit` 表示被检索使用。
- `reinforce` 表示有新事件证据支持。
- 两者可以同时发生，但业务语义要分开记录。

验收标准：

- evidence count 只来自事件关系，不来自普通检索。
- hit_count 不影响“证据强度”。

---

## P2：可恢复性和运维

### 1. 增加 embedding rebuild 任务

用途：

- Qdrant collection 重建。
- embedding provider 更换。
- payload schema 变更。
- 修复历史写入失败。

建议接口：

```text
POST /api/v1/systems/{profile_id}/admin/embeddings/rebuild
```

参数：

```json
{
  "owner_id": "optional",
  "status": ["active", "cooldown"],
  "dry_run": true
}
```

验收标准：

- 可 dry-run。
- 可按 profile / owner 执行。
- 返回重建数量、失败数量、失败原因。

### 2. 完善 health check

当前问题：

- Qdrant health 是占位。
- embedding provider health 是占位。

建议：

- PostgreSQL：简单查询。
- Qdrant：真实 health check。
- embedding：可配置是否真实探测，避免频繁调用外部 API。
- LLM：可选探测。

验收标准：

- Qdrant 不可用时 health 至少 degraded。
- PostgreSQL 不可用时 unhealthy。
- 外部 provider 探测可关闭。

### 3. 增加 processing trace

建议记录：

- trace_id
- event_id
- profile_id
- owner_id
- relevance 耗时
- structure event 耗时
- extraction 耗时
- matching 耗时
- reconcile 耗时
- embedding 耗时
- qdrant 写入耗时
- token usage / model / prompt version

验收标准：

- 同步和异步事件都返回 trace_id。
- 失败事件能定位失败阶段。

---

## P2：文档和测试

### 1. 更新 README

README 应改为新版模型：

- `profile_id` 是业务系统命名空间。
- `owner_id` 只在 profile 内有意义。
- `scope_id` 是业务侧上下文。
- `is_global` 表示同一 `(profile_id, owner_id)` 下跨 scope 可用。
- Memory Server 不做业务 Query Planning。
- 业务侧通过 hint 调用检索原语。

### 2. 增加核心调用示例

至少包含：

- 创建 system profile
- 写入事件
- 异步写入事件
- 直接创建 memory
- 普通检索
- bucket 检索
- 查询 evidence
- 查询 history
- retry failed event
- rebuild embeddings

### 3. 增加测试脚本

建议测试集：

- 多 profile 同 owner_id 隔离。
- 同 owner 不同 scope 隔离。
- global memory 跨 scope 可查。
- `scope_id = null` 只查 global。
- 事件重复处理幂等。
- supersede 版本链线性。
- Qdrant payload 状态同步。
- decay update 正常执行。
- candidate memory 默认不污染普通检索。

---

## 建议执行顺序

1. 修 `scope_id = null` 检索 SQL。
2. 修 decay score SQL 参数。
3. 增加 `processing` 状态和 event claim。
4. 收敛 `scope_id` / `is_global` 语义。
5. 把事件处理 DB 写入放进事务。
6. 增加 Qdrant payload 同步和 embedding rebuild。
7. 支持业务侧传 retrieval hint。
8. 支持 bucket retrieval。
9. 引入 candidate memory。
10. 更新 README 和测试脚本。
