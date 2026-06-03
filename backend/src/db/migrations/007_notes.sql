-- Freeform markdown notes, manageable from the Notes window and by the chat agent.
CREATE TABLE IF NOT EXISTS notes (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    content TEXT NOT NULL,          -- markdown
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_notes_updated ON notes(updated_at DESC);
