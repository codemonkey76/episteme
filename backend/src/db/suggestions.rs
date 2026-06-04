use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Suggestion {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub start_at: Option<String>,
    pub end_at: Option<String>,
    pub context: Option<String>,
    pub status: String,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

pub async fn insert(
    pool: &SqlitePool,
    kind: &str,
    title: &str,
    start_at: Option<&str>,
    end_at: Option<&str>,
    context: Option<&str>,
) -> Result<Suggestion> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO suggestions (id, kind, title, start_at, end_at, context, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, 'pending', ?)",
    )
    .bind(&id)
    .bind(kind)
    .bind(title)
    .bind(start_at)
    .bind(end_at)
    .bind(context)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(Suggestion {
        id,
        kind: kind.to_string(),
        title: title.to_string(),
        start_at: start_at.map(str::to_string),
        end_at: end_at.map(str::to_string),
        context: context.map(str::to_string),
        status: "pending".to_string(),
        created_at: now,
        resolved_at: None,
    })
}

pub async fn list_pending(pool: &SqlitePool) -> Result<Vec<Suggestion>> {
    Ok(sqlx::query_as::<_, Suggestion>(
        "SELECT id, kind, title, start_at, end_at, context, status, created_at, resolved_at
         FROM suggestions WHERE status = 'pending' ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<Suggestion>> {
    Ok(sqlx::query_as::<_, Suggestion>(
        "SELECT id, kind, title, start_at, end_at, context, status, created_at, resolved_at
         FROM suggestions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn resolve(pool: &SqlitePool, id: &str, status: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE suggestions SET status = ?, resolved_at = ? WHERE id = ?")
        .bind(status)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
