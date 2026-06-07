use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub tool_calls: Option<String>,
    pub tool_call_id: Option<String>,
    pub created_at: String,
}

pub async fn insert(
    pool: &SqlitePool,
    session_id: &str,
    role: &str,
    content: &str,
    tool_calls: Option<&str>,
    tool_call_id: Option<&str>,
) -> Result<Message> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO messages (id, session_id, role, content, tool_calls, tool_call_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(session_id)
    .bind(role)
    .bind(content)
    .bind(tool_calls)
    .bind(tool_call_id)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(Message {
        id,
        session_id: session_id.to_string(),
        role: role.to_string(),
        content: content.to_string(),
        tool_calls: tool_calls.map(str::to_string),
        tool_call_id: tool_call_id.map(str::to_string),
        created_at: now,
    })
}

pub async fn list_for_session(pool: &SqlitePool, session_id: &str) -> Result<Vec<Message>> {
    let rows = sqlx::query_as::<_, Message>(
        "SELECT id, session_id, role, content, tool_calls, tool_call_id, created_at
         FROM messages WHERE session_id = ? ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Messages newer than the compaction cutoff (all of them when None) — the
/// model-facing slice of the transcript. Timestamps are RFC3339 from a single
/// writer, so the string comparison is chronological.
pub async fn list_for_session_after(
    pool: &SqlitePool,
    session_id: &str,
    after: Option<&str>,
) -> Result<Vec<Message>> {
    let rows = sqlx::query_as::<_, Message>(
        "SELECT id, session_id, role, content, tool_calls, tool_call_id, created_at
         FROM messages WHERE session_id = ? AND (? IS NULL OR created_at > ?)
         ORDER BY created_at ASC",
    )
    .bind(session_id)
    .bind(after)
    .bind(after)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// One full-text search hit, with enough context to jump to the session.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SearchHit {
    pub session_id: String,
    pub session_title: String,
    pub message_id: String,
    pub role: String,
    /// FTS5-generated snippet around the match, '…'-elided.
    pub snippet: String,
    pub created_at: String,
}

/// Quote each whitespace-separated term so user input can't break FTS5 query
/// syntax (implicit AND between terms; embedded quotes are stripped).
fn fts_query(q: &str) -> String {
    q.split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Full-text search across the user's sessions, best matches first.
pub async fn search(
    pool: &SqlitePool,
    user_id: &str,
    q: &str,
    limit: i64,
) -> Result<Vec<SearchHit>> {
    let query = fts_query(q);
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, SearchHit>(
        "SELECT m.session_id, s.title AS session_title, m.id AS message_id, m.role,
                snippet(message_fts, 2, '', '', '…', 12) AS snippet,
                m.created_at
         FROM message_fts f
         JOIN messages m ON m.id = f.message_id
         JOIN sessions s ON s.id = m.session_id
         WHERE message_fts MATCH ? AND s.user_id = ?
         ORDER BY rank
         LIMIT ?",
    )
    .bind(&query)
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_query_quotes_terms_and_strips_embedded_quotes() {
        assert_eq!(fts_query("hello world"), "\"hello\" \"world\"");
        assert_eq!(fts_query("a\"b OR *"), "\"ab\" \"OR\" \"*\"");
        assert_eq!(fts_query("   "), "");
    }
}
