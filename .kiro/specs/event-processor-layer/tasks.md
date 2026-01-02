# Implementation Plan: Event Processor Layer

## Overview

基于设计文档扩展现有 Memory Server，实现从原始事件（Event）提纯为结构化记忆（Memory）的能力。采用增量开发方式，从 Domain 层类型定义开始，逐步构建 Repository、Service 和 API 层。

## Tasks

- [x] 1. Domain 层扩展 - 添加推断类型和处理模式
  - [x] 1.1 添加 InferenceType 枚举
    - 定义 Fact, Preference, Pattern, Rule 四种推断类型
    - 实现 sqlx 类型映射、serde 序列化、Display trait
    - _Requirements: 3.2, 7.1_

  - [x] 1.2 添加 ProcessingMode 枚举
    - 定义 Auto, Assisted, Manual 三种处理模式
    - Assisted 为默认值
    - 实现 sqlx 类型映射、serde 序列化、Display trait
    - _Requirements: 6.1, 6.5_

  - [x] 1.3 扩展 Memory 结构体添加推断字段
    - 添加 inference_type: Option<InferenceType>
    - 添加 inference_confidence: Option<f32>
    - 添加 inference_reasoning: Option<String>
    - _Requirements: 7.1, 7.2_

  - [x] 1.4 添加 ExtractedMemory 结构体
    - 包含 content, inference_type, confidence, category, tags, importance, reasoning 字段
    - 实现 serde 序列化
    - _Requirements: 4.1, 4.2_

  - [ ]* 1.5 编写单元测试验证新类型
    - 测试枚举序列化/反序列化
    - 测试默认值
    - _Requirements: 3.2, 6.1_

- [x] 2. 数据库迁移 - 扩展 memories 表
  - [x] 2.1 创建迁移脚本 002_add_inference_fields.sql
    - 添加 inference_type VARCHAR(50) 字段
    - 添加 inference_confidence REAL 字段
    - 添加 inference_reasoning TEXT 字段
    - 使用 IF NOT EXISTS 确保可重复执行
    - _Requirements: 7.1_

- [x] 3. Repository 层扩展 - 支持推断字段
  - [x] 3.1 更新 MemoryRepository create() 方法
    - 支持写入 inference_type, inference_confidence, inference_reasoning 字段
    - _Requirements: 7.1_

  - [x] 3.2 更新 MemoryRepository get_by_id() 方法
    - 返回推断相关字段
    - _Requirements: 7.1, 7.3_

  - [x] 3.3 添加 CreateMemoryFromEventInput 结构体
    - 包含从事件创建记忆所需的所有字段
    - _Requirements: 7.1_

  - [ ]* 3.4 编写 Property Test: Inference Fields Round-Trip
    - **Property 12: Inference Traceability**
    - **Validates: Requirements 7.1**

- [ ] 4. Checkpoint - 基础层验证
  - 确保所有测试通过，如有问题请询问用户

- [x] 5. 错误处理扩展
  - [x] 5.1 添加事件处理相关错误类型
    - 添加 EventContentTooShort 错误
    - 添加 NoMemoriesExtracted 错误
    - 添加 QueryEnhancementFailed 错误
    - 实现正确的 HTTP 状态码映射
    - _Requirements: 2.2_

- [x] 6. MemoryProcessor 扩展 - 事件提取功能
  - [x] 6.1 添加 ExtractFromEventRequest 结构体
    - 包含 content: String 和 context: Option<String>
    - _Requirements: 2.1, 2.5_

  - [x] 6.2 添加 ExtractFromEventResult 结构体
    - 包含 event_summary, extracted_memories, confidence
    - _Requirements: 4.1_

  - [x] 6.3 实现 needs_summary() 辅助方法
    - 判断内容是否超过 4000 字符
    - _Requirements: 2.2, 2.4_

  - [x] 6.4 实现 summarize_conversation() 方法
    - 调用 LLM 进行长文本摘要
    - 保留关键事实和用户偏好
    - _Requirements: 2.2, 2.3_

  - [x] 6.5 添加事件提取的 prompt 模板
    - 构建 LLM 提取 prompt
    - 不限定事件类型，让 LLM 自己理解
    - _Requirements: 2.1, 3.1_

  - [x] 6.6 实现 extract_from_event() 方法
    - 长内容先摘要
    - 调用 LLM 提取记忆
    - 解析并返回 ExtractedMemory 列表
    - _Requirements: 2.1, 3.1, 3.2, 3.3, 3.4, 3.5, 3.6_

  - [x] 6.7 添加 EnhancedQuery 结构体
    - 包含 original_query 和 enhanced_query
    - _Requirements: 5.2_

  - [x] 6.8 实现 enhance_query() 方法
    - 调用 LLM 扩展查询
    - 添加语义同义词
    - _Requirements: 5.1, 5.4_

  - [ ]* 6.9 编写 Property Test: Long Content Compression
    - **Property 2: Long Content Compression**
    - **Validates: Requirements 2.2, 2.3**

  - [ ]* 6.10 编写 Property Test: Short Content Passthrough
    - **Property 3: Short Content Passthrough**
    - **Validates: Requirements 2.4**

  - [ ]* 6.11 编写 Property Test: Confidence Score Validity
    - **Property 4: Confidence Score Validity**
    - **Validates: Requirements 3.4, 3.5**

