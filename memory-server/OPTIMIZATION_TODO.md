# Memory Server 优化 TODO

## 背景定义

当前框架的核心方向保持不变：以事件驱动记忆的创建、强化、冲突更新和生命周期治理。

需要修正和明确的是 `SystemProfile` / `profile_id` 的语义：

- `SystemProfile` 表示一个业务系统的记忆画像和处理边界，例如系统 A、系统 B。
- `profile_id` 是系统命名空间，不是用户租户，也不是用户身份。
- `owner_id` 由业务侧定义，只在某个 `profile_id` 内有意义。
- 即使系统 A 和系统 B 都有一个 `owner_id = 张三`，也不应被认为是同一个记忆主体。
- 记忆归属的最小身份应理解为 `(profile_id, owner_id, scope_id)`。
- `is_global = true` 只表示同一个 `(profile_id, owner_id)` 下跨 `scope_id` 可用，不允许跨 `profile_id`。

因此，后续优化目标是：保留事件驱动架构，但让系统边界、调用入口和内部编排更清晰。

## P0：统一系统命名空间语义

- [x] 将文档和代码注释里的 `SystemProfile is singleton` 改为“系统级命名空间”。

  当前 `src/domain/profile.rs`、`src/repository/profile_repo.rs` 中的注释容易让人误解为整个服务只能服务一个系统。更准确的描述应该是：一个 `SystemProfile` 对应一个业务系统的记忆空间。

- [x] 决定是否允许一个服务实例内存在多个 `SystemProfile`。

  建议允许多个。即使部署时可以一个实例只服务一个系统，数据模型也应支持多系统隔离，因为现有 schema 已经在 events 和 memories 上强制要求 `profile_id`。

- [x] 调整 `ProfileRepository` 的单例逻辑。

  当前 `exists()` 和 `get()` 默认只处理第一条 profile。应改为：

  - `create(profile)`：创建一个系统画像。
  - `get_by_id(profile_id)`：按系统 ID 获取画像。
  - `get_by_name(name)`：可选，用于管理端查询。
  - `list()`：可选，用于管理多个系统。
  - `update(profile_id, input)`：更新指定系统画像。

- [x] 给 `system_profiles.name` 加唯一约束或明确允许重名。

  如果系统名称用于人类识别，建议 `name` 唯一。若允许重名，则管理端必须始终以 `profile_id` 操作。

- [x] 明确 `owner_id` 的约束范围。

  文档中应写清楚：`owner_id` 不要求全局唯一，只要求业务侧在同一个 `profile_id` 内保持一致。

## P0：修复 profile 级数据隔离

- [x] 所有 memory 查询必须带 `profile_id`。

  重点检查并改造这些路径：

  - `find_by_owner_scope_grouped_by_provider`
  - `find_for_retrieval_by_profile`
  - `fulltext_search`
  - `find_similar_by_content`
  - `update_decay_scores`
  - `find_eviction_candidates`
  - lifecycle / promotion 相关查询

- [x] 所有 event 查询和关联查询必须校验 `profile_id`。

  例如 `GET /events/{id}` 如果只传 event id，理论上 UUID 已经唯一，但对外 API 最好仍落在系统路径下，避免管理和权限语义混乱。

- [x] Qdrant payload 增加 `profile_id` 和 `owner_id`。

  当前 vector payload 只有 `memory_id`、`scope_id`、`category`、`is_global`、`status`。建议至少加入：

  ```text
  profile_id
  owner_id
  scope_id
  is_global
  status
  category
  ```

- [x] Qdrant search filter 必须带 `profile_id` 和 `owner_id`。

  不能只依赖 PostgreSQL 二次过滤。否则不同系统同名 owner 的向量候选会混入召回阶段，影响性能、排序和可解释性。

- [x] 所有 lifecycle 任务按 `profile_id` 隔离。

  衰减、淘汰、global promotion 都不应只按 `owner_id` 跑。

- [x] 审计日志建议补充 `profile_id`。

  已在 `audit_logs` 增加 `profile_id`，并将查询入口收敛到 `/api/v1/systems/{profile_id}/audit`。

