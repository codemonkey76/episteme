//! Semantic email index. The auto-sort worker already fetches every incoming
//! message's metadata on its polling interval; this module rides along,
//! embedding `from + subject + preview` (local Ollama, same Phase-2 infra as
//! memories/documents) into `email_embeddings`. `email_search` then matches
//! by meaning, merging cosine hits into Graph's keyword results.
//!
//! Strictly best-effort: if Ollama is down a batch is simply skipped — the
//! categorizer indexes whatever it fetches, so coverage is "mail seen by
//! auto-sort", not a guaranteed archive.

use anyhow::Result;
use serde_json::Value;
use sqlx::SqlitePool;

use crate::db;
use crate::integrations::embeddings;
use crate::state::AppState;

/// Rows kept per user — at ~3 KB an embedding this caps the index near 60 MB.
const KEEP_NEWEST: i64 = 20_000;
/// Minimum cosine for a semantic hit: below this the match is noise and only
/// Graph's keyword results are returned.
const MIN_SCORE: f32 = 0.5;

/// Index a batch of Graph message summaries (the auto-sort fetch shape:
/// id/subject/from/bodyPreview/receivedDateTime). Already-indexed ids are
/// skipped without an embedding call. Designed for `tokio::spawn` — takes the
/// pool + client (both cheap clones, the `embeddings::embed` pattern) so the
/// detached task owns its captures; logs failures, never blocks the sort run.
pub async fn index_messages(
    pool: SqlitePool,
    client: reqwest::Client,
    user_id: String,
    mailbox: String,
    messages: Vec<Value>,
) {
    if let Err(e) = index_inner(&pool, &client, &user_id, &mailbox, &messages).await {
        tracing::warn!("email indexing skipped: {e}");
    }
}

async fn index_inner(
    pool: &SqlitePool,
    client: &reqwest::Client,
    user_id: &str,
    mailbox: &str,
    messages: &[Value],
) -> Result<()> {
    let ids: Vec<&str> = messages.iter().filter_map(|m| m["id"].as_str()).collect();
    if ids.is_empty() {
        return Ok(());
    }
    let seen = db::email_index::existing_ids(pool, user_id, &ids).await?;
    let mut indexed = 0usize;

    for m in messages {
        let Some(id) = m["id"].as_str() else { continue };
        if seen.iter().any(|s| s == id) {
            continue;
        }
        let subject = m["subject"].as_str().unwrap_or("(no subject)");
        let name = m["from"]["emailAddress"]["name"].as_str().unwrap_or("");
        let addr = m["from"]["emailAddress"]["address"].as_str().unwrap_or("");
        let sender = if name.is_empty() { addr.to_string() } else { format!("{name} <{addr}>") };
        let snippet: String = m["bodyPreview"].as_str().unwrap_or("").chars().take(300).collect();
        let received = m["receivedDateTime"].as_str().unwrap_or("");

        // From/subject/preview is what a human skims to judge relevance —
        // the same signal works for the embedding.
        let text = format!("From: {sender}\nSubject: {subject}\n{snippet}");
        // First failure aborts the batch (Ollama down) — next run retries,
        // since only successfully embedded ids land in the table.
        let vec = embeddings::embed(pool, client, &text).await?;
        db::email_index::insert(
            pool,
            user_id,
            id,
            mailbox,
            subject,
            &sender,
            &snippet,
            received,
            &embeddings::to_blob(&vec),
        )
        .await?;
        indexed += 1;
    }

    if indexed > 0 {
        db::email_index::prune(pool, user_id, KEEP_NEWEST).await?;
        tracing::debug!("email index: embedded {indexed} new message(s)");
    }
    Ok(())
}

/// One semantic search hit, ready for the email_search tool result.
pub struct Hit {
    pub message_id: String,
    pub subject: String,
    pub sender: String,
    pub snippet: String,
    pub received_at: String,
}

/// Meaning-based search over the indexed mail of one mailbox: embed the
/// query, brute-force cosine, top-`limit` above the noise floor. Errors
/// bubble up — the caller treats semantic results as a best-effort bonus.
pub async fn search(
    state: &AppState,
    user_id: &str,
    mailbox: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<Hit>> {
    let rows = db::email_index::list_for_mailbox(&state.db, user_id, mailbox).await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let qvec = embeddings::embed(&state.db, &state.http_client, query).await?;

    let mut scored: Vec<(f32, db::email_index::IndexedEmail)> = rows
        .into_iter()
        .map(|r| (embeddings::cosine(&qvec, &embeddings::from_blob(&r.embedding)), r))
        .filter(|(score, _)| *score >= MIN_SCORE)
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);

    Ok(scored
        .into_iter()
        .map(|(_, r)| Hit {
            message_id: r.message_id,
            subject: r.subject,
            sender: r.sender,
            snippet: r.snippet,
            received_at: r.received_at,
        })
        .collect())
}
