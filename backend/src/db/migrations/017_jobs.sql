-- Phase 8: background/scheduled agent runs tracked as jobs, plus a call_id on
-- pending_actions so a parked tool call can be executed later (the resumed
-- turn needs the original call id to write its tool-result row).

CREATE TABLE jobs (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK(kind IN ('background', 'scheduled')),
    name        TEXT NOT NULL,
    provider    TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'running'
                CHECK(status IN ('running', 'needs_approval', 'done', 'failed')),
    summary     TEXT,
    error       TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_jobs_user_time ON jobs(user_id, created_at);
CREATE INDEX idx_jobs_session ON jobs(session_id);

ALTER TABLE pending_actions ADD COLUMN call_id TEXT;
