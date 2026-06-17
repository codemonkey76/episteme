use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

/// A recorded notification — the durable twin of a push notification.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Notification {
    pub id: String,
    pub category: String,
    pub title: String,
    pub body: String,
    pub link_kind: Option<String>,
    pub link_id: Option<String>,
    /// JSON payload for actionable notifications (e.g. a ticket reply draft).
    pub data: Option<String>,
    pub read_at: Option<String>,
    pub created_at: String,
}

pub async fn insert(
    pool: &SqlitePool,
    user_id: &str,
    category: &str,
    title: &str,
    body: &str,
    link_kind: Option<&str>,
    link_id: Option<&str>,
) -> Result<Notification> {
    insert_with_data(pool, user_id, category, title, body, link_kind, link_id, None).await
}

/// Like [`insert`] but with a structured `data` JSON payload the UI uses to
/// render action buttons (see the `data` column).
#[allow(clippy::too_many_arguments)]
pub async fn insert_with_data(
    pool: &SqlitePool,
    user_id: &str,
    category: &str,
    title: &str,
    body: &str,
    link_kind: Option<&str>,
    link_id: Option<&str>,
    data: Option<&str>,
) -> Result<Notification> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO notifications (id, user_id, category, title, body, link_kind, link_id, data, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(category)
    .bind(title)
    .bind(body)
    .bind(link_kind)
    .bind(link_id)
    .bind(data)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(Notification {
        id,
        category: category.to_string(),
        title: title.to_string(),
        body: body.to_string(),
        link_kind: link_kind.map(str::to_string),
        link_id: link_id.map(str::to_string),
        data: data.map(str::to_string),
        read_at: None,
        created_at: now,
    })
}

pub async fn list(pool: &SqlitePool, user_id: &str, limit: i64) -> Result<Vec<Notification>> {
    Ok(sqlx::query_as::<_, Notification>(
        "SELECT id, category, title, body, link_kind, link_id, data, read_at, created_at \
         FROM notifications WHERE user_id = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn unread_count(pool: &SqlitePool, user_id: &str) -> Result<i64> {
    let n: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM notifications WHERE user_id = ? AND read_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(n.0)
}

pub async fn mark_read(pool: &SqlitePool, user_id: &str, id: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE notifications SET read_at = ? WHERE id = ? AND user_id = ? AND read_at IS NULL")
        .bind(&now)
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_all_read(pool: &SqlitePool, user_id: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE notifications SET read_at = ? WHERE user_id = ? AND read_at IS NULL")
        .bind(&now)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, user_id: &str, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM notifications WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Clear all of a user's notifications. Returns how many were removed.
pub async fn clear_all(pool: &SqlitePool, user_id: &str) -> Result<u64> {
    let res = sqlx::query("DELETE FROM notifications WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    // The full lifecycle the Notifications window relies on: a recorded
    // notification (with a deep-link) is listed, counts as unread, links to its
    // source, then reads/clears as expected. Mirrors the scheduled-agent path,
    // where a finished job records an "agent" notification linked to its session.
    #[tokio::test]
    async fn record_list_read_and_clear() {
        let pool = db::init("sqlite::memory:").await.expect("migrations should run");

        // A briefing-style notification linked to a chat session.
        let n = insert(&pool, "u1", "agent", "Monday briefing", "12 emails, 3 need replies", Some("session"), Some("sess-42"))
            .await
            .unwrap();
        // A second, unlinked one for another user must stay isolated.
        insert(&pool, "u2", "info", "other", "not yours", None, None).await.unwrap();

        let items = list(&pool, "u1", 100).await.unwrap();
        assert_eq!(items.len(), 1, "only u1's notification is listed");
        assert_eq!(items[0].title, "Monday briefing");
        assert_eq!(items[0].category, "agent");
        assert_eq!(items[0].link_kind.as_deref(), Some("session"));
        assert_eq!(items[0].link_id.as_deref(), Some("sess-42"));
        assert!(items[0].read_at.is_none());

        assert_eq!(unread_count(&pool, "u1").await.unwrap(), 1);

        // Mark read → unread count drops, row remains.
        mark_read(&pool, "u1", &n.id).await.unwrap();
        assert_eq!(unread_count(&pool, "u1").await.unwrap(), 0);
        assert!(list(&pool, "u1", 100).await.unwrap()[0].read_at.is_some());

        // A user can't read or delete another user's notification.
        mark_read(&pool, "u1", "nope").await.unwrap(); // no-op, no error
        delete(&pool, "u1", "wrong-id").await.unwrap();
        assert_eq!(list(&pool, "u1", 100).await.unwrap().len(), 1);

        // Clear all removes only this user's.
        let removed = clear_all(&pool, "u1").await.unwrap();
        assert_eq!(removed, 1);
        assert!(list(&pool, "u1", 100).await.unwrap().is_empty());
        assert_eq!(list(&pool, "u2", 100).await.unwrap().len(), 1, "u2 untouched");
    }
}
