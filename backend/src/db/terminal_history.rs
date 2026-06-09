use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct HistoryEntry {
    pub id: String,
    pub user_id: String,
    pub shell: String,
    pub command: String,
    pub created_at: String,
}

/// Record a command run in a terminal. Skips blanks and an immediate duplicate
/// of the user's previous command in the same shell (so holding Enter or
/// re-running the last line doesn't spam the history).
pub async fn insert(pool: &SqlitePool, user_id: &str, shell: &str, command: &str) -> Result<()> {
    let command = command.trim();
    if command.is_empty() {
        return Ok(());
    }
    let last: Option<(String,)> = sqlx::query_as(
        "SELECT command FROM terminal_history
         WHERE user_id = ? AND shell = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(shell)
    .fetch_optional(pool)
    .await?;
    if last.as_ref().map(|(c,)| c.as_str()) == Some(command) {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO terminal_history (id, user_id, shell, command, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(shell)
    .bind(command)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Most recent commands for a user, newest first. Optionally filtered to a shell
/// and/or a substring search. `limit` caps the rows returned.
pub async fn list(
    pool: &SqlitePool,
    user_id: &str,
    shell: Option<&str>,
    search: Option<&str>,
    limit: i64,
) -> Result<Vec<HistoryEntry>> {
    let mut sql = String::from(
        "SELECT id, user_id, shell, command, created_at FROM terminal_history WHERE user_id = ?",
    );
    if shell.is_some() {
        sql.push_str(" AND shell = ?");
    }
    if search.is_some() {
        sql.push_str(" AND command LIKE ?");
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ?");

    let mut q = sqlx::query_as::<_, HistoryEntry>(&sql).bind(user_id);
    if let Some(s) = shell {
        q = q.bind(s.to_string());
    }
    if let Some(s) = search {
        q = q.bind(format!("%{s}%"));
    }
    q = q.bind(limit);
    Ok(q.fetch_all(pool).await?)
}
