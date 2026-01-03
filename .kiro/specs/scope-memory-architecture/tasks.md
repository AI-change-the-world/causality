# Implementation Plan: Scope-Memory Architecture Refactoring

## Overview

本实现计划将 Memory Server 重构为 Event-driven 架构，实现 Memory 版本链、LFU 淘汰机制和简化的数据模型。实现顺序遵循依赖关系：先数据模型，再核心逻辑，最后 API 层。

## Tasks

- [x] 1. 数据库迁移和数据模型更新
  - [x] 1.1 创建数据库迁移文件
    - 合并到 `migrations/001_init.sql`
    - 删除 `scope_type`, `layer`, `scene`, `ttl_seconds`, `expires_at` 字段
    - 添加版本链字段: `root_memory_id`, `version_number`, `is_current_version`, `supersedes`, `superseded_by`
    - 添加生命周期字段: `is_global`, `decay_score`
    - 修改 `scope_id` 为可空
    - 创建版本链索引: `idx_memories_root`, `idx_memories_current`
    - 更新 `status` 枚举: 添加 `cooldown`, `candidate`, `superseded`
    - _Requirements: 2.3, 2.4, 6.1_

  - [x] 1.2 更新 Memory domain 实体
    - 修改 `src/domain/memory.rs`
    - 删除 `ScopeType`, `Layer`, `scene` 相关字段
    - 添加版本链字段和生命周期字段
    - 更新 `CreateMemoryInput` 和 `CreateMemoryFromEventInput`
    - 添加 `CreateSupersedingMemoryInput` 用于创建新版本
    - 添加 `Memory::new_superseding()` 和 `Memory::mark_superseded()` 方法
    - 更新 `Status` 枚举添加 `Cooldown`, `Candidate`, `Superseded`
    - _Requirements: 2.2, 2.3, 2.4_

  - [x] 1.3 更新 Event domain 实体
    - 修改 `src/domain/event.rs`
    - 删除 `ScopeType` 依赖
    - 修改 `scope_id` 为 `Option<String>`
    - 添加 `source` 字段
    - 删除 `event_type` 和 `scene` 字段
    - 删除 `ContradictedBy` 关系类型 (冲突通过版本链处理)
    - 删除 `EventMemoryRelation.similarity_score` 字段
    - _Requirements: 1.1, 1.2_

  - [x] 1.4 删除废弃的 domain 文件
    - 删除 `src/domain/layer.rs`
    - 删除 `src/domain/scope.rs`
    - 更新 `src/domain/mod.rs` 导出
    - _Requirements: 2.2_

- [x] 2. Repository 层更新
  - [x] 2.1 更新 MemoryRepository
    - 修改 `src/repository/memory_repo.rs`
    - 更新所有 SQL 查询移除旧字段
    - 添加版本链查询方法: `find_by_root_memory_id`, `get_version_history`
    - 添加 `update_superseded` 方法
    - 添加 `find_current_version` 方法
    - _Requirements: 7.1, 7.2, 7.3_

  - [x] 2.2 更新 EventRepository
    - 修改 `src/repository/event_repo.rs`
    - 更新 SQL 查询适配新字段
    - 添加 `source` 字段支持
    - _Requirements: 1.1, 1.2_

  - [x] 2.3 更新 EventMemoryRelationRepository
    - 修改关系查询方法
    - 确保只使用 `created_from` 和 `reinforced_by` 类型
    - _Requirements: 2.5, 3.3_

  - [ ]* 2.4 编写 Repository 集成测试
    - 测试版本链查询
    - 测试关系创建
    - _Requirements: 7.1, 7.4_

- [x] 3. 实现核心服务组件
  - [x] 3.1 实现 MemoryMatcher
    - 创建 `src/service/memory_matcher.rs`
    - 实现 `find_similar` 方法
    - 只搜索 `is_current_version = true` 的 Memory
    - 搜索范围包含 `scope_id` 匹配或 `is_global = true`
    - _Requirements: 3.1, 3.2_

  - [x] 3.2 实现 MemoryReconciler
    - 创建 `src/service/memory_reconciler.rs`
    - 实现 `reconcile` 方法
    - 处理三种情况: CreateNew, Reinforce, Supersede
    - Supersede 时创建新版本并更新旧版本
    - _Requirements: 3.3, 3.4_

  - [ ]* 3.3 编写 MemoryReconciler 属性测试
    - **Property 3: Version Chain Integrity**
    - **Validates: Requirements 2.4**

  - [ ]* 3.4 编写 MemoryReconciler 属性测试
    - **Property 4: Single Current Version**
    - **Validates: Requirements 2.4, 7.1**

  - [x] 3.5 实现 DecayCalculator
    - 创建 `src/service/decay_calculator.rs`
    - 实现衰减分数计算公式
    - 实现批量更新方法
    - _Requirements: 6.1, 6.2_

  - [ ]* 3.6 编写 DecayCalculator 属性测试
    - **Property 9: Decay Score Monotonicity**
    - **Validates: Requirements 6.1, 6.2**

  - [x] 3.7 实现 GlobalPromoter
    - 创建 `src/service/global_promoter.rs`
    - 实现 `check_eligibility` 方法
    - 实现 `promote` 方法
    - _Requirements: 5.1, 5.2, 5.3_

  - [ ]* 3.8 编写 GlobalPromoter 属性测试
    - **Property 8: Global Memory Scope**
    - **Validates: Requirements 5.2**

  - [x] 3.9 实现 EvictionManager
    - 创建 `src/service/eviction_manager.rs`
    - 实现状态转换逻辑: Active → Cooldown → Candidate → Archived
    - 实现容量限制检查
    - _Requirements: 6.3, 6.4, 6.5, 6.6, 6.7_

  - [ ]* 3.10 编写 EvictionManager 属性测试
    - **Property 10: Status Transition Validity**
    - **Validates: Requirements 6.3, 6.4, 6.5, 6.6**

