-- Single-admin authentication. The app expects at most one row in auth_users
-- (created via the first-run setup screen), but the schema does not enforce a
-- cap so multi-user support can be added later without a migration.

CREATE TABLE IF NOT EXISTS auth_users (
    id            TEXT PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,      -- argon2 PHC string
    created_at    TEXT NOT NULL
);

-- Opaque server-side sessions. The token is stored in an HttpOnly cookie; the
-- row is the source of truth, so logout/expiry are enforced server-side.
CREATE TABLE IF NOT EXISTS auth_sessions (
    token       TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL,
    expires_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_auth_sessions_user ON auth_sessions(user_id);
