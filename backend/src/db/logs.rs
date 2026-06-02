use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct LogEntry {
    pub id: String,
    pub ts: i64,
    pub category: String,
    pub level: String,
    pub message: String,
}

pub async fn insert(pool: &SqlitePool, entry: &LogEntry) -> Result<()> {
    sqlx::query(
        "INSERT INTO logs (id, ts, category, level, message) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&entry.id)
    .bind(entry.ts)
    .bind(&entry.category)
    .bind(&entry.level)
    .bind(&entry.message)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
    category: Option<&str>,
    level: Option<&str>,
    q: Option<&str>,
) -> Result<Vec<LogEntry>> {
    let mut sql = String::from(
        "SELECT id, ts, category, level, message FROM logs WHERE 1=1",
    );
    if category.is_some() { sql.push_str(" AND category = ?"); }
    if level.is_some()    { sql.push_str(" AND level = ?"); }
    if q.is_some()        { sql.push_str(" AND message LIKE ?"); }
    sql.push_str(" ORDER BY ts ASC LIMIT ? OFFSET ?");

    let mut qb = sqlx::query_as::<_, LogEntry>(&sql);
    if let Some(c) = category { qb = qb.bind(c); }
    if let Some(l) = level    { qb = qb.bind(l); }
    if let Some(s) = q        { qb = qb.bind(format!("%{s}%")); }
    qb = qb.bind(limit).bind(offset);

    Ok(qb.fetch_all(pool).await?)
}

pub async fn clear(pool: &SqlitePool) -> Result<()> {
    sqlx::query("DELETE FROM logs").execute(pool).await?;
    Ok(())
}
