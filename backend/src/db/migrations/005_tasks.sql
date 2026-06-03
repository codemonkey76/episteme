-- User to-do tasks, manageable from the Tasks window and by the chat agent.
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    notes TEXT,
    due_at TEXT,                              -- RFC3339 UTC, nullable
    priority TEXT NOT NULL DEFAULT 'normal',  -- low | normal | high
    status TEXT NOT NULL DEFAULT 'open',      -- open | done
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tasks_status_due ON tasks(status, due_at);
