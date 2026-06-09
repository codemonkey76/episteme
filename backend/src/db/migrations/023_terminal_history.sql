-- Persistent, searchable command history for the in-app terminals. One row per
-- command the user runs in a bash/pwsh terminal window (captured via the shell
-- integration's OSC 633 sequence). Survives restarts; searched with LIKE.
CREATE TABLE terminal_history (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    shell TEXT NOT NULL,
    command TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_terminal_history_user_shell ON terminal_history(user_id, shell, created_at);