## P0：收敛事件处理编排

- [x] 新增 `EventIngestionService` 或 `MemoryApplicationService`。

  它应该负责完整事件处理链路：

  ```text
  create event
  load system profile
  load context memories
  relevance check
  structure event
  extract memories
  match existing memories
  reconcile create/reinforce/supersede
  generate embedding
  upsert Qdrant
  mark event processed
  return processing result
  ```

- [x] API handler 只做参数解析、校验和调用 service。

  当前 `src/api/event.rs` 里手工创建 `EventHandler`、`MemoryProcessor`、embedding provider context、Qdrant repo，这些都应下沉到 service。

- [x] 简化 `EventProcessingContext`。

  已删除 `embedding_providers: HashMap`，当前按全局 embedding provider 统一处理。

- [x] 合并同步和异步事件处理逻辑。

  `create_event` 和 `create_event_async` 应复用同一个 service：

  - 同步：`ingest_event_sync(request)`
  - 异步：`create_pending_event(request)` + `process_existing_event(event_id)`

- [x] 事件处理结果要完整返回。

  `CreateFromEventResult` 需要包含：

  - `created_memory_ids`
  - `reinforced_memory_ids`
  - `superseded_memory_ids`
  - `memories_created`
  - `memories_reinforced`
  - `memories_superseded`
  - `skipped`
  - `skip_reason`

- [x] 结构化事件需要真正落库。

  当前已有 `structured_events` 表和 `StructuredEventRepository`，但处理链路里没有保存。若六要素模型是框架特色，应在 `structure_event` 成功后写入表。

## P1：简化外部 API

- [x] 将系统边界放到 path 中，而不是散落在 body 中。

  建议风格：

  ```text
  POST /api/v1/systems/{profile_id}/events
  POST /api/v1/systems/{profile_id}/events-async
  GET  /api/v1/systems/{profile_id}/events/{event_id}
  POST /api/v1/systems/{profile_id}/memories/retrieve
  POST /api/v1/systems/{profile_id}/memories
  GET  /api/v1/systems/{profile_id}/memories/{memory_id}
  ```

- [x] 事件写入 API 保持最小请求体。

  建议最小字段：

  ```json
  {
    "owner_id": "user-123",
    "content": "用户说他现在更喜欢安静的房源",
    "scope_id": "session-456",
    "context": "一次房源推荐对话",
    "source": "conversation"
  }
  ```

- [x] 检索 API 保持最小请求体。

  建议最小字段：

  ```json
  {
    "owner_id": "user-123",
    "query": "用户喜欢什么样的房子",
    "scope_id": "session-456",
    "top_k": 10
  }
  ```

- [x] 高级检索参数移到 advanced endpoint 或 options 对象。

  如 `use_vector`、`use_fulltext`、`highlight`、`include_evidence`、`include_history`、`min_score`、`min_confidence` 不应干扰普通调用。

- [x] 明确 `scope_id = null` 的语义。

  建议：

  - 写入事件时 `scope_id = null` 表示该事件不属于某个短期上下文。
  - 检索时 `scope_id = null` 默认只查全局记忆，还是查该 owner 的所有记忆，需要明确。当前代码不同路径语义不完全一致，建议统一。

- [x] `source` 建议提供推荐枚举。

  可选值：

  - `conversation`
  - `user_action`
  - `system_event`
  - `manual`
  - `api`

  字段仍可保留为字符串，但文档应给推荐值。

- [x] 提供 SDK 级别的简化方法。

  对智能体侧最好只暴露：

  ```text
  remember(system, owner, scope, content)
  recall(system, owner, scope, query)
  ```

  不让调用方理解内部的 extraction、reconcile、embedding、Qdrant。

## P1：统一匹配和冲突处理

- [x] 事件处理链路统一使用 `MemoryMatcher`。

  当前已有 `MemoryMatcher`，但 `MemoryGuard::process_event_with_context` 中又手写了一套 provider group + Qdrant search + score filter。应复用 `MemoryMatcher`，避免规则分散。

