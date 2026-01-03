-- Memory Server Database Schema
-- Migration: 002_events_and_relations.sql
-- Description: Add events table and event-memory relations for evidence-based memory promotion

-- ============================================================================
-- NEW ENUM TYPES
-- ============================================================================

-- Relation type between events and memories
CREATE TYPE event_memory_relation_type AS ENUM ('created_from', 'reinforced_by', 'contradicted_by');

-- ============================================================================
-- EVENTS TABLE
-- ============================================================================

-- Events table to store raw events as evidence for memories
CREATE TABLE events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id VARCHAR(255) NOT NULL,
    -- Event content and metadata
    content TEXT NOT NULL,
    context TEXT,
    event_type VARCHAR(100),
    -- Scope information (inherited by created memories)
    scope_type scope_type NOT NULL,
    scope_id VARCHAR(255) NOT NULL,
    scene VARCHAR(255) NOT NULL,
    -- LLM processing results
    summary TEXT,
    processed BOOLEAN NOT NULL DEFAULT false,
    -- Timestamps
    event_time TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================================
-- EVENT-MEMORY RELATIONS TABLE
-- ============================================================================

-- Many-to-many relationship between events and memories
CREATE TABLE event_memory_relations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id UUID NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    memory_id UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    relation_type event_memory_relation_type NOT NULL,
    -- Similarity score when matching existing memories
    similarity_score REAL,
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Ensure unique event-memory-relation combination
    UNIQUE(event_id, memory_id, relation_type)
);

-- ============================================================================
-- ADD PROMOTION FIELDS TO MEMORIES TABLE
-- ============================================================================

-- Add promotion tracking fields (evidence_count is computed from relations table)
ALTER TABLE memories ADD COLUMN IF NOT EXISTS promoted_at TIMESTAMPTZ;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS promotion_reason TEXT;

-- ============================================================================
-- INDEXES FOR EVENTS
-- ============================================================================

CREATE INDEX idx_events_owner ON events(owner_id);
CREATE INDEX idx_events_owner_scope ON events(owner_id, scope_type, scope_id);
CREATE INDEX idx_events_scope ON events(scope_type, scope_id);
CREATE INDEX idx_events_scene ON events(scene);
CREATE INDEX idx_events_event_time ON events(event_time);
CREATE INDEX idx_events_processed ON events(processed);
CREATE INDEX idx_events_created_at ON events(created_at);

-- ============================================================================
-- INDEXES FOR EVENT-MEMORY RELATIONS
-- ============================================================================

CREATE INDEX idx_event_memory_relations_event ON event_memory_relations(event_id);
CREATE INDEX idx_event_memory_relations_memory ON event_memory_relations(memory_id);
CREATE INDEX idx_event_memory_relations_type ON event_memory_relations(relation_type);

-- ============================================================================
-- INDEXES FOR MEMORY PROMOTION
-- ============================================================================

CREATE INDEX idx_memories_promoted_at ON memories(promoted_at) WHERE promoted_at IS NOT NULL;

-- ============================================================================
-- PROMOTION CONFIGURATION
-- ============================================================================

-- Add promotion configuration to lifecycle_config
INSERT INTO lifecycle_config (key, value) VALUES
    ('promotion_criteria', '{
        "min_evidence_count": 3,
        "min_confidence": 0.7,
        "min_scope_diversity": 2,
        "min_age_hours": 24
    }')
ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW();
