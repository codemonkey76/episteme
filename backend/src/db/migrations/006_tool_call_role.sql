-- Allow role 'tool_call' (the assistant's tool invocations, replayed into
-- model history so it knows it already acted). SQLite can't alter a CHECK
-- constraint, so rebuild the table.
CREATE TABLE messages_new (
    id          TEXT PRIMARY KEY,
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role        TEXT NOT NULL CHECK(role IN ('user', 'assistant', 'tool', 'tool_call')),
    content     TEXT NOT NULL,     -- JSON: string or structured content parts
    tool_calls  TEXT,              -- JSON array of tool call objects, nullable
    tool_call_id TEXT,             -- set when role = 'tool'
    created_at  TEXT NOT NULL
);

INSERT INTO messages_new
    SELECT id, session_id, role, content, tool_calls, tool_call_id, created_at FROM messages;

DROP TABLE messages;
ALTER TABLE messages_new RENAME TO messages;

CREATE INDEX IF NOT EXISTS messages_session_id ON messages(session_id, created_at);