- [x] 匹配阈值进入配置。

  当前事件处理里有硬编码相似度阈值 `0.70`。建议配置：

  ```text
  match_similarity_threshold
  conflict_check_similarity_threshold
  max_match_candidates
  ```

- [x] LLM consistency checker 输出结构化 JSON。

  现在只要求模型返回 `CONSISTENT` 或 `CONFLICTING`，建议改为：

  ```json
  {
    "result": "consistent|conflicting",
    "reason": "为什么一致或冲突",
    "confidence": 0.87
  }
  ```

- [x] supersede 记录冲突原因。

  新版本记忆应保存：

  - 触发冲突的 event id
  - 被替代 memory id
  - 冲突判断 reason
  - consistency confidence

- [x] reinforce 不只增加 confidence。

  建议增加或显式维护：

  - `reinforcement_count`
  - `last_reinforced_at`
  - supporting event count

  这样检索解释和 global promotion 都会更可靠。

## P1：事件状态模型优化

- [x] 将 `processed: bool` 升级为处理状态。

  建议字段：

  ```text
  processing_status: pending | processing | completed | skipped | failed
  error_message: nullable text
  processed_at: nullable timestamp
  ```

- [x] 不要把 skip 信息写进 summary。

  `summary` 应该只表示事件摘要。跳过原因应使用：

  ```text
  skipped: bool
  skip_reason: text
  relevance_score: float
  ```

- [x] 异步任务失败要落库。

  当前 background task 失败只打日志。应将事件状态更新为 `failed`，并记录错误信息。

- [x] 支持重试失败事件。

  增加管理接口：

  ```text
  POST /api/v1/systems/{profile_id}/events/{event_id}/retry
  ```

## P2：降低 LLM 调用成本和延迟

- [ ] 合并相关性判断和记忆抽取。

  当前可能有多次 LLM 调用：

  ```text
  relevance check
  structure event
  extract memory
  consistency check
  ```

  可以先保留多阶段，但提供一个 fast path：一次 LLM 输出 relevance、event_summary、structured_event、extracted_memories。

- [ ] 结构化事件可作为可选能力。

  对高频事件场景，六要素结构化可能不是每次都必须。可以配置：

  ```text
  structure_event_enabled: true | false
  ```

- [ ] 对空价值事件做更早过滤。

  在进入 LLM 前增加规则过滤，例如内容过短、纯噪声、重复事件等。

- [ ] 给 prompt 版本化。

  记忆抽取 prompt、相关性 prompt、profile prompt 应有版本号，方便回溯某条记忆由哪个规则生成。

- [ ] 抽取结果增加 `operation_hint`。

  LLM 可以给出：

  ```text
  create | reinforce | update | ignore
  ```

  后续 matcher/reconciler 仍做最终判断，但这个 hint 能提高解释性。

## P2：检索体验优化

- [ ] 检索结果默认返回轻量结构。

  普通调用只返回：

  - memory id
  - content
  - score
  - confidence
  - category
  - tags

- [ ] evidence/history 按需加载。

  `include_evidence` 和 `include_history` 可能导致额外查询，默认关闭是合理的。

- [ ] 检索 score 解释可选返回。

  高级模式可以返回：

  - vector similarity
  - fulltext score
  - importance contribution
  - recency contribution
  - hit count contribution

- [ ] 明确 global memory 排序策略。

  global memory 是否天然加权，需要配置化。否则长期偏好可能压过当前 scope 的短期事实。

- [ ] 支持按 category/tags 的高级检索入口。

  可以保留在 advanced API，不放入普通记忆调用。

## P2：生命周期治理增强

- [ ] lifecycle 任务按系统维度执行。

  建议接口：

  ```text
  POST /api/v1/systems/{profile_id}/admin/lifecycle/run
  POST /api/v1/systems/{profile_id}/admin/decay/update
  POST /api/v1/systems/{profile_id}/admin/promotion/run
  ```

- [x] global promotion 规则按 `profile_id` 隔离。

  不同系统的 scope diversity、reinforcement count 不能互相影响。

