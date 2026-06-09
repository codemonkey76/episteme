//! Durable terminal scrollback: the full PTY output stream, batched into rows,
//! kept so a reconnected terminal can repaint its prior scrollback and the whole
//! archive stays searchable across restarts. See migration 024.

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;

/// Replay no more than this many bytes of saved scrollback on reconnect — enough
/// to feel continuous without flooding the client (full archive stays in the DB,
/// reachable via search).
const RESTORE_MAX_BYTES: usize = 256 * 1024;

/// Append one batched chunk of output for a terminal. `data` is the raw PTY
/// bytes (ANSI intact); `text` is the stripped form for search.
pub async fn append(
    pool: &SqlitePool,
    terminal_id: &str,
    user_id: &str,
    shell: &str,
    data: &[u8],
    text: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO terminal_output (terminal_id, user_id, shell, data, text, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(terminal_id)
    .bind(user_id)
    .bind(shell)
    .bind(data)
    .bind(text)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// The saved scrollback tail for a terminal, raw bytes in chronological order,
/// capped at `RESTORE_MAX_BYTES` (keeping the most recent output). Empty when
/// the terminal has no history yet. Scoped to `user_id` so a client can't
/// replay another user's scrollback by guessing/passing their terminal id.
pub async fn restore_tail(pool: &SqlitePool, user_id: &str, terminal_id: &str) -> Result<Vec<u8>> {
    // Newest first so we can stop once we have enough, then reverse to replay
    // in order.
    let rows: Vec<(Vec<u8>,)> = sqlx::query_as(
        "SELECT data FROM terminal_output WHERE user_id = ? AND terminal_id = ? ORDER BY id DESC",
    )
    .bind(user_id)
    .bind(terminal_id)
    .fetch_all(pool)
    .await?;

    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut total = 0usize;
    for (data,) in rows {
        total += data.len();
        chunks.push(data);
        if total >= RESTORE_MAX_BYTES {
            break;
        }
    }
    chunks.reverse();
    Ok(chunks.concat())
}

/// One search hit: a chunk whose stripped text matched, with a short context
/// snippet (the matching line plus a little surrounding context).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct OutputHit {
    pub terminal_id: String,
    pub shell: String,
    pub created_at: String,
    pub snippet: String,
}

/// Search a user's whole terminal archive (case-insensitive substring). Returns
/// the most recent matches first, each with a context snippet around the match.
pub async fn search(
    pool: &SqlitePool,
    user_id: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<OutputHit>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let like = format!("%{}%", escape_like(query));
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT terminal_id, shell, created_at, text FROM terminal_output
         WHERE user_id = ? AND text LIKE ? ESCAPE '\\' ORDER BY id DESC LIMIT ?",
    )
    .bind(user_id)
    .bind(like)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let needle = query.to_lowercase();
    Ok(rows
        .into_iter()
        .map(|(terminal_id, shell, created_at, text)| OutputHit {
            terminal_id,
            shell,
            created_at,
            snippet: snippet_around(&text, &needle),
        })
        .collect())
}

/// Up to 5 lines of context around the first matching line in `text`.
fn snippet_around(text: &str, needle: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let hit = lines
        .iter()
        .position(|l| l.to_lowercase().contains(needle))
        .unwrap_or(0);
    let start = hit.saturating_sub(2);
    let end = (hit + 3).min(lines.len());
    lines[start..end].join("\n").trim_end().to_string()
}

/// Escape LIKE wildcards in a user query so `%`/`_`/`\` are matched literally.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}
