-- Memory Server Database Schema
-- Migration: 001_init.sql
-- Description: Complete schema setup with all tables, enums, indexes, full-text search, and LLM providers

-- ============================================================================
-- ENUM TYPES
-- ============================================================================

-- Memory layer determining lifecycle and TTL behavior
CREATE TYPE layer AS ENUM ('session', 'task', 'long_term');

-- Memory scope type determining ownership
CREATE TYPE scope_type AS ENUM ('user', 'org', 'project', 'task', 'session');

-- Memory lifecycle status
CREATE TYPE status AS ENUM ('candidate', 'active', 'stable', 'cooldown', 'ignored', 'archived');

-- Embedding generation status
CREATE TYPE embedding_status AS ENUM ('pending', 'completed', 'failed');

-- Processing status for LLM memory processing
CREATE TYPE processing_status AS ENUM ('pending', 'completed', 'failed', 'skipped');

-- Memory content update mode
CREATE TYPE update_mode AS ENUM ('append', 'merge', 'supersede');

-- Provider type (for both embedding and LLM providers)
CREATE TYPE provider_type AS ENUM ('openai', 'azure', 'local');

-- Memory category for LLM classification
CREATE TYPE memory_category AS ENUM ('user_preference', 'behavior_pattern', 'business_rule', 'factual_knowledge', 'other');

-- ============================================================================
-- TABLES
-- ============================================================================

-- Memory main table
CREATE TABLE memories (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    layer layer NOT NULL,
    scope_type scope_type NOT NULL,
    scope_id VARCHAR(255) NOT NULL,
    scene VARCHAR(255) NOT NULL,
    status status NOT NULL DEFAULT 'active',
    -- Content fields
    content TEXT NOT NULL,
    raw_content TEXT,
    -- Category and tags
    category memory_category,
    tags TEXT[],
    -- Full-text search
    content_tsv TSVECTOR,
    -- Metadata
    importance REAL NOT NULL DEFAULT 0.5,
    confidence REAL NOT NULL DEFAULT 1.0,
    hit_count BIGINT NOT NULL DEFAULT 0,
    last_hit_at TIMESTAMPTZ,
    ttl_seconds BIGINT,
    expires_at TIMESTAMPTZ,
    event_source VARCHAR(255),
    event_time TIMESTAMPTZ,
    -- Processing status
    embedding_status embedding_status NOT NULL DEFAULT 'pending',
    embedding_provider VARCHAR(100),
    processing_status processing_status NOT NULL DEFAULT 'skipped',
    llm_provider VARCHAR(100),
    -- Inference fields (for memories extracted from events)
    inference_type VARCHAR(50),
    inference_confidence REAL,
    inference_reasoning TEXT,
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);


