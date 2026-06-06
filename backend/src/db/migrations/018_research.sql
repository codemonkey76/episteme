-- Phase 9: deep-research jobs + rendered HTML reports.
-- SQLite can't alter a CHECK constraint, so rebuild jobs (006-style) to admit
-- kind='research' and a small JSON meta blob ({"topic":…,"depth":…}).

CREATE TABLE jobs_new (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK(kind IN ('background', 'scheduled', 'research')),
    name        TEXT NOT NULL,
    provider    TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'running'
                CHECK(status IN ('running', 'needs_approval', 'done', 'failed')),
    summary     TEXT,
    error       TEXT,
    meta        TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
INSERT INTO jobs_new (id, user_id, session_id, kind, name, provider, status, summary, error, created_at, updated_at)
    SELECT id, user_id, session_id, kind, name, provider, status, summary, error, created_at, updated_at FROM jobs;
DROP TABLE jobs;
ALTER TABLE jobs_new RENAME TO jobs;
CREATE INDEX idx_jobs_user_time ON jobs(user_id, created_at);
CREATE INDEX idx_jobs_session ON jobs(session_id);

-- Rendered research reports: one self-contained HTML document each.
CREATE TABLE reports (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    job_id      TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    title       TEXT NOT NULL,
    html        TEXT NOT NULL,
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_reports_user_time ON reports(user_id, created_at);
