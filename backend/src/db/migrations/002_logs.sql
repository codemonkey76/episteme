CREATE TABLE IF NOT EXISTS logs (
    id          TEXT PRIMARY KEY,
    ts          INTEGER NOT NULL,   -- unix milliseconds
    category    TEXT NOT NULL,
    level       TEXT NOT NULL CHECK(level IN ('debug', 'info', 'warn', 'error')),
    message     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS logs_ts ON logs(ts DESC);
