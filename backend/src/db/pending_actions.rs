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
    })
}

pub async fn list_pending(pool: &SqlitePool, session_id: &str) -> Result<Vec<PendingAction>> {
    let rows = sqlx::query_as::<_, PendingAction>(
        "SELECT id, session_id, tool_name, tool_args, status, created_at, resolved_at
         FROM pending_actions WHERE session_id = ? AND status = 'pending'
         ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
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
