-- Two-factor authentication (TOTP). totp_secret set = 2FA enabled;
-- totp_pending holds the secret during enrollment (QR shown, first code not
-- yet verified) so an abandoned setup never locks anyone out.
-- totp_last_step records the last accepted 30s timestep — a code can't be
-- replayed within its validity window.
ALTER TABLE auth_users ADD COLUMN totp_secret TEXT;
ALTER TABLE auth_users ADD COLUMN totp_pending TEXT;
ALTER TABLE auth_users ADD COLUMN totp_last_step INTEGER;

-- Single-use recovery codes, SHA-256 hex of the plaintext shown once at
-- enrollment. A used code keeps its row (used_at set) for the audit trail.
CREATE TABLE IF NOT EXISTS auth_recovery_codes (
    user_id    TEXT NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
    code_hash  TEXT NOT NULL,
    created_at TEXT NOT NULL,
    used_at    TEXT,
    PRIMARY KEY (user_id, code_hash)
);
