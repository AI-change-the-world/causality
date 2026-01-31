# Requirements Document

## Introduction

SystemProfile（系统画像）是 Memory System 的核心配置实体，用于定义系统的业务边界、目标对象和行为规范。通过 SystemProfile，系统能够：
- 明确业务领域和用途，避免功能发散
- 定义事件分类，便于记忆的组织和检索
- 指导 LLM 进行更精准的记忆提取和分类
- 简化配置，使用单一 LLM/Embedding provider 配置

同时，本功能将简化 LLM 和 Embedding Provider 的配置方式，从"用户可无限创建"改为"环境变量单一配置"。

## Glossary

- **System_Profile**: 系统画像实体，包含系统用途、业务领域、目标对象、事件分类等配置信息
- **Profile_Service**: 负责 SystemProfile 的创建、查询、更新的服务层
- **Profile_Repository**: SystemProfile 的数据持久化层
- **LLM_Provider**: 大语言模型提供者，用于理解用户输入并生成结构化配置
- **Initialization_API**: 系统初始化接口，接收用户描述并生成 SystemProfile

## Requirements

### Requirement 1: 系统画像数据模型

**User Story:** As a system administrator, I want to define a structured system profile, so that the memory system has clear boundaries and behavior guidelines.

#### Acceptance Criteria

1. THE System_Profile SHALL contain a unique identifier (id)
2. THE System_Profile SHALL contain a system name (name) with maximum 100 characters
3. THE System_Profile SHALL contain a system description (description) for storing the original user input
4. THE System_Profile SHALL contain a purpose field describing what the system does
5. THE System_Profile SHALL contain a domain field describing the business domain
6. THE System_Profile SHALL contain a target_audience field describing who the system serves
7. THE System_Profile SHALL contain an event_categories field as a list of valid event types
8. THE System_Profile SHALL contain a memory_focus field as a list of memory types to prioritize
9. THE System_Profile SHALL contain a boundaries field as a list of things the system should not handle
10. THE System_Profile SHALL contain created_at and updated_at timestamps
11. THE System_Profile SHALL be a singleton - only one profile can exist per system instance

### Requirement 2: 系统初始化

**User Story:** As a system administrator, I want to initialize the system with a natural language description, so that the system can automatically understand and configure itself.

#### Acceptance Criteria

1. WHEN a user provides a system description THEN THE Initialization_API SHALL use LLM to parse the description into structured fields
2. WHEN the LLM parses the description THEN THE System_Profile SHALL be created with all extracted fields
3. WHEN a System_Profile already exists THEN THE Initialization_API SHALL return an error indicating the system is already initialized
4. IF the LLM fails to parse the description THEN THE Initialization_API SHALL return a meaningful error message
5. WHEN initialization succeeds THEN THE Initialization_API SHALL return the created System_Profile

### Requirement 3: 系统画像查询

**User Story:** As a developer, I want to query the current system profile, so that I can understand the system's configuration and boundaries.

#### Acceptance Criteria

1. WHEN a user requests the system profile THEN THE Profile_Service SHALL return the current System_Profile
2. IF no System_Profile exists THEN THE Profile_Service SHALL return a 404 error with a message indicating the system is not initialized

### Requirement 4: 系统画像更新

**User Story:** As a system administrator, I want to update the system profile, so that I can refine the system's behavior over time.

#### Acceptance Criteria

1. WHEN a user provides updated fields THEN THE Profile_Service SHALL update only the provided fields
2. WHEN a user provides a new description THEN THE Profile_Service SHALL use LLM to re-parse and update all derived fields
3. WHEN updating THEN THE Profile_Service SHALL update the updated_at timestamp
4. IF no System_Profile exists THEN THE Profile_Service SHALL return a 404 error

### Requirement 5: LLM/Embedding Provider 配置简化

**User Story:** As a system administrator, I want to configure LLM and Embedding providers in config.yaml only, so that the system is simpler without database-managed providers.

#### Acceptance Criteria

1. THE config.yaml SHALL contain complete LLM provider configuration (type, endpoint, api_key, model)
2. THE config.yaml SHALL contain complete Embedding provider configuration (type, endpoint, api_key, model, dimension)
3. THE System SHALL remove embedding_providers and llm_providers database tables
4. THE System SHALL remove provider management APIs (POST/PUT/DELETE /api/v1/config/providers)
5. THE Event API SHALL use the configured providers from config.yaml directly
6. WHEN config.yaml provider configuration is invalid THEN THE System SHALL fail to start with a clear error message

### Requirement 6: 事件结构化（六要素模型）

**User Story:** As a developer, I want events to be structured using a standardized format (six elements + two auxiliary), so that memory extraction is more systematic and consistent.

#### Acceptance Criteria

1. THE Structured_Event SHALL contain time element (when the event occurred)
2. THE Structured_Event SHALL contain location element (device, page, content position)
3. THE Structured_Event SHALL contain actor element (user identifier)
4. THE Structured_Event SHALL contain cause element (why the event was triggered, inferred from scope)
5. THE Structured_Event SHALL contain process element (what happened step by step)
6. THE Structured_Event SHALL contain result element (outcome of the event)
7. THE Structured_Event SHALL contain background auxiliary element (context/scenario)
8. THE Structured_Event SHALL contain details auxiliary element (key details, follow-up actions)
9. WHEN processing raw event content THEN THE Event_Handler SHALL use LLM to extract structured elements
10. THE Event_Handler SHALL reference System_Profile's extraction_strategy for element extraction guidance

### Requirement 7: 事件提取 Prompt 配置

**User Story:** As a system administrator, I want the system to generate a domain-specific extraction prompt, so that the LLM knows how to interpret events for this specific domain.

#### Acceptance Criteria

1. THE System_Profile SHALL contain an extraction_prompt field storing a complete prompt template
2. WHEN initializing the system THEN THE LLM SHALL generate extraction_prompt based on the system description and domain
3. THE extraction_prompt SHALL include guidance for extracting all six elements plus two auxiliary elements
4. WHEN processing events THEN THE Event_Handler SHALL use the extraction_prompt directly
5. THE extraction_prompt SHALL be updatable through the profile update API

### Requirement 8: 事件处理集成

**User Story:** As a developer, I want event processing to use the system profile and structured events, so that memories are extracted according to the defined strategies.

#### Acceptance Criteria

1. WHEN processing an event THEN THE Event_Handler SHALL first structure the raw content into six elements
2. WHEN extracting memories THEN THE Memory_Processor SHALL use the structured event elements
3. WHEN an element cannot be extracted THEN THE Event_Handler SHALL mark it as null/unknown
4. IF no System_Profile exists THEN THE Event processing SHALL use generic extraction without domain hints

### Requirement 9: 系统画像序列化

**User Story:** As a developer, I want the system profile to be serializable, so that it can be stored and transmitted as JSON.

#### Acceptance Criteria

1. THE System_Profile SHALL be serializable to JSON format
2. THE System_Profile SHALL be deserializable from JSON format
3. WHEN serializing THEN THE System_Profile SHALL include all fields with proper naming (snake_case)
