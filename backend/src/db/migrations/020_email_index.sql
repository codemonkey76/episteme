-- Semantic email: embeddings of incoming mail seen by the auto-sort worker,
-- so email_search can match by meaning, not just Graph $search keywords. Only
-- lightweight metadata is kept (sender, subject, a short snippet) — bodies
-- stay in the mailbox.
CREATE TABLE IF NOT EXISTS email_embeddings (
    user_id     TEXT NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
    message_id  TEXT NOT NULL,
    -- Shared mailbox address, or '' for the user's own mailbox.
    mailbox     TEXT NOT NULL DEFAULT '',
    subject     TEXT NOT NULL,
    sender      TEXT NOT NULL,
    snippet     TEXT NOT NULL,
    received_at TEXT NOT NULL,
    embedding   BLOB NOT NULL,     -- f32 little-endian, same shape as memories
    created_at  TEXT NOT NULL,
    PRIMARY KEY (user_id, message_id)
);

CREATE INDEX IF NOT EXISTS email_embeddings_user ON email_embeddings(user_id, received_at);
