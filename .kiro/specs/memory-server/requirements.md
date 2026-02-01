# Requirements Document

## Introduction

Memory Server 是一个独立的、可嵌入任意 Agent / LLM 应用的上下文记忆治理服务。负责结构化记忆的存储、检索、演化与治理，不负责推理、不绑定模型、不侵入业务。基于 Rust 实现，支持 Docker Compose 一键部署。

## Glossary

- **Memory_Server**: 记忆治理服务，提供 HTTP API 接口
- **Memory**: 结构化上下文/结论/规则，区别于原始对话历史和知识库
- **Layer**: 记忆层级，决定记忆的生命周期（session/task/long-term）
- **Scope**: 记忆归属，决定记忆属于谁（user/org/project/task/session）
- **Scene**: 使用场景，决定记忆在什么情境下被检索（如 work.contract_review）
- **Status**: 记忆状态，决定是否参与检索（active/cooldown/ignored/archived）
- **Event_Source**: 事件来源，记录触发记忆创建的用户行为（自由字符串，如 "button_click:like", "conversation:preference"）
- **Retrieval_Engine**: 检索引擎，负责结构过滤 + 全文检索 + 向量召回 + 综合打分
- **Lifecycle_Manager**: 生命周期管理器，负责状态转换和 TTL 管理
- **Memory_Guard**: 写入守卫，负责验证和约束写入操作
- **Memory_Processor**: 记忆处理器，负责 LLM 驱动的记忆压缩、提纯和分类
- **Raw_Content**: 原始内容，用户提交的未经处理的对话记录或操作日志
- **Processed_Content**: 处理后内容，经过 LLM 压缩提纯后的结构化记忆
- **Memory_Category**: 记忆分类，如 user_preference（用户偏好）、behavior_pattern（行为模式）、business_rule（业务规则）、factual_knowledge（事实知识）

## Requirements

### Requirement 1: 事件驱动的记忆创建

**User Story:** As an Agent developer, I want to create memories based on user events with flexible event metadata, so that I can capture user behaviors with appropriate importance determined by my application logic.

#### Acceptance Criteria

1. WHEN an Agent sends a create memory request with valid layer, scope, scene, content and importance, THE Memory_Server SHALL persist the memory and return a unique memory ID
2. WHEN an Agent sends a create memory request, THE Memory_Server SHALL accept an optional event_source string to record the triggering user behavior
3. WHEN an Agent sends a create memory request, THE Memory_Server SHALL accept an optional event_time timestamp to record when the user behavior occurred
4. THE Memory_Server SHALL allow the Agent to specify importance (0.0-1.0) directly, as the Agent/LLM determines importance based on user behavior context
5. WHEN an Agent sends a create memory request with layer set to "long-term", THE Memory_Server SHALL reject the request and return an error indicating manual confirmation is required
6. THE Memory_Server SHALL persist event_source and event_time with each memory for traceability

### Requirement 2: Memory CRUD 操作

**User Story:** As an Agent developer, I want to read, update and delete memories, so that I can manage context for my LLM application.

#### Acceptance Criteria

1. WHEN an Agent queries a memory by ID, THE Memory_Server SHALL return the complete memory record including event_source, event_time and all metadata
2. WHEN an Agent updates a memory with mode "append", THE Memory_Server SHALL append new content to existing content
3. WHEN an Agent updates a memory with mode "merge", THE Memory_Server SHALL merge new content with existing content preserving both
4. WHEN an Agent updates a memory with mode "supersede", THE Memory_Server SHALL replace existing content with new content
5. WHEN an Agent updates a memory with a new importance value, THE Memory_Server SHALL update the importance field
6. WHEN an Agent deletes a memory, THE Memory_Server SHALL mark the memory as archived rather than physically deleting it
7. IF a create or update request contains invalid scope_type, THEN THE Memory_Server SHALL return a validation error

### Requirement 3: Memory 检索

**User Story:** As an Agent developer, I want to retrieve relevant memories based on context, so that I can provide appropriate context to my LLM.

#### Acceptance Criteria

