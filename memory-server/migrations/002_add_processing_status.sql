-- Add explicit in-flight status for event processing claims.
ALTER TYPE processing_status ADD VALUE IF NOT EXISTS 'processing';
