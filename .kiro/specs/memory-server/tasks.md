# Implementation Plan: Memory Server

## Overview

基于 Rust 实现 Memory Server，采用增量开发方式，从核心数据模型开始，逐步构建 API 层、服务层和部署配置。

## Tasks

- [x] 1. 项目初始化和基础设施
  - [x] 1.1 创建 Rust 项目结构和 Cargo.toml 依赖配置
    - 配置 axum, tokio, sqlx, serde, uuid, chrono, tower, tracing 等依赖
    - 设置 workspace 结构
    - _Requirements: 6.1_

  - [x] 1.2 创建配置模块和 YAML 加载
    - 实现 config/mod.rs 配置结构体
    - 支持环境变量覆盖
    - _Requirements: 7.1, 7.2, 7.4_

  - [x] 1.3 创建错误处理模块
    - 定义 ErrorResponse 结构
    - 实现错误码枚举和转换
    - _Requirements: 2.7_

- [x] 2. Domain 层实现
  - [x] 2.1 实现核心枚举类型 (Layer, ScopeType, Status)
    - 定义 domain/layer.rs, scope.rs, status.rs
    - 实现 sqlx 类型映射
    - _Requirements: 1.1_

  - [x] 2.2 实现 Memory 实体和验证逻辑
    - 定义 Memory 结构体（包含 event_source, event_time 字段）
    - 实现创建验证（禁止 long-term 直接创建）
    - _Requirements: 1.1, 1.2, 1.3, 1.5, 1.6_

  - [ ]* 2.3 编写 Property Test: Long-Term Layer Rejection
    - **Property 2: Long-Term Layer Rejection**
    - **Validates: Requirements 1.5**

  - [ ]* 2.4 编写 Property Test: Event Source Traceability
    - **Property 13: Event Source Traceability**
    - **Validates: Requirements 1.2, 1.3, 1.6**

- [x] 3. Repository 层实现
  - [x] 3.1 创建 PostgreSQL 数据库迁移脚本
    - 实现 migrations/001_init.sql
    - 包含所有表和索引定义（包括 event_source 索引）
    - _Requirements: 6.2_

  - [x] 3.2 实现 MemoryRepository
    - CRUD 操作（支持 event_source, event_time 字段）
    - 支持三种更新模式 (append, merge, supersede)
    - _Requirements: 1.1, 1.4, 2.1, 2.2, 2.3, 2.4, 2.5, 2.6_

  - [ ]* 3.3 编写 Property Test: Memory CRUD Round-Trip
    - **Property 1: Memory CRUD Round-Trip**
    - **Validates: Requirements 1.1, 2.1**

  - [ ]* 3.4 编写 Property Test: Update Mode Correctness
    - **Property 3: Update Mode Correctness**
    - **Validates: Requirements 2.2, 2.3, 2.4**

  - [x] 3.5 实现 AuditRepository
    - 审计日志写入
    - 审计日志查询
    - _Requirements: 8.1, 8.2, 8.3, 8.4_

  - [ ]* 3.6 编写 Property Test: Audit Log Completeness
    - **Property 11: Audit Log Completeness**
    - **Validates: Requirements 4.5, 8.1, 8.2**

- [ ] 4. Checkpoint - 基础层验证
  - 确保所有测试通过，如有问题请询问用户

- [x] 5. Embedding 模块实现
  - [x] 5.1 定义 EmbeddingProvider trait
    - 抽象 embedding 接口
    - 支持多 provider
    - _Requirements: 5.5, 10.1_

  - [x] 5.2 实现 Openai Embedding Provider
    - HTTP 客户端调用
    - 重试逻辑 (3次指数退避)
    - _Requirements: 5.1, 5.3, 5.4_

  - [x] 5.3 实现 Local Embedding Provider
    - 兼容 Openai API 格式
    - _Requirements: 10.8_

  - [x] 5.4 实现 ConfigRepository (Provider 配置持久化)
    - Provider CRUD
    - 默认 provider 管理
    - _Requirements: 10.2, 10.3, 11.6_