1. WHEN an Agent sends a retrieval request with scope and scene filters, THE Retrieval_Engine SHALL first filter memories by PostgreSQL structured query
2. WHEN a retrieval request includes a text query, THE Retrieval_Engine SHALL perform PostgreSQL full-text search on memory content using tsvector/tsquery
3. WHEN structured and full-text filtering is complete, THE Retrieval_Engine SHALL perform vector similarity search on the filtered set
4. WHEN vector search is complete, THE Retrieval_Engine SHALL compute a composite score combining text_match, similarity, importance, recency and hit_count
5. THE Retrieval_Engine SHALL return top-K memories sorted by composite score
6. WHEN a memory is returned in retrieval results, THE Retrieval_Engine SHALL increment hit_count and update last_hit_at
7. WHILE a memory has status "ignored" or "archived", THE Retrieval_Engine SHALL exclude it from all retrieval results
8. WHILE a memory has status "cooldown", THE Retrieval_Engine SHALL apply a penalty factor to its composite score
9. WHEN filtering by event_source pattern, THE Retrieval_Engine SHALL support filtering memories by their event_source field using prefix matching
10. WHEN filtering by category, THE Retrieval_Engine SHALL support filtering memories by their category field

### Requirement 4: Memory 生命周期管理

**User Story:** As a system administrator, I want memories to automatically transition through lifecycle states, so that the system remains performant and relevant.

#### Acceptance Criteria

1. WHEN a memory's TTL expires, THE Lifecycle_Manager SHALL transition its status to "archived"
2. WHEN a memory has not been hit for a configurable duration, THE Lifecycle_Manager SHALL transition its status to "cooldown"
3. WHEN a cooldown memory is hit again, THE Lifecycle_Manager SHALL transition its status back to "active"
4. WHEN an administrator explicitly marks a memory as ignored, THE Lifecycle_Manager SHALL set status to "ignored" and record the reason
5. THE Lifecycle_Manager SHALL log all state transitions to an audit table with timestamp and reason

### Requirement 5: 向量化服务集成

**User Story:** As an Agent developer, I want memories to be automatically embedded, so that semantic search works without manual embedding.

#### Acceptance Criteria

1. WHEN a memory is created or content is updated, THE Memory_Server SHALL call the configured embedding provider to generate a vector
2. THE Memory_Server SHALL store the embedding vector in Qdrant with the memory ID as reference
3. WHEN embedding fails, THE Memory_Server SHALL retry up to 3 times with exponential backoff
4. IF embedding fails after retries, THEN THE Memory_Server SHALL mark the memory with embedding_status "failed" and continue operation
5. THE Memory_Server SHALL support configurable embedding providers (Openai, local models)

### Requirement 6: Docker Compose 部署

**User Story:** As a DevOps engineer, I want to deploy the entire Memory Server stack with one command, so that I can quickly set up the service.

#### Acceptance Criteria

1. WHEN a user runs "docker-compose up", THE deployment SHALL start PostgreSQL, Qdrant and Memory_Server containers
2. THE deployment SHALL automatically run database migrations on first startup
3. THE deployment SHALL expose Memory_Server API on a configurable port (default 8080)
4. THE deployment SHALL persist PostgreSQL and Qdrant data to named volumes
5. THE deployment SHALL support environment-based configuration for all services
6. WHEN any container fails health check, THE deployment SHALL restart it automatically

### Requirement 7: 配置管理

**User Story:** As a system administrator, I want to configure the Memory Server through YAML files, so that I can customize behavior without code changes.

#### Acceptance Criteria

1. THE Memory_Server SHALL load configuration from a YAML file at startup
2. THE configuration SHALL include database connection settings, embedding provider settings, and lifecycle thresholds
3. WHEN configuration file is missing required fields, THE Memory_Server SHALL fail startup with a clear error message
4. THE Memory_Server SHALL support environment variable overrides for sensitive values like API keys

### Requirement 8: 审计日志

**User Story:** As a system administrator, I want all memory operations to be auditable, so that I can track changes and debug issues.

#### Acceptance Criteria

1. WHEN any memory is created, updated or deleted, THE Memory_Server SHALL record an audit log entry
2. THE audit log SHALL include timestamp, operation type, memory ID, actor ID and change details
3. THE Memory_Server SHALL provide an API to query audit logs by memory ID or time range
4. THE audit logs SHALL be stored in PostgreSQL for durability

### Requirement 9: 健康检查与监控

**User Story:** As a DevOps engineer, I want to monitor the Memory Server health, so that I can ensure service reliability.

#### Acceptance Criteria

