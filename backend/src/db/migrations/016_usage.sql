-- Token usage accounting: one row per model request that reported counts.
-- purpose: chat | memory | style | auto-sort | commitments | email-ai | …
CREATE TABLE usage (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    model_id TEXT NOT NULL,
    purpose TEXT NOT NULL,
    prompt_tokens INTEGER NOT NULL,
    completion_tokens INTEGER NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_usage_user_time ON usage(user_id, created_at);
CREATE INDEX idx_usage_time ON usage(created_at);