- [x] 6. Service 层实现
  - [x] 6.1 实现 MemoryGuard 服务
    - 写入验证
    - 层级约束检查
    - 更新模式处理
    - _Requirements: 1.5, 2.7_

  - [ ]* 6.2 编写 Property Test: Invalid Input Rejection
    - **Property 5: Invalid Input Rejection**
    - **Validates: Requirements 2.7**

  - [x] 6.3 实现 RetrievalEngine 服务
    - PostgreSQL 结构过滤（支持 event_source_prefix 过滤）
    - Qdrant 向量召回
    - 综合打分排序
    - Hit count 更新
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8_

  - [ ]* 6.4 编写 Property Test: Retrieval Result Ordering
    - **Property 6: Retrieval Result Ordering**
    - **Validates: Requirements 3.4**

  - [ ]* 6.5 编写 Property Test: Hit Count Increment
    - **Property 7: Hit Count Increment**
    - **Validates: Requirements 3.5**

  - [ ]* 6.6 编写 Property Test: Status-Based Exclusion
    - **Property 8: Status-Based Exclusion**
    - **Validates: Requirements 3.6**

  - [ ]* 6.7 编写 Property Test: Event Source Prefix Filtering
    - **Property 14: Event Source Prefix Filtering**
    - **Validates: Requirements 3.8**

  - [x] 6.8 实现 LifecycleManager 服务
    - TTL 过期处理
    - Cooldown 状态转换
    - 状态变更审计
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5_

  - [ ]* 6.9 编写 Property Test: Cooldown Recovery
    - **Property 10: Cooldown Recovery**
    - **Validates: Requirements 4.3**

  - [x] 6.10 实现 ConfigCenter 服务
    - Provider 管理 API
    - Rate limiting
    - 热更新支持
    - _Requirements: 10.4, 10.5, 10.6, 10.7, 11.1, 11.2, 11.3, 11.4, 11.5, 11.7_

- [ ] 7. Checkpoint - 服务层验证
  - 确保所有测试通过，如有问题请询问用户

- [x] 8. API 层实现
  - [x] 8.1 实现 Memory CRUD API
    - POST /api/v1/memories（支持 event_source, event_time, importance 参数）
    - GET /api/v1/memories/{id}
    - PUT /api/v1/memories/{id}
    - DELETE /api/v1/memories/{id}
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7_

  - [x] 8.2 实现 Retrieval API
    - POST /api/v1/memories/retrieve（支持 event_source_prefix 过滤）
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8_

  - [x] 8.3 实现 Config API
    - GET/POST/PUT /api/v1/config/providers
    - PUT /api/v1/config/default-provider
    - _Requirements: 11.1, 11.2, 11.3, 11.4, 11.5_

  - [x] 8.4 实现 Audit API
    - GET /api/v1/audit
    - _Requirements: 8.3_

  - [x] 8.5 实现 Health & Metrics API
    - GET /health
    - GET /metrics
    - _Requirements: 9.1, 9.2, 9.3, 9.4, 9.5_

- [x] 9. 应用入口和启动逻辑
  - [x] 9.1 实现 main.rs 启动流程
    - 配置加载
    - 数据库连接池初始化
    - Qdrant 客户端初始化
    - HTTP 服务器启动
    - _Requirements: 6.1, 7.1, 7.3_

  - [x] 9.2 实现数据库迁移自动执行
    - 启动时检查并执行迁移
    - _Requirements: 6.2_

- [x] 10. Docker 部署配置
  - [x] 10.1 创建 Dockerfile
    - 多阶段构建
    - 最小化镜像
    - _Requirements: 6.1_

  - [x] 10.2 创建 docker-compose.yml
    - Memory Server, PostgreSQL, Qdrant 服务定义
    - 健康检查配置
    - Volume 持久化
    - _Requirements: 6.1, 6.3, 6.4, 6.5, 6.6_

  - [x] 10.3 创建示例配置文件
    - config/config.yaml
    - .env.example
    - _Requirements: 7.1, 7.2_

- [ ] 11. Final Checkpoint - 完整验证
  - 确保所有测试通过
  - 验证 docker-compose up 可正常启动
  - 如有问题请询问用户