-- Embedding provider configuration table
CREATE TABLE embedding_providers (
    name VARCHAR(100) PRIMARY KEY,
    provider_type provider_type NOT NULL,
    endpoint VARCHAR(500) NOT NULL,
    api_key_encrypted BYTEA,
    model VARCHAR(200) NOT NULL,
    dimension INTEGER NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    is_default BOOLEAN NOT NULL DEFAULT false,
    rpm_limit INTEGER,
    tpm_limit INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- LLM provider configuration table (for memory processing)
CREATE TABLE llm_providers (
    name VARCHAR(100) PRIMARY KEY,
    provider_type provider_type NOT NULL,
    endpoint VARCHAR(500) NOT NULL,
    api_key_encrypted BYTEA,
    model VARCHAR(200) NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    is_default BOOLEAN NOT NULL DEFAULT false,
    rpm_limit INTEGER,
    tpm_limit INTEGER,
    -- Processing configuration
    compression_prompt TEXT,
    classification_prompt TEXT,
    max_input_tokens INTEGER DEFAULT 4000,
    max_output_tokens INTEGER DEFAULT 1000,
    temperature REAL DEFAULT 0.3,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Audit log table
CREATE TABLE audit_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    memory_id UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    operation VARCHAR(50) NOT NULL,
    actor_id VARCHAR(255),
    old_value JSONB,
    new_value JSONB,
    reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Lifecycle configuration table
CREATE TABLE lifecycle_config (
    key VARCHAR(100) PRIMARY KEY,
    value JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================================
-- INDEXES
-- ============================================================================

-- Memory table indexes
CREATE INDEX idx_memories_scope ON memories(scope_type, scope_id);
CREATE INDEX idx_memories_scene ON memories(scene);
CREATE INDEX idx_memories_layer ON memories(layer);
CREATE INDEX idx_memories_status ON memories(status);
CREATE INDEX idx_memories_category ON memories(category) WHERE category IS NOT NULL;
CREATE INDEX idx_memories_tags ON memories USING GIN(tags) WHERE tags IS NOT NULL;
CREATE INDEX idx_memories_expires_at ON memories(expires_at) WHERE expires_at IS NOT NULL;
CREATE INDEX idx_memories_last_hit_at ON memories(last_hit_at);
CREATE INDEX idx_memories_event_source ON memories(event_source) WHERE event_source IS NOT NULL;
CREATE INDEX idx_memories_embedding_status ON memories(embedding_status);
CREATE INDEX idx_memories_created_at ON memories(created_at);
-- Full-text search index
CREATE INDEX idx_memories_content_tsv ON memories USING GIN(content_tsv);

-- Audit log indexes
CREATE INDEX idx_audit_logs_memory_id ON audit_logs(memory_id);
CREATE INDEX idx_audit_logs_created_at ON audit_logs(created_at);
CREATE INDEX idx_audit_logs_operation ON audit_logs(operation);


-- ============================================================================
-- FUNCTIONS
-- ============================================================================

-- Function to update updated_at timestamp
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Function to automatically update tsvector when content changes
CREATE OR REPLACE FUNCTION memories_update_tsv()
RETURNS TRIGGER AS $$
BEGIN
    NEW.content_tsv := to_tsvector('simple', COALESCE(NEW.content, ''));
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- TRIGGERS
-- ============================================================================

-- Auto-update updated_at for memories table
CREATE TRIGGER update_memories_updated_at
    BEFORE UPDATE ON memories
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Auto-update updated_at for embedding_providers table
CREATE TRIGGER update_embedding_providers_updated_at
    BEFORE UPDATE ON embedding_providers
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Auto-update updated_at for llm_providers table
CREATE TRIGGER update_llm_providers_updated_at
    BEFORE UPDATE ON llm_providers
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Auto-update updated_at for lifecycle_config table
CREATE TRIGGER update_lifecycle_config_updated_at
    BEFORE UPDATE ON lifecycle_config
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Auto-update tsvector on INSERT or UPDATE of content
CREATE TRIGGER memories_tsv_trigger
    BEFORE INSERT OR UPDATE OF content ON memories
    FOR EACH ROW
    EXECUTE FUNCTION memories_update_tsv();

-- ============================================================================
-- DEFAULT DATA
-- ============================================================================

-- Default lifecycle configuration
INSERT INTO lifecycle_config (key, value) VALUES
    ('cooldown_threshold_days', '{"session": 1, "task": 7, "long_term": 30}'),
    ('default_ttl_seconds', '{"session": 3600, "task": 604800, "long_term": null}'),
    ('compression_prompt', '"你是一个记忆压缩助手。请从以下对话/操作记录中提取关键信息，生成简洁的结构化记忆。\n\n要求：\n1. 保留核心事实和用户偏好\n2. 去除冗余和无关信息\n3. 使用简洁的陈述句\n4. 保持原意不变\n\n原始内容：\n{content}\n\n压缩后的记忆："'),
    ('classification_prompt', '"请将以下记忆分类到最合适的类别：\n- user_preference: 用户偏好（如喜好、习惯设置）\n- behavior_pattern: 行为模式（如工作习惯、操作方式）\n- business_rule: 业务规则（如流程、规定）\n- factual_knowledge: 事实知识（如日期、数据）\n- other: 其他\n\n记忆内容：\n{content}\n\n请只返回类别名称："');