1. THE Memory_Server SHALL expose a /health endpoint returning service status
2. THE health check SHALL verify PostgreSQL connectivity
3. THE health check SHALL verify Qdrant connectivity
4. WHEN any dependency is unhealthy, THE health endpoint SHALL return degraded status with details
5. THE Memory_Server SHALL expose Prometheus-compatible metrics endpoint

### Requirement 10: 模型配置中心

**User Story:** As a system administrator, I want to configure multiple embedding models and their parameters, so that I can switch providers and tune performance without code changes.

#### Acceptance Criteria

1. THE Memory_Server SHALL support configuring multiple embedding providers (Openai, Azure Openai, local models like BGE)
2. THE configuration SHALL include provider-specific settings: API endpoint, API key, model name, embedding dimension
3. THE Memory_Server SHALL support setting a default embedding provider for new memories
4. WHEN creating a memory, THE Agent SHALL be able to specify which embedding provider to use
5. THE Memory_Server SHALL validate embedding dimension matches the configured Qdrant collection dimension
6. THE configuration SHALL support rate limiting settings per provider (requests per minute, tokens per minute)
7. WHEN rate limit is exceeded, THE Memory_Server SHALL queue embedding requests and process them when quota refreshes
8. THE Memory_Server SHALL support local embedding models via HTTP endpoint (compatible with Openai API format)

### Requirement 11: 模型参数热更新

**User Story:** As a system administrator, I want to update model configurations at runtime, so that I can adjust settings without restarting the service.

#### Acceptance Criteria

1. THE Memory_Server SHALL provide an API to list all configured embedding providers
2. THE Memory_Server SHALL provide an API to add a new embedding provider at runtime
3. THE Memory_Server SHALL provide an API to update an existing provider's configuration
4. THE Memory_Server SHALL provide an API to disable/enable a provider without removing it
5. WHEN the default provider is disabled, THE Memory_Server SHALL reject new memory creation until a new default is set
6. THE Memory_Server SHALL persist runtime configuration changes to the database
7. WHEN Memory_Server restarts, THE Memory_Server SHALL load the latest configuration from database merged with YAML defaults


### Requirement 12: LLM 记忆处理管道

**User Story:** As an Agent developer, I want the Memory Server to optionally process raw content (conversation logs, operation records) through LLM to extract, compress and classify memories, so that I can store high-quality structured memories without implementing this logic myself.

#### Acceptance Criteria

1. WHEN creating a memory, THE Agent SHALL be able to set a `process_with_llm` flag to enable LLM processing
2. WHEN `process_with_llm` is true, THE Memory_Processor SHALL call the configured LLM to compress and extract key information from raw content
3. WHEN `process_with_llm` is true, THE Memory_Processor SHALL automatically classify the memory into categories (user_preference, behavior_pattern, business_rule, factual_knowledge, other)
4. WHEN `process_with_llm` is true, THE Memory_Server SHALL store both raw_content and processed_content, using processed_content for retrieval
5. WHEN `process_with_llm` is false or not specified, THE Memory_Server SHALL store content directly without LLM processing
6. THE Memory_Processor SHALL extract structured tags/keywords from content for improved retrieval
7. WHEN LLM processing fails, THE Memory_Server SHALL fall back to storing raw content and mark processing_status as "failed"
8. THE Memory_Server SHALL support configuring the LLM provider for memory processing (can be different from embedding provider)
9. THE Memory_Processor SHALL use a configurable prompt template for compression and classification
10. WHEN updating a memory with `process_with_llm` enabled, THE Memory_Processor SHALL re-process the combined content

### Requirement 13: 全文检索支持

**User Story:** As an Agent developer, I want to search memories using keyword matching in addition to semantic search, so that I can find memories with specific terms or phrases.

#### Acceptance Criteria

1. THE Memory_Server SHALL create PostgreSQL tsvector index on memory content for full-text search
2. WHEN a memory is created or updated, THE Memory_Server SHALL automatically update the tsvector column
3. THE Retrieval_Engine SHALL support full-text search queries using PostgreSQL tsquery syntax
4. THE Retrieval_Engine SHALL support combining full-text search with vector similarity search
5. WHEN both full-text and vector search are used, THE Retrieval_Engine SHALL compute a weighted composite score
6. THE full-text search SHALL support Chinese language tokenization (using zhparser or pg_jieba extension)
7. THE Retrieval_Engine SHALL support highlighting matched terms in search results
8. THE Memory_Server SHALL support configuring full-text search weight in the composite score calculation
