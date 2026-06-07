use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;

/// One indexed email: enough metadata to show a search hit (the body stays in
/// the mailbox — `email_read` fetches it live by id).
#[derive(Debug, sqlx::FromRow)]
pub struct IndexedEmail {
    pub message_id: String,
    pub subject: String,
    pub sender: String,
    pub snippet: String,
    pub received_at: String,
    pub embedding: Vec<u8>,
}

/// Insert one embedded email; an already-indexed id is left untouched.
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &SqlitePool,
    user_id: &str,
    message_id: &str,
    mailbox: &str,
    subject: &str,
    sender: &str,
    snippet: &str,
    received_at: &str,
    embedding: &[u8],
) -> Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO email_embeddings
         (user_id, message_id, mailbox, subject, sender, snippet, received_at, embedding, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(message_id)
    .bind(mailbox)
    .bind(subject)
    .bind(sender)
    .bind(snippet)
    .bind(received_at)
    .bind(embedding)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Of the given ids, the ones already indexed — so the indexer skips the
/// embedding call for mail it has seen before.
pub async fn existing_ids(
    pool: &SqlitePool,
    user_id: &str,
    ids: &[&str],
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    // Few enough per batch (≤50) that per-id lookups beat building dynamic IN().
    for id in ids {
        let found: Option<(String,)> = sqlx::query_as(
            "SELECT message_id FROM email_embeddings WHERE user_id = ? AND message_id = ?",
        )
        .bind(user_id)
        .bind(id)
        .fetch_optional(pool)
        .await?;
        if let Some((id,)) = found {
            out.push(id);
        }
    }
    Ok(out)
}

/// All indexed mail for a mailbox, newest first — brute-force cosine over
/// these is plenty fast into the tens of thousands of rows (same call as
/// memories/documents).
pub async fn list_for_mailbox(
    pool: &SqlitePool,
    user_id: &str,
    mailbox: &str,
) -> Result<Vec<IndexedEmail>> {
    let rows = sqlx::query_as::<_, IndexedEmail>(
        "SELECT message_id, subject, sender, snippet, received_at, embedding
         FROM email_embeddings WHERE user_id = ? AND mailbox = ?
         ORDER BY received_at DESC",
    )
    .bind(user_id)
    .bind(mailbox)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Bound the index: drop the oldest rows beyond `keep` for this user.
pub async fn prune(pool: &SqlitePool, user_id: &str, keep: i64) -> Result<()> {
    sqlx::query(
        "DELETE FROM email_embeddings WHERE user_id = ? AND message_id NOT IN (
            SELECT message_id FROM email_embeddings WHERE user_id = ?
            ORDER BY received_at DESC LIMIT ?
         )",
    )
    .bind(user_id)
    .bind(user_id)
    .bind(keep)
    .execute(pool)
    .await?;
    Ok(())
}
