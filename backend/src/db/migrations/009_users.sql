-- Multi-user: roles + status on accounts, single-use invites, and per-user
-- ownership of all personal data. Everything existing belongs to the first
-- (oldest) account, which becomes the admin.

-- Roles / status. SQLite can't ALTER with CHECK on existing tables cleanly,
-- so plain columns; values are enforced in code.
ALTER TABLE auth_users ADD COLUMN role TEXT NOT NULL DEFAULT 'member';
ALTER TABLE auth_users ADD COLUMN status TEXT NOT NULL DEFAULT 'active';

UPDATE auth_users SET role = 'admin'
WHERE id = (SELECT id FROM auth_users ORDER BY created_at ASC LIMIT 1);

-- Single-use invite codes; the link is https://<domain>/?invite=<code>.
CREATE TABLE IF NOT EXISTS invites (
    code TEXT PRIMARY KEY,
    label TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_by TEXT REFERENCES auth_users(id),
    used_at TEXT
);

-- Per-user ownership of personal data; legacy rows go to the admin.
ALTER TABLE sessions    ADD COLUMN user_id TEXT;
ALTER TABLE tasks       ADD COLUMN user_id TEXT;
ALTER TABLE notes       ADD COLUMN user_id TEXT;
ALTER TABLE memories    ADD COLUMN user_id TEXT;
ALTER TABLE suggestions ADD COLUMN user_id TEXT;

UPDATE sessions    SET user_id = (SELECT id FROM auth_users ORDER BY created_at ASC LIMIT 1);
UPDATE tasks       SET user_id = (SELECT id FROM auth_users ORDER BY created_at ASC LIMIT 1);
UPDATE notes       SET user_id = (SELECT id FROM auth_users ORDER BY created_at ASC LIMIT 1);
UPDATE memories    SET user_id = (SELECT id FROM auth_users ORDER BY created_at ASC LIMIT 1);
UPDATE suggestions SET user_id = (SELECT id FROM auth_users ORDER BY created_at ASC LIMIT 1);

CREATE INDEX IF NOT EXISTS idx_sessions_user    ON sessions(user_id, updated_at);
CREATE INDEX IF NOT EXISTS idx_tasks_user       ON tasks(user_id, status);
CREATE INDEX IF NOT EXISTS idx_notes_user       ON notes(user_id, updated_at);
CREATE INDEX IF NOT EXISTS idx_memories_user    ON memories(user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_suggestions_user ON suggestions(user_id, status);

-- Per-user settings move to suffixed keys (<key>:<user_id>); the admin
-- inherits the existing values. App-level Azure credentials split out of the
-- per-user mailbox config in code (global key microsoft_app).
INSERT OR IGNORE INTO settings (key, value)
SELECT 'timezone:' || (SELECT id FROM auth_users ORDER BY created_at ASC LIMIT 1), value
FROM settings WHERE key = 'timezone';

INSERT OR IGNORE INTO settings (key, value)
SELECT 'microsoft_email:' || (SELECT id FROM auth_users ORDER BY created_at ASC LIMIT 1), value
FROM settings WHERE key = 'microsoft_email';

INSERT OR IGNORE INTO settings (key, value)
SELECT 'email_categorizer:' || (SELECT id FROM auth_users ORDER BY created_at ASC LIMIT 1), value
FROM settings WHERE key = 'email_categorizer';

INSERT OR IGNORE INTO settings (key, value)
SELECT 'email_categorizer_state:' || (SELECT id FROM auth_users ORDER BY created_at ASC LIMIT 1), value
FROM settings WHERE key = 'email_categorizer_state';

DELETE FROM settings
WHERE key IN ('timezone', 'email_categorizer', 'email_categorizer_state');
-- 'microsoft_email' is kept temporarily: code migrates the shared Azure app
-- credentials out of it into 'microsoft_app' on first run, then removes it.
