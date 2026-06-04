-- Admin impersonation: a session may act as one user while recording the
-- admin who started it, so the UI can show a banner and offer a way back.
ALTER TABLE auth_sessions ADD COLUMN impersonator_id TEXT REFERENCES auth_users(id);