- [ ] 4. Checkpoint - 核心服务测试
  - 确保所有核心服务单元测试通过
  - 确保属性测试通过
  - 如有问题请询问用户

- [x] 5. 更新 Event 处理流程
  - [x] 5.1 重构 MemoryGuard
    - 修改 `src/service/memory_guard.rs`
    - 集成 MemoryMatcher 和 MemoryReconciler
    - 实现完整的 Event → Memory 处理流程
    - _Requirements: 2.1, 3.1, 3.2, 3.3, 3.4_

  - [x] 5.2 更新 EventProcessor
    - 修改 LLM 提取逻辑
    - 生成 `category` (层级式) 替代 `scene`
    - 生成 `importance` 和 `confidence`
    - _Requirements: 2.1, 4.1, 4.2_

  - [ ]* 5.3 编写 Event 处理集成测试
    - 测试完整的 Event → Memory 流程
    - 测试 Reinforce 和 Supersede 场景
    - _Requirements: 2.1, 3.3, 3.4_

- [x] 6. 更新检索引擎
  - [x] 6.1 重构 RetrievalEngine
    - 修改 `src/service/retrieval_engine.rs`
    - 默认过滤 `is_current_version = true`
    - 默认排除 `status IN ('superseded', 'archived')`
    - 支持 `scope_id` + `is_global` 联合查询
    - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5, 8.7_

  - [x] 6.2 实现 Category 前缀查询
    - 支持 `category_prefix` 参数
    - 实现 `CategoryQuery` 枚举
    - _Requirements: 4.3, 4.4_

  - [x] 6.3 实现 Evidence 和 History 加载
    - 支持 `include_evidence` 参数
    - 支持 `include_history` 参数
    - _Requirements: 8.6_

  - [ ]* 6.4 编写检索属性测试
    - **Property 11: Retrieval Excludes Non-Current**
    - **Property 12: Scope + Global Retrieval**
    - **Validates: Requirements 8.4, 8.7**

- [ ] 7. Checkpoint - 服务层测试
  - 确保所有服务层测试通过
  - 运行完整的集成测试
  - 如有问题请询问用户

- [x] 8. API 层更新
  - [x] 8.1 更新 Event API
    - 修改 `src/api/event.rs`
    - 更新请求/响应结构
    - 返回 `memories_created` 和 `memories_reinforced` 计数
    - _Requirements: 9.1_

  - [x] 8.2 更新 Memory API
    - 修改 `src/api/memory.rs`
    - 删除 `scope_type`, `layer`, `scene` 相关参数
    - 添加 `category_prefix`, `include_evidence`, `include_history` 参数
    - _Requirements: 9.2_

  - [x] 8.3 添加 Memory History API
    - 添加 `GET /memories/{id}/history` 端点
    - 返回版本历史和关联 Events
    - _Requirements: 9.2_

  - [x] 8.4 添加 Memory Promote API
    - 添加 `POST /memories/{id}/promote` 端点
    - 手动提升为 Global Memory
    - _Requirements: 9.2_

  - [x] 8.5 添加 Admin API
    - 添加 `POST /admin/eviction` 端点
    - 添加 `POST /admin/decay-update` 端点
    - _Requirements: 9.3_

  - [ ]* 8.6 编写 API 集成测试
    - 测试所有新端点
    - 测试错误处理
    - _Requirements: 9.1, 9.2, 9.3_

- [x] 9. 清理和文档
  - [x] 9.1 删除废弃代码
    - 删除 `ScopeType` 枚举
    - 删除 `Layer` 枚举
    - 删除 `scene` 相关代码
    - 删除 TTL 相关代码
    - 更新 `src/domain/mod.rs` 导出

  - [x] 9.2 更新配置文件
    - 更新 `config/config.yaml`
    - 添加 `decay_config` 和 `eviction_config` 配置项

  - [ ]* 9.3 更新 API 文档
    - 更新 OpenAPI/Swagger 文档
    - 记录废弃的端点和参数

- [ ] 10. Final Checkpoint - 完整测试
  - 运行 `cargo test` 确保所有测试通过
  - 运行 `cargo clippy` 确保无警告
  - 如有问题请询问用户

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- 属性测试使用 `proptest` 库
- 数据库迁移需要在测试环境验证后再应用到生产
- 版本链查询使用物化路径方案，O(1) 复杂度
- Event 和 Memory 内容不可变，冲突时创建新版本
