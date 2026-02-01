-- Memory Server Database Schema
-- Migration: 001_init.sql
-- Description: Simplified schema with Event-driven Memory architecture, version chains, and LFU eviction

-- ============================================================================
-- ENUM TYPES
-- ============================================================================

-- Memory lifecycle status (simplified)
CREATE TYPE status AS ENUM ('active', 'cooldown', 'candidate', 'superseded', 'archived');

-- Embedding generation status
CREATE TYPE embedding_status AS ENUM ('pending', 'completed', 'failed');

-- Processing status for LLM memory processing
CREATE TYPE processing_status AS ENUM ('pending', 'completed', 'failed', 'skipped');

-- Provider type (for both embedding and LLM providers)
CREATE TYPE provider_type AS ENUM ('openai', 'azure', 'local');

-- Memory category for LLM classification
CREATE TYPE memory_category AS ENUM ('user_preference', 'behavior_pattern', 'business_rule', 'factual_knowledge', 'other');

-- Relation type between events and memories
CREATE TYPE event_memory_relation_type AS ENUM ('created_from', 'reinforced_by');

-- ============================================================================
-- EVENTS TABLE (Immutable)
-- ============================================================================

CREATE TABLE events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id UUID NOT NULL REFERENCES system_profiles(id),
    owner_id VARCHAR(255) NOT NULL,
    scope_id VARCHAR(255),  -- nullable, user-defined
    -- Event content and metadata
    content TEXT NOT NULL,
    context TEXT,
    summary TEXT,
    source VARCHAR(50),  -- 'user_created' | 'api' | etc.
    -- Processing status
    processed BOOLEAN NOT NULL DEFAULT false,
    -- Timestamps
    event_time TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================================
-- MEMORIES TABLE (Content Immutable, Metadata Mutable)
-- ============================================================================

CREATE TABLE memories (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id UUID NOT NULL REFERENCES system_profiles(id),
    owner_id VARCHAR(255) NOT NULL,
    scope_id VARCHAR(255),  -- nullable, null = global memory
    
    -- Content fields (immutable after creation)
    content TEXT NOT NULL,
    category VARCHAR(500),  -- hierarchical: work.code.eslint
    tags TEXT[],
    importance REAL NOT NULL DEFAULT 0.5,
    confidence REAL NOT NULL DEFAULT 1.0,
    
    -- Version chain (materialized path for O(1) queries)
    root_memory_id UUID,  -- points to the root of version chain
    version_number INTEGER NOT NULL DEFAULT 1,
    is_current_version BOOLEAN NOT NULL DEFAULT true,
    supersedes UUID REFERENCES memories(id),
    superseded_by UUID REFERENCES memories(id),
    
    -- Lifecycle management (LFU eviction)
    is_global BOOLEAN NOT NULL DEFAULT false,
    hit_count BIGINT NOT NULL DEFAULT 0,
    last_hit_at TIMESTAMPTZ,
    decay_score REAL NOT NULL DEFAULT 1.0,
    
    -- Source tracking
    source_event_id UUID,  -- will add FK after events table exists
    
    -- Status
    status status NOT NULL DEFAULT 'active',
    
    -- Full-text search
    content_tsv TSVECTOR,
    
    -- Processing status
    embedding_status embedding_status NOT NULL DEFAULT 'pending',
    embedding_provider VARCHAR(100),
    processing_status processing_status NOT NULL DEFAULT 'skipped',
    llm_provider VARCHAR(100),
    
    -- Inference fields (for memories extracted from events)
    inference_type VARCHAR(50),
    inference_confidence REAL,
    inference_reasoning TEXT,
    
    -- Promotion tracking
    promoted_at TIMESTAMPTZ,
    promotion_reason TEXT,
    
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Add foreign key for source_event_id
ALTER TABLE memories ADD CONSTRAINT fk_memories_source_event 
    FOREIGN KEY (source_event_id) REFERENCES events(id);

-- ============================================================================
-- EVENT-MEMORY RELATIONS TABLE (Immutable)
-- ============================================================================

CREATE TABLE event_memory_relations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id UUID NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    memory_id UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    relation_type event_memory_relation_type NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Ensure unique event-memory combination
    UNIQUE(event_id, memory_id)
);

-- ============================================================================
-- SYSTEM PROFILE TABLE (Multi-tenant)
-- ============================================================================

CREATE TABLE system_profiles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL,
    description TEXT NOT NULL,
    purpose TEXT NOT NULL,
    domain VARCHAR(100) NOT NULL,
    target_audience VARCHAR(255) NOT NULL,
    event_categories TEXT[] NOT NULL DEFAULT '{}',
    memory_focus TEXT[] NOT NULL DEFAULT '{}',
    boundaries TEXT[] NOT NULL DEFAULT '{}',
    -- 事件提取 prompt (完整的 prompt 模板)
    extraction_prompt TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for profile name lookup
CREATE INDEX idx_system_profiles_name ON system_profiles(name);

-- ============================================================================
-- STRUCTURED EVENTS TABLE (六要素 + 两辅助)
-- ============================================================================

