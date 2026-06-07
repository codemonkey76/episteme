use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PendingAction {
    pub id: String,
    pub session_id: String,
    pub tool_name: String,
    pub tool_args: String,
    pub status: String,
    pub created_at: String,
    pub resolved_at: Option<String>,
    /// The model's tool-call id — set for PARKED actions (unattended runs) so
    /// the tool-result row can be written when the action is decided. NULL for
    /// live-chat approvals, which execute inside the still-running turn.
    #[sqlx(default)]
    pub call_id: Option<String>,
}

pub async fn insert(
    pool: &SqlitePool,
    session_id: &str,
    tool_name: &str,
    tool_args: &str,
) -> Result<PendingAction> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO pending_actions (id, session_id, tool_name, tool_args, status, created_at)
         VALUES (?, ?, ?, ?, 'pending', ?)",
    )
    .bind(&id)
    .bind(session_id)
    .bind(tool_name)
    .bind(tool_args)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(PendingAction {
        id,
        session_id: session_id.to_string(),
        tool_name: tool_name.to_string(),
        tool_args: tool_args.to_string(),
        status: "pending".to_string(),
        created_at: now,
        resolved_at: None,
        call_id: None,
    })
}

/// Park a gated tool call from an unattended run: like `insert`, but records
/// the call id so the decision handler can execute it later.
pub async fn insert_parked(
    pool: &SqlitePool,
    session_id: &str,
    tool_name: &str,
    tool_args: &str,
    call_id: &str,
) -> Result<PendingAction> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO pending_actions (id, session_id, tool_name, tool_args, status, created_at, call_id)
         VALUES (?, ?, ?, ?, 'pending', ?, ?)",
    )
    .bind(&id)
    .bind(session_id)
    .bind(tool_name)
    .bind(tool_args)
    .bind(&now)
    .bind(call_id)
    .execute(pool)
    .await?;

    Ok(PendingAction {
        id,
        session_id: session_id.to_string(),
        tool_name: tool_name.to_string(),
        tool_args: tool_args.to_string(),
        status: "pending".to_string(),
        created_at: now,
        resolved_at: None,
        call_id: Some(call_id.to_string()),
    })
}

pub async fn list_pending(pool: &SqlitePool, session_id: &str) -> Result<Vec<PendingAction>> {
    let rows = sqlx::query_as::<_, PendingAction>(
        "SELECT id, session_id, tool_name, tool_args, status, created_at, resolved_at, call_id
         FROM pending_actions WHERE session_id = ? AND status = 'pending'
         ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Does this pending action belong to one of the user's sessions?
pub async fn owned_by(pool: &SqlitePool, id: &str, user_id: &str) -> Result<bool> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pending_actions p
         JOIN sessions s ON s.id = p.session_id
         WHERE p.id = ? AND s.user_id = ?",
    )
    .bind(id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(n > 0)
}

pub async fn resolve(pool: &SqlitePool, id: &str, approved: bool) -> Result<()> {
    let status = if approved { "approved" } else { "rejected" };
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE pending_actions SET status = ?, resolved_at = ? WHERE id = ?",
    )
    .bind(status)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Reject every still-pending action for a session — used when its job is
/// cancelled, so the global approval queue stops showing orphaned rows.
pub async fn clear_for_session(pool: &SqlitePool, session_id: &str) -> Result<()> {
    sqlx::query(
        "UPDATE pending_actions SET status = 'rejected', resolved_at = ?
         WHERE session_id = ? AND status = 'pending'",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<PendingAction>> {
    Ok(sqlx::query_as::<_, PendingAction>(
        "SELECT id, session_id, tool_name, tool_args, status, created_at, resolved_at, call_id
         FROM pending_actions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// Pending rows still blocking this session's suspended run.
pub async fn count_pending(pool: &SqlitePool, session_id: &str) -> Result<i64> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pending_actions WHERE session_id = ? AND status = 'pending'",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await?;
    Ok(n)
}

/// A pending action joined with its session title, for the global queue UI.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PendingActionWithSession {
    pub id: String,
    pub session_id: String,
    pub session_title: String,
    pub tool_name: String,
    pub tool_args: String,
    pub created_at: String,
}

/// Every pending action across all of the user's sessions, oldest first.
pub async fn list_pending_for_user(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<PendingActionWithSession>> {
    Ok(sqlx::query_as::<_, PendingActionWithSession>(
        "SELECT p.id, p.session_id, s.title AS session_title, p.tool_name, p.tool_args, p.created_at
         FROM pending_actions p
         JOIN sessions s ON s.id = p.session_id
         WHERE s.user_id = ? AND p.status = 'pending'
         ORDER BY p.created_at ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}
