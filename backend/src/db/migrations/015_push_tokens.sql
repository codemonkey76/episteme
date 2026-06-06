-- FCM device tokens for push notifications. One row per device; tokens are
-- upserted on app login and pruned when FCM reports them dead.
CREATE TABLE push_tokens (
    token TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    platform TEXT NOT NULL DEFAULT 'android',
    created_at TEXT NOT NULL
);
CREATE INDEX idx_push_tokens_user ON push_tokens(user_id);
