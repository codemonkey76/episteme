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

const VALID_CATEGORIES: [&str; 5] = ["preference", "fact", "feedback", "project", "other"];

/// Prepend a system message listing stored memories, if any exist.
pub async fn inject(history: &mut Vec<ChatMessage>, pool: &SqlitePool) {
    let memories = match db::memories::list_recent(pool, INJECT_LIMIT).await {
        Ok(m) if !m.is_empty() => m,
        _ => return,
    };

    let mut text = String::from(
        "Persistent memory about the user, learned from past conversations. Use it to \
personalize your responses and stay consistent. Do not mention these notes unless relevant.\n\n\
Memories:\n",
    );
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

const EXTRACT_SYSTEM: &str = "You extract durable, long-term memories about the user from a \
single chat exchange. Capture only things worth remembering for FUTURE conversations: stable \
preferences, personal/work facts, ongoing projects, and explicit feedback on how the user wants \
you to behave. Ignore one-off task details, transient context, and anything already obvious.\n\n\
Categorize each as one of: preference, fact, feedback, project, other.\n\n\
Respond with ONLY a JSON array, no prose, no code fences. Each element: \
{\"content\": \"<concise third-person note>\", \"category\": \"<category>\"}. If there is nothing \
worth remembering, return [].";

/// Best-effort extraction of memories from a finished exchange. Errors are
/// logged and swallowed — this must never affect the chat turn.
pub async fn extract(
    state: &AppState,
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
    let history = vec![
        ChatMessage { role: "system".to_string(), content: Value::String(EXTRACT_SYSTEM.to_string()) },
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
    if items.is_empty() {
        return;
    }

    // Pull recent memories once for dedup.
    let existing = db::memories::list_recent(&state.db, DEDUP_LIMIT).await.unwrap_or_default();

    for item in items {
        let content = item.content.trim();
        if content.is_empty() {
            continue;
        }
        if is_duplicate(content, &existing) {
            continue;
        }
        let category = normalize_category(&item.category);

        match db::memories::insert(&state.db, content, &category, "auto", session_id.as_deref()).await {
            Ok(_) => log_event(state, format!("Remembered [{category}]: {content}")),
            Err(e) => tracing::warn!("failed to save memory: {e}"),
        }
    }
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
