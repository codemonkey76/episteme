-- Persisted notifications: every push notification is also recorded here so the
-- user can review past ones, mark them read, clear them, and jump to the item
-- that produced them (a chat session, an approval, etc.). Push delivery stays
-- best-effort/fire-and-forget; this table is the durable record.
CREATE TABLE IF NOT EXISTS notifications (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT 'info',  -- agent | approval | error | suggestion | email | info
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    link_kind TEXT,                         -- e.g. 'session' (null = no associated item)
    link_id TEXT,
    read_at TEXT,                           -- null = unread
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_notifications_user ON notifications(user_id, created_at DESC);
