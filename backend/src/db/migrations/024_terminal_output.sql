-- Durable, searchable terminal SCROLLBACK for the in-app terminals. Unlike
-- terminal_history (one row per command), this stores the full output stream so
-- a refreshed/reconnected terminal can repaint its prior scrollback (display
-- only — never replayed into the shell) and the whole archive stays searchable
-- across server restarts. One row per batched chunk of PTY output.
--   data: raw PTY bytes (ANSI intact) for faithful, full-colour replay.
--   text: the same bytes with ANSI/escape sequences stripped, for LIKE search.
CREATE TABLE terminal_output (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    terminal_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    shell TEXT NOT NULL,
    data BLOB NOT NULL,
    text TEXT NOT NULL,
    created_at TEXT NOT NULL
);
-- Replay: fetch a terminal's tail in order. Search: scan a user's archive.
CREATE INDEX idx_terminal_output_terminal ON terminal_output(terminal_id, id);
CREATE INDEX idx_terminal_output_user ON terminal_output(user_id, id);
