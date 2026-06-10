-- Multiple named instances of an integration per user (helpdesk/phoneus/github),
-- replacing the old one-config-per-type settings keys. A tool resolves which
-- instance to use by name, falling back to the type's default. `config` is the
-- type-specific JSON (token, base_url, email, default_owner, …); tokens stay
-- server-side and are never returned to the UI.
CREATE TABLE integrations (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    kind        TEXT NOT NULL,              -- helpdesk | phoneus | github
    name        TEXT NOT NULL,
    is_default  INTEGER NOT NULL DEFAULT 0,
    config      TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_integrations_user_kind ON integrations(user_id, kind);