- [x] 12. 全文检索支持
  - [x] 12.1 更新数据库迁移脚本
    - 添加 content_tsv TSVECTOR 列
    - 创建 GIN 索引
    - 添加自动更新触发器
    - _Requirements: 13.1, 13.2_

  - [x] 12.2 更新 MemoryRepository 支持全文检索
    - 实现 tsvector 自动更新
    - 实现 tsquery 检索方法
    - 支持高亮返回
    - _Requirements: 13.3, 13.4, 13.7_

  - [x] 12.3 更新 RetrievalEngine 集成全文检索
    - 结构过滤 + 全文检索 + 向量召回三阶段
    - 综合打分（加入 text_match 权重）
    - _Requirements: 13.4, 13.5, 13.8_

  - [ ]* 12.4 编写 Property Test: Full-Text Search Relevance
    - **Property 15: Full-Text Search Relevance**
    - **Validates: Requirements 13.3, 13.4**

- [x] 13. LLM 记忆处理管道
  - [x] 13.1 定义 LLM Provider trait
    - 抽象 LLM 调用接口
    - 支持多 provider (Openai, Azure, Local)
    - _Requirements: 12.8_

  - [x] 13.2 实现 Openai LLM Provider
    - Chat Completion API 调用
    - 重试逻辑
    - _Requirements: 12.8_

  - [x] 13.3 实现 Local LLM Provider
    - 兼容 Openai API 格式 (Ollama 等)
    - _Requirements: 12.8_

  - [x] 13.4 实现 MemoryProcessor 服务
    - 记忆压缩提纯
    - 自动分类
    - 关键词/标签提取
    - _Requirements: 12.2, 12.3, 12.6_

  - [x] 13.5 更新 MemoryGuard 集成 LLM 处理
    - 根据 process_with_llm 开关决定是否处理
    - 存储 raw_content 和 processed content
    - 处理失败时 fallback 到原始内容
    - _Requirements: 12.1, 12.4, 12.5, 12.7, 12.10_

  - [ ]* 13.6 编写 Property Test: LLM Processing Preservation
    - **Property 16: LLM Processing Preservation**
    - **Validates: Requirements 12.4**

  - [ ]* 13.7 编写 Property Test: Category Classification Consistency
    - **Property 17: Category Classification Consistency**
    - **Validates: Requirements 12.3**

  - [ ]* 13.8 编写 Property Test: Processing Fallback
    - **Property 18: Processing Fallback**
    - **Validates: Requirements 12.7**

- [x] 14. Domain 层扩展
  - [x] 14.1 添加 MemoryCategory 枚举
    - user_preference, behavior_pattern, business_rule, factual_knowledge, other
    - sqlx 类型映射
    - _Requirements: 12.3_

  - [x] 14.2 添加 ProcessingStatus 枚举
    - pending, completed, failed, skipped
    - sqlx 类型映射
    - _Requirements: 12.1_

  - [x] 14.3 更新 Memory 实体
    - 添加 raw_content, category, tags, processing_status, llm_provider 字段
    - _Requirements: 12.4, 12.6_

- [x] 15. 更新 API 层支持新功能
  - [x] 15.1 更新 Memory CRUD API
    - 支持 process_with_llm, llm_provider 参数
    - 返回 category, tags, processing_status
    - _Requirements: 12.1, 12.4_

  - [x] 15.2 更新 Retrieval API
    - 支持 categories, tags 过滤
    - 支持 use_fulltext, fulltext_weight 参数
    - 返回 text_match_score, highlights
    - _Requirements: 3.10, 13.4, 13.7_

  - [ ]* 15.3 编写 Property Test: Category Filter Correctness
    - **Property 19: Category Filter Correctness**
    - **Validates: Requirements 3.10**

- [x] 16. LLM Provider 配置管理
  - [x] 16.1 创建 LlmProviderRepository
    - LLM Provider CRUD
    - 默认 provider 管理
    - Prompt 模板管理
    - _Requirements: 12.8, 12.9_

  - [x] 16.2 更新 ConfigCenter 支持 LLM Provider
    - LLM Provider 热更新
    - Rate limiting
    - _Requirements: 12.8_

- [ ] 17. Final Checkpoint - 完整功能验证
  - 确保所有测试通过
  - 验证全文检索功能
  - 验证 LLM 处理管道
  - 验证 docker-compose up 可正常启动
  - 如有问题请询问用户

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- Each task references specific requirements for traceability
- Checkpoints ensure incremental validation
- Property tests validate universal correctness properties
- Unit tests validate specific examples and edge cases