- [ ] 7. Checkpoint - MemoryProcessor 验证
  - 确保所有测试通过，如有问题请询问用户

- [x] 8. MemoryGuard 扩展 - 事件处理入口
  - [x] 8.1 添加 CreateFromEventRequest 结构体
    - 包含 content, context, scope_type, scope_id, scene, mode
    - _Requirements: 8.5_

  - [x] 8.2 添加 CreateFromEventResult 结构体
    - 包含 event_summary, extracted_memories, created_memory_ids
    - _Requirements: 4.1_

  - [x] 8.3 实现 create_from_event() 方法
    - 调用 MemoryProcessor.extract_from_event()
    - 根据 mode 决定是否创建记忆
    - _Requirements: 6.1, 6.2, 6.3, 6.4_

  - [x] 8.4 实现 retrieve_with_enhancement() 方法
    - 根据 enhance 参数决定是否增强查询
    - 调用现有 retrieve() 方法
    - _Requirements: 5.1, 5.3_

  - [ ]* 8.5 编写 Property Test: Processing Mode - Auto Creates Memories
    - **Property 6: Processing Mode - Auto Creates Memories**
    - **Validates: Requirements 6.2**

  - [ ]* 8.6 编写 Property Test: Processing Mode - Assisted Returns Proposals
    - **Property 7: Processing Mode - Assisted Returns Proposals**
    - **Validates: Requirements 6.3**

  - [ ]* 8.7 编写 Property Test: Default Mode is Assisted
    - **Property 9: Default Mode is Assisted**
    - **Validates: Requirements 6.5**

- [ ] 9. Checkpoint - Service 层验证
  - 确保所有测试通过，如有问题请询问用户

- [x] 10. API 层扩展 - 事件处理端点
  - [x] 10.1 添加 CreateFromEventApiRequest 结构体
    - 包含 content, context, mode, scope_type, scope_id, scene
    - 添加 OpenAPI 文档注解
    - _Requirements: 8.2, 8.5_

  - [x] 10.2 添加 CreateFromEventApiResponse 结构体
    - 包含 event_summary, extracted_memories, created_memory_ids
    - 添加 OpenAPI 文档注解
    - _Requirements: 4.1_

  - [x] 10.3 添加 ExtractedMemoryResponse 结构体
    - 包含 content, inference_type, confidence, category, tags, importance, reasoning
    - 添加 OpenAPI 文档注解
    - _Requirements: 4.2_

  - [x] 10.4 实现 POST /api/v1/memories/from-event 端点
    - 调用 MemoryGuard.create_from_event()
    - 返回 CreateFromEventApiResponse
    - _Requirements: 8.2_

  - [x] 10.5 扩展 RetrieveApiRequest 添加 enhance_query 参数
    - 默认为 false
    - _Requirements: 8.3_

  - [x] 10.6 更新 POST /api/v1/memories/retrieve 端点
    - 支持 enhance_query 参数
    - 调用 retrieve_with_enhancement()
    - _Requirements: 5.1, 5.3, 8.3_

  - [x] 10.7 从 CreateMemoryApiRequest 移除 process_with_llm 参数
    - 直接创建不使用 LLM
    - _Requirements: 1.4, 8.4_

  - [x] 10.8 更新 API 路由注册
    - 注册新端点到 router
    - _Requirements: 8.2_

- [ ] 11. Checkpoint - API 层验证
  - 确保所有测试通过，如有问题请询问用户

- [ ] 12. 集成测试
  - [ ]* 12.1 测试事件提取完整流程
    - 提交事件 → 提取记忆 → 验证结果
    - _Requirements: 2.1, 3.1_

  - [ ]* 12.2 测试三种处理模式的行为
    - Auto: 自动创建记忆
    - Assisted: 返回建议不创建
    - Manual: 仅摘要
    - _Requirements: 6.2, 6.3, 6.4_

  - [ ]* 12.3 测试查询增强功能
    - 启用增强 vs 禁用增强
    - _Requirements: 5.1, 5.3_

  - [ ]* 12.4 测试推断字段的持久化和查询
    - 创建带推断字段的记忆
    - 查询并验证字段值
    - _Requirements: 7.1, 7.3_

  - [ ]* 12.5 测试错误处理场景
    - 内容过短
    - 无法提取记忆
    - _Requirements: 2.2_

- [x] 13. 文档更新
  - [x] 13.1 更新 README 添加事件处理功能说明
    - 描述两种创建路径
    - 添加 API 使用示例
    - _Requirements: 8.1, 8.2_

- [ ] 14. Final Checkpoint - 完整功能验证
  - 确保所有测试通过
  - 验证事件提取功能
  - 验证查询增强功能
  - 如有问题请询问用户

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- Each task references specific requirements for traceability
- Checkpoints ensure incremental validation
- Property tests validate universal correctness properties
- Unit tests validate specific examples and edge cases
- 所有代码使用 Rust 语言
- 遵循现有代码风格和命名规范
- 使用 `async-openai` 库进行 LLM 调用
- 使用 `serde` 进行 JSON 序列化/反序列化
- 使用 `utoipa` 生成 OpenAPI 文档
- 测试使用 `#[tokio::test]` 进行异步测试
