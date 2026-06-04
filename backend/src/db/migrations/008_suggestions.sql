-- AI-detected commitments from sent emails, awaiting user accept/dismiss.
CREATE TABLE IF NOT EXISTS suggestions (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('task', 'event')),
    title TEXT NOT NULL,
    start_at TEXT,            -- RFC3339 UTC; event start or task due (nullable)
    end_at TEXT,              -- events only, nullable
    context TEXT,             -- where it came from, e.g. "Reply to dave@…: Re: Maintenance"
    status TEXT NOT NULL DEFAULT 'pending',  -- pending | accepted | dismissed
    created_at TEXT NOT NULL,
    resolved_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_suggestions_status ON suggestions(status, created_at);
