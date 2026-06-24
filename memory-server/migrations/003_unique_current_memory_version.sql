-- Prevent a version chain from having more than one current memory.
CREATE UNIQUE INDEX IF NOT EXISTS idx_memories_one_current_per_root
    ON memories(root_memory_id)
    WHERE is_current_version = true
      AND root_memory_id IS NOT NULL;
