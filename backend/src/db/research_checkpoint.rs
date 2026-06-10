//! Per-job research checkpoint — the gathered state saved after each round so an
//! interrupted run resumes instead of restarting. See migration 027 and
//! `research::run`.

use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;

/// Save (upsert) the checkpoint for a job at `round` with serialized `state`.
pub async fn save(pool: &SqlitePool, job_id: &str, round: i64, state: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO research_checkpoints (job_id, round, state, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(job_id) DO UPDATE
           SET round = excluded.round, state = excluded.state, updated_at = excluded.updated_at",
    )
    .bind(job_id)
    .bind(round)
    .bind(state)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// The saved (round, state) for a job, if any.
pub async fn load(pool: &SqlitePool, job_id: &str) -> Result<Option<(i64, String)>> {
    Ok(sqlx::query_as::<_, (i64, String)>(
        "SELECT round, state FROM research_checkpoints WHERE job_id = ?",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn delete(pool: &SqlitePool, job_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM research_checkpoints WHERE job_id = ?")
        .bind(job_id)
        .execute(pool)
        .await?;
    Ok(())
}
