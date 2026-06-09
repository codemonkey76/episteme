-- Memory consolidation ("dreaming"): the nightly/manual pass merges redundant
-- memories, resolves conflicts, and synthesises lessons. Deletes are SOFT so a
-- consolidation is reversible — the row is hidden, not gone, and can be restored.
--   deleted_at:    NULL = active; a timestamp = archived (soft-deleted).
--   superseded_by: when a memory was merged away, the id of the consolidated
--                  memory that replaced it (provenance for restore/inspection).
ALTER TABLE memories ADD COLUMN deleted_at TEXT;
ALTER TABLE memories ADD COLUMN superseded_by TEXT;
CREATE INDEX IF NOT EXISTS memories_active ON memories(user_id, deleted_at);
