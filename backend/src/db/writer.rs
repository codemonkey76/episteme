use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

/// A long-form markdown document edited in the Writer window.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WriterDoc {
    pub id: String,
    pub title: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn insert(pool: &SqlitePool, user_id: &str, title: &str, content: &str) -> Result<WriterDoc> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO writer_docs (id, user_id, title, content, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(title)
    .bind(content)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(WriterDoc {
        id,
        title: title.to_string(),
        content: content.to_string(),
        created_at: now.clone(),
        updated_at: now,
    })
}

/// List documents most-recently-updated first, optionally filtered by a
/// title/content substring.
pub async fn list(pool: &SqlitePool, user_id: &str, q: Option<&str>, limit: i64) -> Result<Vec<WriterDoc>> {
    let mut sql = String::from(
        "SELECT id, title, content, created_at, updated_at FROM writer_docs WHERE user_id = ?",
    );
    if q.is_some() { sql.push_str(" AND (title LIKE ? OR content LIKE ?)"); }
    sql.push_str(" ORDER BY updated_at DESC LIMIT ?");

    let mut qb = sqlx::query_as::<_, WriterDoc>(&sql);
    qb = qb.bind(user_id);
    if let Some(s) = q {
        let like = format!("%{s}%");
        qb = qb.bind(like.clone()).bind(like);
    }
    qb = qb.bind(limit);

    Ok(qb.fetch_all(pool).await?)
}

pub async fn get(pool: &SqlitePool, user_id: &str, id: &str) -> Result<Option<WriterDoc>> {
    Ok(sqlx::query_as::<_, WriterDoc>(
        "SELECT id, title, content, created_at, updated_at FROM writer_docs WHERE id = ? AND user_id = ?",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?)
}

/// Apply a partial update and return the resulting row (None if id unknown).
pub async fn update(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
    title: Option<String>,
    content: Option<String>,
) -> Result<Option<WriterDoc>> {
    let Some(mut doc) = get(pool, user_id, id).await? else {
        return Ok(None);
    };
    if let Some(t) = title { doc.title = t; }
    if let Some(c) = content { doc.content = c; }
    doc.updated_at = Utc::now().to_rfc3339();

    sqlx::query("UPDATE writer_docs SET title = ?, content = ?, updated_at = ? WHERE id = ?")
        .bind(&doc.title)
        .bind(&doc.content)
        .bind(&doc.updated_at)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(Some(doc))
}

pub async fn delete(pool: &SqlitePool, user_id: &str, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM writer_docs WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
