use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Memory {
    pub id: String,
    pub content: String,
    pub category: String,
    pub source: String,
    pub session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn insert(
    pool: &SqlitePool,
    content: &str,
    category: &str,
    source: &str,
    session_id: Option<&str>,
) -> Result<Memory> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO memories (id, content, category, source, session_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(content)
    .bind(category)
    .bind(source)
    .bind(session_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(Memory {
        id,
        content: content.to_string(),
        category: category.to_string(),
        source: source.to_string(),
        session_id: session_id.map(str::to_string),
        created_at: now.clone(),
        updated_at: now,
    })
}

/// List memories newest-first, optionally filtered by category and/or a content
/// substring. Mirrors the dynamic-WHERE approach in `db::logs::list`.
pub async fn list(
    pool: &SqlitePool,
    category: Option<&str>,
    q: Option<&str>,
    limit: i64,
) -> Result<Vec<Memory>> {
    let mut sql = String::from(
        "SELECT id, content, category, source, session_id, created_at, updated_at
         FROM memories WHERE 1=1",
    );
    if category.is_some() { sql.push_str(" AND category = ?"); }
    if q.is_some()        { sql.push_str(" AND content LIKE ?"); }
    sql.push_str(" ORDER BY created_at DESC LIMIT ?");

    let mut qb = sqlx::query_as::<_, Memory>(&sql);
    if let Some(c) = category { qb = qb.bind(c.to_string()); }
    if let Some(s) = q        { qb = qb.bind(format!("%{s}%")); }
    qb = qb.bind(limit);

    Ok(qb.fetch_all(pool).await?)
}

/// Newest `limit` memories — used to build the system-prompt injection.
pub async fn list_recent(pool: &SqlitePool, limit: i64) -> Result<Vec<Memory>> {
    list(pool, None, None, limit).await
}

pub async fn update(pool: &SqlitePool, id: &str, content: &str, category: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE memories SET content = ?, category = ?, updated_at = ? WHERE id = ?",
    )
    .bind(content)
    .bind(category)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM memories WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
