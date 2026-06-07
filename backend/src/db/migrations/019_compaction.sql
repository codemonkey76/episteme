-- Phase 10: context compaction. A rolling summary of the session's older
-- messages plus the created_at cursor of the last message it covers; the
-- model-facing history becomes summary + messages after the cursor. The full
-- transcript stays in messages for display and search.
ALTER TABLE sessions ADD COLUMN summary TEXT;
ALTER TABLE sessions ADD COLUMN summary_until TEXT;
