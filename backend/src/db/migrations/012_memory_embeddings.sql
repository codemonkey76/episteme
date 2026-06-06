-- Semantic memory: per-row embedding vector (little-endian f32 blob from the
-- Ollama embedding model). NULL = not yet embedded; backfilled lazily.
ALTER TABLE memories ADD COLUMN embedding BLOB;
