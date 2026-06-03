CREATE TABLE IF NOT EXISTS memories (
    id          TEXT PRIMARY KEY,
    content     TEXT NOT NULL,
    category    TEXT NOT NULL DEFAULT 'fact',   -- preference|fact|feedback|project|other
    source      TEXT NOT NULL DEFAULT 'auto',   -- auto|manual
    session_id  TEXT,                           -- origin session, nullable
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS memories_created ON memories(created_at DESC);
