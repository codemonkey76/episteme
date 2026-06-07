use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Report listing row — the HTML body is fetched separately (it can be MBs
/// with embedded images).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ReportMeta {
    pub id: String,
    pub job_id: Option<String>,
    pub title: String,
    pub created_at: String,
    /// Opaque public-share token; set = anyone with the link can view it.
    pub share_token: Option<String>,
}

pub async fn insert(
    pool: &SqlitePool,
    user_id: &str,
    job_id: Option<&str>,
    title: &str,
    html: &str,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO reports (id, user_id, job_id, title, html, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(job_id)
    .bind(title)
    .bind(html)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn list_for_user(pool: &SqlitePool, user_id: &str, limit: i64) -> Result<Vec<ReportMeta>> {
    Ok(sqlx::query_as::<_, ReportMeta>(
        "SELECT id, job_id, title, created_at, share_token FROM reports
         WHERE user_id = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn get_html(pool: &SqlitePool, user_id: &str, id: &str) -> Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT html FROM reports WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(h,)| h))
}

/// This report's current share token for its owner: Outer None = no such
/// report (or not theirs); Some(None) = private; Some(Some(t)) = shared.
pub async fn share_token(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
) -> Result<Option<Option<String>>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT share_token FROM reports WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(t,)| t))
}

/// Set or clear the share token (owner-scoped). Returns whether a row matched.
pub async fn set_share_token(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
    token: Option<&str>,
) -> Result<bool> {
    let res = sqlx::query("UPDATE reports SET share_token = ? WHERE id = ? AND user_id = ?")
        .bind(token)
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() == 1)
}

/// Public lookup by share token (no account): the report's (title, html).
pub async fn get_shared(pool: &SqlitePool, token: &str) -> Result<Option<(String, String)>> {
    Ok(sqlx::query_as("SELECT title, html FROM reports WHERE share_token = ?")
        .bind(token)
        .fetch_optional(pool)
        .await?)
}

pub async fn delete(pool: &SqlitePool, user_id: &str, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM reports WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
