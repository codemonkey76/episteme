-- Resumable deep research: a per-job snapshot of the gathered state (notes,
-- sources, evolving draft, budgets) saved after each round. If a run is
-- interrupted (e.g. a server restart), the orphan-recovery re-enqueue picks up
-- from the last completed round instead of starting over. Deleted on completion.
CREATE TABLE research_checkpoints (
    job_id      TEXT PRIMARY KEY,
    round       INTEGER NOT NULL,
    state       TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