CREATE TABLE structured_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id UUID NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    -- 六要素
    time_element TEXT,
    location_element TEXT,
    actor_element VARCHAR(255) NOT NULL,
    cause_element TEXT,
    process_element TEXT,
    result_element TEXT,
    -- 两辅助
    background_element TEXT,
    details_element TEXT,
    -- 分类
    category VARCHAR(100),
    -- 时间戳
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- 一个事件只有一个结构化记录
    UNIQUE(event_id)
);

CREATE INDEX idx_structured_events_event ON structured_events(event_id);
CREATE INDEX idx_structured_events_category ON structured_events(category);
CREATE INDEX idx_structured_events_actor ON structured_events(actor_element);

-- ============================================================================
-- AUDIT LOG TABLE
-- ============================================================================

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

-- ============================================================================
-- LIFECYCLE CONFIGURATION TABLE
-- ============================================================================

CREATE TABLE lifecycle_config (
    key VARCHAR(100) PRIMARY KEY,
    value JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================================
-- INDEXES FOR EVENTS
-- ============================================================================

CREATE INDEX idx_events_profile ON events(profile_id);
CREATE INDEX idx_events_profile_owner ON events(profile_id, owner_id);
CREATE INDEX idx_events_owner ON events(owner_id);
CREATE INDEX idx_events_owner_scope ON events(owner_id, scope_id);
CREATE INDEX idx_events_processed ON events(processed) WHERE processed = false;
CREATE INDEX idx_events_event_time ON events(event_time);
CREATE INDEX idx_events_created_at ON events(created_at);

-- ============================================================================
-- INDEXES FOR MEMORIES
-- ============================================================================

-- Profile index
CREATE INDEX idx_memories_profile ON memories(profile_id);
CREATE INDEX idx_memories_profile_owner ON memories(profile_id, owner_id);

-- Version chain indexes (core optimization for O(1) queries)
CREATE INDEX idx_memories_root ON memories(root_memory_id);
CREATE INDEX idx_memories_current ON memories(root_memory_id, is_current_version) 
    WHERE is_current_version = true;

-- Basic query indexes
CREATE INDEX idx_memories_owner ON memories(owner_id);
CREATE INDEX idx_memories_owner_scope ON memories(owner_id, scope_id);
CREATE INDEX idx_memories_status ON memories(status);
CREATE INDEX idx_memories_global ON memories(owner_id, is_global) WHERE is_global = true;

-- Category index for prefix queries
CREATE INDEX idx_memories_category ON memories(category);

-- Lifecycle indexes
CREATE INDEX idx_memories_decay_score ON memories(decay_score);
CREATE INDEX idx_memories_last_hit_at ON memories(last_hit_at);
CREATE INDEX idx_memories_promoted_at ON memories(promoted_at) WHERE promoted_at IS NOT NULL;

-- Processing indexes
CREATE INDEX idx_memories_embedding_status ON memories(embedding_status);
CREATE INDEX idx_memories_created_at ON memories(created_at);

-- Full-text search index
CREATE INDEX idx_memories_content_tsv ON memories USING GIN(content_tsv);

-- Tags index
CREATE INDEX idx_memories_tags ON memories USING GIN(tags) WHERE tags IS NOT NULL;

-- ============================================================================
-- INDEXES FOR EVENT-MEMORY RELATIONS
-- ============================================================================

CREATE INDEX idx_emr_event ON event_memory_relations(event_id);
CREATE INDEX idx_emr_memory ON event_memory_relations(memory_id);
CREATE INDEX idx_emr_type ON event_memory_relations(relation_type);

-- ============================================================================
-- INDEXES FOR AUDIT LOGS
-- ============================================================================

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

-- Auto-update updated_at for system_profiles table
CREATE TRIGGER update_system_profiles_updated_at
    BEFORE UPDATE ON system_profiles
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
    ('decay_config', '{
        "decay_half_life_days": 7,
        "hit_boost_factor": 0.1,
        "global_boost": 2.0
    }'),
    ('eviction_config', '{
        "cooldown_threshold_days": 14,
        "candidate_threshold_days": 30,
        "archive_threshold_days": 90,
        "max_memories_per_owner": 10000,
        "decay_score_threshold": 0.1
    }'),
    ('promotion_criteria', '{
        "min_scope_diversity": 2,
        "min_reinforcements": 3,
        "min_confidence": 0.7,
        "min_age_hours": 24
    }'),
    ('compression_prompt', '"你是一个记忆压缩助手。请从以下对话/操作记录中提取关键信息，生成简洁的结构化记忆。\n\n要求：\n1. 保留核心事实和用户偏好\n2. 去除冗余和无关信息\n3. 使用简洁的陈述句\n4. 保持原意不变\n\n原始内容：\n{content}\n\n压缩后的记忆："'),
    ('classification_prompt', '"请将以下记忆分类到最合适的类别：\n- user_preference: 用户偏好（如喜好、习惯设置）\n- behavior_pattern: 行为模式（如工作习惯、操作方式）\n- business_rule: 业务规则（如流程、规定）\n- factual_knowledge: 事实知识（如日期、数据）\n- other: 其他\n\n记忆内容：\n{content}\n\n请只返回类别名称："');
