-- Long-form markdown documents written in the Writer window, with an AI
-- "Improve" feature. Separate from `documents` (which is uploaded files for
-- retrieval) and `notes` (short freeform jottings).
CREATE TABLE IF NOT EXISTS writer_docs (
    id TEXT PRIMARY KEY,
    user_id TEXT,
    title TEXT NOT NULL,
    content TEXT NOT NULL,          -- markdown
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_writer_docs_user ON writer_docs(user_id, updated_at DESC);
