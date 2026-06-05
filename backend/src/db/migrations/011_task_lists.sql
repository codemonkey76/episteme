-- Named to-do lists. A task references one via list_id; NULL means the
-- implicit "General" list, so pre-existing tasks need no backfill and the
-- default list can never be deleted out from under them.
CREATE TABLE IF NOT EXISTS task_lists (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_task_lists_user ON task_lists(user_id);

ALTER TABLE tasks ADD COLUMN list_id TEXT;

CREATE INDEX IF NOT EXISTS idx_tasks_list ON tasks(list_id);