- [ ] promotion 需要可解释结果。

  返回：

  - promoted memory ids
  - skipped candidates
  - reason
  - criteria snapshot

- [ ] archive 前支持 dry run。

  淘汰任务应先支持只看候选，不实际更新状态。

- [ ] 提供 embedding rebuild 任务。

  当 embedding provider 更新、Qdrant 写入失败、collection 重建时，需要：

  ```text
  POST /api/v1/systems/{profile_id}/admin/embeddings/rebuild
  ```

## P2：可观测性和审计

- [ ] 为每次事件处理生成 `processing_trace_id`。

  同步和异步接口都应返回这个 ID。

- [ ] 记录阶段耗时。

  至少包括：

  - profile load
  - context memory load
  - relevance check
  - structure event
  - memory extraction
  - matching
  - reconciliation
  - embedding
  - qdrant upsert
  - database writes

- [ ] 记录 LLM 调用元数据。

  包括：

  - provider
  - model
  - prompt type
  - prompt version
  - latency
  - token usage
  - error

- [ ] 记录 embedding 调用元数据。

  包括：

  - provider
  - model
  - dimension
  - latency
  - error

- [ ] health check 做真实依赖检查。

  当前 health 里还有 TODO。建议检查：

  - PostgreSQL
  - Qdrant
  - LLM provider
  - embedding provider

## P3：开发体验和文档

- [ ] 增加 README 快速开始。

  应包含：

  - 创建 system profile
  - 写入事件
  - 检索记忆
  - 查看事件证据链
  - 查看版本历史

- [ ] 增加核心概念文档。

  重点解释：

  - `profile_id`
  - `owner_id`
  - `scope_id`
  - `global memory`
  - `event`
  - `memory`
  - `reinforce`
  - `supersede`

- [ ] 增加调用示例。

  至少提供 curl 示例：

  - 初始化系统画像
  - 事件写入
  - 事件异步写入
  - 检索记忆
  - 查询 evidence
  - 查询 history

- [ ] 增加业务侧接入建议。

  说明业务侧应如何选择：

  - `profile_id`
  - `owner_id`
  - `scope_id`
  - `source`
  - event content 格式

- [ ] 增加迁移说明。

  如果从当前版本迁移到系统命名空间版本，需要说明：

  - profile singleton 如何迁移到多 profile
  - 已有 memories/events 的 `profile_id` 如何处理
  - Qdrant payload 如何重建

## 建议执行顺序

1. 已完成：统一 `profile_id = 系统命名空间` 的代码注释和文档。
2. 已完成：改造 ProfileRepository 和 Profile API，使其支持按 `profile_id` 管理系统画像。
3. 已完成：修复 repository 查询的 `profile_id` 隔离。
4. 已完成：给 Qdrant payload/filter 加上 `profile_id` 和 `owner_id`。
5. 已完成：修复事件处理结果返回不完整和 structured event 未落库问题。
6. 已完成：简化外部 API，改成 `/systems/{profile_id}/...` 路由风格。
7. 已完成：抽出 `EventIngestionService`，让 API handler 变薄。
8. 已完成：合并同步/异步事件处理逻辑。
9. 已完成：事件状态、异步失败落库、失败重试、匹配配置、检索 options、结构化冲突判断、强化元数据、source 枚举和 `MemoryFacade::remember/recall` 简化入口。下一步：优化 LLM 调用成本、可观测性和 lifecycle 任务。

## 最终验收标准

- [x] 不同 `profile_id` 下相同 `owner_id` 的记忆完全隔离。
- [x] 普通调用方只需要理解 `system/profile + owner + scope + content/query`。
- [x] API handler 不再手工组装复杂处理上下文。
- [x] 事件处理结果能准确说明 created、reinforced、superseded、skipped、failed。
- [x] Qdrant 召回不会跨系统或跨 owner 污染候选。
- [x] 六要素结构化事件如果启用，就能持久化和查询。
- [x] lifecycle、promotion、decay、eviction 都按 `profile_id` 隔离执行。
- [x] 文档能让业务侧明确知道：用户和记忆的关联由业务定义，框架只负责在系统命名空间内治理记忆。

