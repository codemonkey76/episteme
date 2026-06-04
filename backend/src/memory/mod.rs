//! Persistent cross-session memory for the chat agent.
//!
//! Two responsibilities:
//! - [`inject`] prepends a system message of stored memories to a turn's history
//!   so the model personalizes and stays consistent across sessions.
//! - [`extract`] runs (detached, best-effort) after a turn to pull durable
//!   facts/preferences out of the exchange and persist them.

use serde::Deserialize;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db;
use crate::db::logs::LogEntry;
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig};
use crate::state::AppState;

/// Cap on memories injected into a turn's context.
const INJECT_LIMIT: i64 = 50;
/// Recent memories considered when deduping a freshly-extracted one.
const DEDUP_LIMIT: i64 = 200;

const VALID_CATEGORIES: [&str; 6] = ["preference", "fact", "feedback", "project", "style", "other"];

/// Prepend a system message listing stored memories, if any exist.
pub async fn inject(history: &mut Vec<ChatMessage>, pool: &SqlitePool, user_id: &str) {
    let memories = match db::memories::list_recent(pool, user_id, INJECT_LIMIT).await {
        Ok(m) if !m.is_empty() => m,
        _ => return,
    };

    let mut text = crate::prompts::get(pool, "memory_inject").await;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    for m in &memories {
        text.push_str(&format!("- [{}] {}\n", m.category, m.content));
    }

    history.insert(0, ChatMessage { role: "system".to_string(), content: Value::String(text) });
}

#[derive(Debug, Deserialize)]
struct Extracted {
    content: String,
    #[serde(default)]
    category: String,
}

/// Best-effort extraction of memories from a finished exchange. Errors are
/// logged and swallowed — this must never affect the chat turn.
pub async fn extract(
    state: &AppState,
    user_id: &str,
    provider: ProviderConfig,
    user_text: String,
    assistant_text: String,
    session_id: Option<String>,
) {
    if user_text.trim().is_empty() {
        return;
    }

    let user = format!(
        "Exchange to analyze:\n\nUser: {user_text}\n\nAssistant: {assistant_text}"
    );
    let system = crate::prompts::get(&state.db, "memory_extract").await;
    let history = vec![
        ChatMessage { role: "system".to_string(), content: Value::String(system) },
        ChatMessage { role: "user".to_string(), content: Value::String(user) },
    ];

    let raw = match ModelRouter::complete(&provider, history).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("memory extraction failed: {e}");
            return;
        }
    };

    let items = match parse(&raw) {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!("memory extraction parse failed: {e}");
            return;
        }
    };
    save_extracted(state, user_id, items, session_id.as_deref(), None).await;
}

/// Validate, dedup, and persist extracted memories. `force_category` pins
/// every item to one category (used by the style path) instead of trusting
/// the model's per-item categorization.
async fn save_extracted(
    state: &AppState,
    user_id: &str,
    items: Vec<Extracted>,
    session_id: Option<&str>,
    force_category: Option<&str>,
) {
    if items.is_empty() {
        return;
    }

    // Pull recent memories once for dedup.
    let existing =
        db::memories::list_recent(&state.db, user_id, DEDUP_LIMIT).await.unwrap_or_default();

    for item in items {
        let content = item.content.trim();
        if content.is_empty() {
            continue;
        }
        if is_duplicate(content, &existing) {
            continue;
        }
        let category = match force_category {
            Some(c) => c.to_string(),
            None => normalize_category(&item.category),
        };

        match db::memories::insert(&state.db, user_id, content, &category, "auto", session_id).await {
            Ok(_) => log_event(state, format!("Remembered [{category}]: {content}")),
            Err(e) => tracing::warn!("failed to save memory: {e}"),
        }
    }
}

/// Cap on draft/sent text fed to the style-extraction prompt.
const STYLE_TEXT_LIMIT: usize = 4000;

fn truncate_chars(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        s.to_string()
    } else {
        s.chars().take(limit).collect()
    }
}

/// Best-effort extraction of writing-style lessons from an edited AI draft.
/// Detached and swallowed like `extract` — must never affect the email send.
pub async fn extract_style(
    state: &AppState,
    user_id: &str,
    provider: ProviderConfig,
    draft: String,
    sent: String,
) {
    if draft.trim().is_empty() || draft.trim() == sent.trim() {
        return;
    }

    let user = format!(
        "AI DRAFT:\n{}\n\nWHAT THE USER ACTUALLY SENT:\n{}",
        truncate_chars(&draft, STYLE_TEXT_LIMIT),
        truncate_chars(&sent, STYLE_TEXT_LIMIT),
    );
    let system = crate::prompts::get(&state.db, "style_extract").await;
    let history = vec![
        ChatMessage { role: "system".to_string(), content: Value::String(system) },
        ChatMessage { role: "user".to_string(), content: Value::String(user) },
    ];

    let raw = match ModelRouter::complete(&provider, history).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("style extraction failed: {e}");
            return;
        }
    };
    let items = match parse(&raw) {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!("style extraction parse failed: {e}");
            return;
        }
    };

    save_extracted(state, user_id, items, None, Some("style")).await;
}

/// Extract the JSON array, tolerating code fences / surrounding prose.
fn parse(raw: &str) -> anyhow::Result<Vec<Extracted>> {
    let start = raw.find('[').ok_or_else(|| anyhow::anyhow!("no JSON array in model output"))?;
    let end = raw.rfind(']').ok_or_else(|| anyhow::anyhow!("no JSON array in model output"))?;
    if end < start {
        anyhow::bail!("malformed JSON array");
    }
    Ok(serde_json::from_str(&raw[start..=end])?)
}

fn normalize_category(c: &str) -> String {
    let c = c.trim().to_lowercase();
    if VALID_CATEGORIES.contains(&c.as_str()) { c } else { "other".to_string() }
}

/// Case-insensitive containment in either direction — keeps near-identical
/// memories out without needing embeddings.
fn is_duplicate(content: &str, existing: &[db::memories::Memory]) -> bool {
    let c = content.to_lowercase();
    existing.iter().any(|m| {
        let e = m.content.to_lowercase();
        e == c || e.contains(&c) || c.contains(&e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tolerates_fences_and_prose() {
        let raw = "Sure! Here you go:\n```json\n[{\"content\": \"Signs off with Cheers\"}]\n```";
        let items = parse(raw).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content, "Signs off with Cheers");
    }

    #[test]
    fn parse_empty_array() {
        assert!(parse("[]").unwrap().is_empty());
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate_chars("héllo wörld", 5), "héllo");
        assert_eq!(truncate_chars("short", 100), "short");
    }
}

/// Persist + broadcast a log entry under the `Memory` category (same pattern as
/// `routes::logs::create`) so saves surface live in the Logs window.
fn log_event(state: &AppState, message: String) {
    let entry = LogEntry {
        id: Uuid::new_v4().to_string(),
        ts: chrono::Utc::now().timestamp_millis(),
        category: "Memory".to_string(),
        level: "info".to_string(),
        message,
    };
    let _ = state.log_tx.send(serde_json::to_string(&entry).unwrap_or_default());
    let pool = state.db.clone();
    tokio::spawn(async move {
        let _ = db::logs::insert(&pool, &entry).await;
    });
}
