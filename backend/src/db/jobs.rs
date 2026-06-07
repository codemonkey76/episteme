use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

/// A tracked agent run: a chat-triggered background task or a scheduled
/// agent fire. The session holds the transcript; this row holds the lifecycle.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Job {
    pub id: String,
    pub user_id: String,
    pub session_id: String,
    /// 'background' (chat-triggered) or 'scheduled'.
    pub kind: String,
    pub name: String,
    /// Provider name; resolved against settings at (re)run time.
    pub provider: String,
    /// running | needs_approval | done | failed
    pub status: String,
    pub summary: Option<String>,
    pub error: Option<String>,
    /// Small JSON blob for kind-specific settings (research: {"topic","depth"}).
    pub meta: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

const COLS: &str = "id, user_id, session_id, kind, name, provider, status, summary, error, meta, created_at, updated_at";

pub async fn insert(
    pool: &SqlitePool,
    user_id: &str,
    session_id: &str,
    kind: &str,
    name: &str,
    provider: &str,
    meta: Option<&str>,
) -> Result<Job> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO jobs (id, user_id, session_id, kind, name, provider, status, meta, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'running', ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(session_id)
    .bind(kind)
    .bind(name)
    .bind(provider)
    .bind(meta)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(Job {
        id,
        user_id: user_id.to_string(),
        session_id: session_id.to_string(),
        kind: kind.to_string(),
        name: name.to_string(),
        provider: provider.to_string(),
        status: "running".to_string(),
        summary: None,
        error: None,
        meta: meta.map(str::to_string),
        created_at: now.clone(),
        updated_at: now,
    })
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<Job>> {
    Ok(sqlx::query_as::<_, Job>(&format!("SELECT {COLS} FROM jobs WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub async fn list_for_user(pool: &SqlitePool, user_id: &str, limit: i64) -> Result<Vec<Job>> {
    Ok(sqlx::query_as::<_, Job>(&format!(
        "SELECT {COLS} FROM jobs WHERE user_id = ? ORDER BY created_at DESC LIMIT ?"
    ))
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn set_status(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    summary: Option<&str>,
    error: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE jobs SET status = ?, summary = ?, error = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(summary)
        .bind(error)
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Fail every job still marked `running` — called once at startup. A running
/// job's worker task lives in process memory, so any survivor of a restart is
/// orphaned: it will never progress and would sit in the UI forever. Returns
/// how many were swept.
pub async fn fail_orphaned_running(pool: &SqlitePool) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE jobs SET status = 'failed',
                error = 'interrupted by a server restart',
                updated_at = ?
         WHERE status = 'running'",
    )
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// User-initiated cancel: fail a job the user owns that's still in flight
/// (running or awaiting approval). Returns false if it wasn't cancellable
/// (already finished, or not theirs). Pending approvals for the session are
/// cleared so the global queue doesn't keep showing them.
pub async fn cancel(pool: &SqlitePool, user_id: &str, id: &str) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE jobs SET status = 'failed', error = 'cancelled', updated_at = ?
         WHERE id = ? AND user_id = ? AND status IN ('running', 'needs_approval')",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

/// The session's suspended job, if any.
pub async fn suspended_for_session(pool: &SqlitePool, session_id: &str) -> Result<Option<Job>> {
    Ok(sqlx::query_as::<_, Job>(&format!(
        "SELECT {COLS} FROM jobs WHERE session_id = ? AND status = 'needs_approval'"
    ))
    .bind(session_id)
    .fetch_optional(pool)
    .await?)
}

/// Atomic spawn gate for racing approvals: flip needs_approval → running and
/// report whether THIS caller won. Exactly one winner per suspension.
pub async fn try_resume(pool: &SqlitePool, id: &str) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE jobs SET status = 'running', updated_at = ? WHERE id = ? AND status = 'needs_approval'",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}
