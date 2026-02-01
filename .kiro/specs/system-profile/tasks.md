# Implementation Plan: System Profile

## Overview

实现 SystemProfile 功能，包括数据模型、数据库迁移、Repository、Service、API 层，以及 LLM/Embedding Provider 的环境变量配置简化。

## Tasks

- [x] 1. 数据库表结构更新
  - 修改 `memory-server/migrations/001_init.sql`
  - 添加 system_profile 表定义（含 extraction_prompt TEXT 字段）
  - 添加 structured_events 表定义（六要素 + 两辅助）
  - 删除 embedding_providers 表
  - 删除 llm_providers 表
  - 添加单例约束索引和触发器
  - _Requirements: 1.1-1.11, 5.3, 6.1-6.8_

- [x] 2. Domain 层实现
  - [x] 2.1 创建 SystemProfile 实体
    - 创建 `memory-server/src/domain/profile.rs`
    - 定义 SystemProfile 结构体（含 extraction_prompt 字段）
    - 实现 Serialize/Deserialize
    - 实现验证逻辑 (name 长度限制)
    - _Requirements: 1.1-1.10, 7.1-7.3, 9.1-9.3_
  - [x] 2.2 创建 StructuredEvent 实体
    - 创建 `memory-server/src/domain/structured_event.rs`
    - 定义 StructuredEvent 结构体（六要素 + 两辅助）
    - 实现 Serialize/Deserialize
    - _Requirements: 6.1-6.8_
  - [ ]* 2.3 编写属性测试 - Round-trip Serialization
    - **Property 1: Round-trip Serialization**
    - **Validates: Requirements 1.1-1.10, 9.1-9.3**
  - [ ]* 2.4 编写属性测试 - Name Length Validation
    - **Property 4: Name Length Validation**
    - **Validates: Requirements 1.2**

- [x] 3. Repository 层实现
  - [x] 3.1 创建 ProfileRepository
    - 创建 `memory-server/src/repository/profile_repo.rs`
    - 实现 create, get, update, exists 方法
    - 更新 `memory-server/src/repository/mod.rs` 导出
    - _Requirements: 1.11, 2.2, 3.1, 4.1_
  - [x] 3.2 创建 StructuredEventRepository
    - 创建 `memory-server/src/repository/structured_event_repo.rs`
    - 实现 create, get_by_event_id 方法
    - 更新 `memory-server/src/repository/mod.rs` 导出
    - _Requirements: 6.1-6.8_
  - [ ]* 3.3 编写属性测试 - Singleton Constraint
    - **Property 2: Singleton Constraint**
    - **Validates: Requirements 1.11, 2.3**

- [x] 4. Service 层实现
  - [x] 4.1 创建 ProfileService
    - 创建 `memory-server/src/service/profile_service.rs`
    - 实现 initialize, get, update 方法
    - 实现 LLM 解析逻辑 (parse_description)
    - 更新 `memory-server/src/service/mod.rs` 导出
    - _Requirements: 2.1-2.5, 3.1-3.2, 4.1-4.4_
  - [ ]* 4.2 编写属性测试 - Partial Update Preservation
    - **Property 3: Partial Update Preservation**
    - **Validates: Requirements 4.1, 4.3**

- [x] 5. API 层实现
  - [x] 5.1 创建 Profile API handlers
    - 创建 `memory-server/src/api/profile.rs`
    - 实现 POST /api/v1/system/init
    - 实现 GET /api/v1/system
    - 实现 PUT /api/v1/system
    - 更新 `memory-server/src/api/mod.rs` 导出和路由
    - _Requirements: 2.1-2.5, 3.1-3.2, 4.1-4.4_
  - [ ]* 5.2 编写 API 单元测试
    - 测试请求/响应序列化
    - 测试错误响应格式
    - _Requirements: 2.4, 3.2, 4.4_

- [ ] 6. Checkpoint - 确保所有测试通过
  - 运行 `cargo test --lib`
  - 确保所有测试通过，如有问题请询问用户

- [-] 7. Provider 配置简化
  - [x] 7.1 更新配置模块
    - 修改 `memory-server/src/config/mod.rs`
    - 添加完整的 LlmConfig 结构（provider_type, endpoint, api_key, model）
    - 添加完整的 EmbeddingConfig 结构（provider_type, endpoint, api_key, model, dimension）
    - _Requirements: 5.1, 5.2_
  - [x] 7.2 删除 Provider 数据库相关代码
    - 删除 `memory-server/src/repository/config_repo.rs` 中的 provider 相关方法
    - 删除 `memory-server/src/repository/llm_provider_repo.rs`
    - 删除 `memory-server/src/api/config.rs` 中的 provider 管理 API
    - 更新相关 mod.rs 导出
    - _Requirements: 5.3, 5.4_
  - [x] 7.3 更新 AppState 初始化
    - 修改 `memory-server/src/main.rs`
    - 在启动时从 config.yaml 加载 LLM 和 Embedding 配置
    - 创建全局 LLM provider 实例并存入 AppState
    - 创建全局 Embedding provider 实例并存入 AppState
    - 确保 Qdrant collection 存在（使用 embedding.dimension）
    - 配置无效时启动失败并输出错误信息
    - _Requirements: 5.5, 5.6_

- [x] 8. Event API 简化
  - [x] 8.1 修改 CreateEventApiRequest
    - 修改 `memory-server/src/api/event.rs`
    - 删除 llm_provider 和 embedding_provider 字段
    - 使用 AppState 中的全局 provider
    - _Requirements: 5.5_
  - [x] 8.2 更新 process_event_background 和 create_event
    - 移除动态 provider 创建逻辑
    - 直接使用 AppState 中的 provider 实例
    - _Requirements: 5.5_

- [x] 9. EventHandler 实现
  - [x] 9.1 创建 EventHandler 服务
    - 创建 `memory-server/src/service/event_handler.rs`
    - 实现 structure_event 方法（使用 LLM 解析六要素）
    - 实现 build_extraction_prompt 方法（使用 SystemProfile 的 extraction_strategy）
    - 更新 `memory-server/src/service/mod.rs` 导出
    - _Requirements: 6.9-6.10, 7.1-7.5_
  - [x] 9.2 集成到事件处理流程
    - 修改 `memory-server/src/api/event.rs` 或相关处理逻辑
    - 在处理事件时先调用 EventHandler.structure_event
    - 将 StructuredEvent 传递给 MemoryProcessor
    - _Requirements: 8.1-8.4_
  - [x] 9.3 更新 MemoryProcessor
    - 修改 `memory-server/src/service/memory_processor.rs`
    - 使用 StructuredEvent 的结构化信息提取记忆
    - 在 prompt 中包含六要素信息
    - _Requirements: 8.2_

- [ ] 10. Final Checkpoint
  - 运行 `cargo test --lib`
  - 运行 `cargo check`
  - 确保所有测试通过，如有问题请询问用户

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- Each task references specific requirements for traceability
- Checkpoints ensure incremental validation
- Property tests validate universal correctness properties
- Unit tests validate specific examples and edge cases
