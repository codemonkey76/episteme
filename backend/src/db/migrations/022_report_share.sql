-- Public share links for research reports. An opaque token (set = shared,
-- NULL = private) serves the self-contained HTML at /shared/:token with no
-- account required. Unique only when present, so most rows stay NULL.
ALTER TABLE reports ADD COLUMN share_token TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_reports_share_token
    ON reports(share_token) WHERE share_token IS NOT NULL;
