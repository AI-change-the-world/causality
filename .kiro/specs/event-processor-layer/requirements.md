# Requirements Document

## Introduction

Event Processor Layer 是对现有 Memory Framework 的增强，提供从原始事件（Event）提纯为结构化记忆（Memory）的能力。

**核心设计原则：**
- **Event 是 Memory 创建的一种方式，不是独立实体**
- **两条创建路径**：
  - **直接创建**：用户自己负责所有字段（content, category, tags, importance 等）
  - **事件提取**：LLM 负责从原始事件中提取和填充字段
- **不限定事件类型**：LLM 自己理解事件内容，不需要用户指定类型

**核心概念：**
- **Event（事件）**：用户原始行为/输入，100% 事实（点击、对话、操作）
- **Memory（记忆）**：提纯后的结构化记忆，90% 事实 + 10% 推断

## Glossary

- **Event**: 用户原始行为/输入，明确的事实（点击事件、对话历史、操作记录）
- **Memory**: 提纯后的结构化记忆，存储在数据库中
- **Memory_Processor**: 记忆处理器，负责事件提取、摘要、查询增强
- **Memory_Guard**: 记忆守卫，负责创建、更新、检索记忆
- **Inference_Type**: 推断类型（fact/preference/pattern/rule）
- **Processing_Mode**: 处理模式（auto/assisted/manual）

## Requirements

### Requirement 1: 直接创建 Memory（用户负责）

**User Story:** As a developer, I want to create memories directly with all fields specified, so that I have full control over the memory content.

#### Acceptance Criteria

1. WHEN a user calls the create memory API, THE system SHALL accept user-specified content, category, tags, and importance
2. WHEN category is not specified, THE system SHALL store it as NULL (not auto-classify)
3. WHEN tags are not specified, THE system SHALL store empty array (not auto-extract)
4. THE system SHALL NOT use LLM for direct memory creation
5. THE system SHALL validate all user-provided fields according to existing rules

### Requirement 2: 事件输入与摘要

**User Story:** As a developer, I want to submit events (conversations, actions, etc.) and have them processed into memories, so that I don't need to manually extract insights.

#### Acceptance Criteria

1. WHEN a user submits event content, THE Memory_Processor SHALL accept it without requiring event type specification
2. WHEN event content exceeds 4000 characters, THE Memory_Processor SHALL summarize it before extraction
3. WHEN summarizing, THE Memory_Processor SHALL preserve key facts and user preferences
4. IF the event content is already concise, THEN THE Memory_Processor SHALL process it directly without summarization
5. THE Memory_Processor SHALL accept optional context to help LLM understand the event better

### Requirement 3: 事件到记忆的提取

**User Story:** As a developer, I want the system to extract meaningful memories from events, so that I can build a knowledge base of user preferences and patterns.

#### Acceptance Criteria

1. WHEN processing an event, THE Memory_Processor SHALL extract both explicit facts and implicit preferences
2. WHEN extracting memories, THE Memory_Processor SHALL assign inference_type: "fact", "preference", "pattern", or "rule"
3. WHEN a single event contains multiple insights, THE Memory_Processor SHALL generate multiple Memory entries
4. THE Memory_Processor SHALL assign confidence scores: facts >= 0.9, preferences/patterns in [0.6, 0.8]
5. THE Memory_Processor SHALL provide reasoning for each extracted memory
6. THE Memory_Processor SHALL auto-classify category and extract tags for each memory

### Requirement 4: 提取结果结构

**User Story:** As a developer, I want extracted memories to have a clear structure, so that I can understand what was extracted and why.

#### Acceptance Criteria

1. THE extraction result SHALL contain: event_summary, extracted_memories[], confidence
2. WHEN returning extracted memories, THE result SHALL include: content, inference_type, confidence, category, tags, importance, reasoning
3. THE confidence scores SHALL be in range [0.0, 1.0]
4. THE reasoning field SHALL explain why this memory was extracted

### Requirement 5: 查询增强

**User Story:** As a developer, I want my queries to be enhanced with semantic understanding, so that I get more relevant memory results.

#### Acceptance Criteria

1. WHEN a user enables query enhancement, THE Memory_Processor SHALL expand the query with semantic synonyms
2. THE enhanced query result SHALL contain both original_query and enhanced_query
3. IF query enhancement is disabled, THEN THE system SHALL use the original query directly
4. WHEN enhancing queries, THE Memory_Processor SHALL respect scope and scene constraints

### Requirement 6: 处理模式

**User Story:** As a developer, I want to control how much automation the system uses, so that I can balance between convenience and control.

#### Acceptance Criteria

1. THE system SHALL support three modes: "auto", "assisted", "manual"
2. WHEN in "auto" mode, THE system SHALL extract and create memories without confirmation
3. WHEN in "assisted" mode, THE system SHALL return proposed memories for user approval (default)
4. WHEN in "manual" mode, THE system SHALL only summarize events without extraction
5. WHEN mode is not specified, THE system SHALL use "assisted" as default

### Requirement 7: 可追溯性

**User Story:** As a developer, I want to know which memories were extracted from events vs created directly, so that I can audit the system.

#### Acceptance Criteria

1. WHEN creating a memory from event extraction, THE system SHALL store inference_type, inference_confidence, and inference_reasoning
2. WHEN a memory is created directly by user, THE inference fields SHALL be NULL
3. THE system SHALL be able to distinguish between user-created and LLM-extracted memories

### Requirement 8: API 设计

**User Story:** As a developer, I want clean APIs for both direct creation and event extraction, so that I can easily integrate them.

#### Acceptance Criteria

1. THE system SHALL expose POST `/api/v1/memories` for direct memory creation (user provides all fields)
2. THE system SHALL expose POST `/api/v1/memories/from-event` for event extraction (LLM extracts fields)
3. THE system SHALL expose POST `/api/v1/memories/retrieve` with optional `enhance_query` parameter
4. THE direct creation API SHALL NOT have `process_with_llm` parameter
5. THE event extraction API SHALL accept: content, context, mode, scope_type, scope_id, scene
