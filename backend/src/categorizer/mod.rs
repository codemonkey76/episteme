//! Background AI email categorizer.
//!
//! Periodically scans the Microsoft 365 Inbox, asks the configured AI provider
//! to classify each new message, and acts on the result: low-priority mail is
//! moved into per-category folders, mail that needs the user's attention is
//! flagged in place. Every action is written to the Logs window for audit since
//! it mutates the live mailbox unattended.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::db;
use crate::db::logs::LogEntry;
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig};
use crate::routes::email;
use crate::state::AppState;

const CONFIG_KEY: &str = "email_categorizer";
const STATE_KEY: &str = "email_categorizer_state";
const GRAPH: &str = "https://graph.microsoft.com/v1.0";
/// Cap on remembered message ids so flagged/left-in-inbox mail isn't re-scanned.
const MAX_PROCESSED: usize = 1000;

/// Guards against overlapping runs (manual "Run now" racing the background
/// worker, or rapid double-clicks). Two concurrent runs would classify the same
/// inbox and both try to move the same messages — the loser hitting 404s on
/// already-moved ids.
static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// ── Config & persisted state ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategorizerConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Provider name to use; empty → first configured provider.
    #[serde(default)]
    pub provider: String,
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    #[serde(default = "default_batch")]
    pub batch_limit: u32,
}

fn default_interval() -> u64 { 300 }
fn default_batch() -> u32 { 25 }

impl Default for CategorizerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: String::new(),
            interval_secs: default_interval(),
            batch_limit: default_batch(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistState {
    #[serde(default)]
    processed_ids: VecDeque<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct RunSummary {
    pub scanned: usize,
    pub moved: usize,
    pub flagged: usize,
    pub skipped: usize,
    pub message: String,
}

// ── Categories ─────────────────────────────────────────────────────────────────

/// Map a model category to its destination folder. `attention`/`none` return
/// None (handled specially: flag in place / leave untouched).
fn folder_for(category: &str) -> Option<&'static str> {
    match category {
        "promotions" => Some("Promotions"),
        "invoices" => Some("Invoices"),
        "notifications" => Some("Notifications"),
        "deliveries" => Some("Deliveries"),
        _ => None,
    }
}

const SYSTEM_PROMPT: &str = "You are an email triage assistant. You are given a \
list of inbox emails. Classify EACH email into exactly one category:\n\
- \"promotions\": marketing, newsletters, sales, offers.\n\
- \"invoices\": bills, receipts, payment confirmations, statements.\n\
- \"notifications\": automated system notifications, alerts, Sentry/error \
reports, CI results, monitoring.\n\
- \"deliveries\": shipping and delivery/order tracking updates.\n\
- \"attention\": anything that needs a human to read or act — personal mail, \
direct questions, requests, anything ambiguous or important.\n\
- \"none\": equivalent to attention; use when unsure. Never guess a low-priority \
category for mail that might matter.\n\n\
Respond with ONLY a JSON array, no prose, no code fences. Each element: \
{\"id\": \"<the email id>\", \"category\": \"<one category>\"}. Include every email \
exactly once.";

// ── Config accessors (used by HTTP routes) ─────────────────────────────────────

pub async fn get_config(pool: &sqlx::SqlitePool) -> Result<CategorizerConfig> {
    Ok(db::settings::get(pool, CONFIG_KEY).await?.unwrap_or_default())
}

pub async fn set_config(pool: &sqlx::SqlitePool, cfg: &CategorizerConfig) -> Result<()> {
    db::settings::set(pool, CONFIG_KEY, cfg).await
}

// ── Core run ───────────────────────────────────────────────────────────────────

/// Scan the inbox and apply categorization once. Runs regardless of the
/// `enabled` flag (the worker gates on `enabled`; manual "Run now" does not).
pub async fn run_once(state: &AppState) -> Result<RunSummary> {
    use std::sync::atomic::Ordering;

    // Acquire the single-run lock; bail if another run is already in flight.
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(RunSummary {
            message: "A categorization run is already in progress.".to_string(),
            ..Default::default()
        });
    }
    // Reset the lock on every exit path (including early returns and errors).
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            RUNNING.store(false, Ordering::SeqCst);
        }
    }
    let _guard = Guard;

    let cfg = get_config(&state.db).await?;

    let provider = resolve_provider(state, &cfg.provider).await?;

    // Fetch the most recent inbox messages.
    let top = cfg.batch_limit.clamp(1, 50).to_string();
    let inbox = email::graph_get(
        state,
        &format!("{GRAPH}/me/mailFolders/inbox/messages"),
        &[
            ("$select", "id,subject,from,bodyPreview,receivedDateTime,isRead"),
            ("$orderby", "receivedDateTime desc"),
            ("$top", &top),
        ],
    )
    .await?;

    let messages = inbox["value"].as_array().cloned().unwrap_or_default();

    let mut st: PersistState = db::settings::get(&state.db, STATE_KEY).await?.unwrap_or_default();
    let seen: std::collections::HashSet<&str> =
        st.processed_ids.iter().map(String::as_str).collect();

    // Keep only messages we haven't processed before.
    let fresh: Vec<&Value> = messages
        .iter()
        .filter(|m| m["id"].as_str().map(|id| !seen.contains(id)).unwrap_or(false))
        .collect();

    let mut summary = RunSummary { scanned: fresh.len(), ..Default::default() };

    if fresh.is_empty() {
        summary.message = "No new mail to categorize.".to_string();
        return Ok(summary);
    }

    // Build the classification request.
    let mut listing = String::new();
    for m in &fresh {
        let id = m["id"].as_str().unwrap_or("");
        let from = m["from"]["emailAddress"]["address"].as_str().unwrap_or("");
        let name = m["from"]["emailAddress"]["name"].as_str().unwrap_or("");
        let subject = m["subject"].as_str().unwrap_or("(no subject)");
        let preview: String = m["bodyPreview"].as_str().unwrap_or("").chars().take(200).collect();
        listing.push_str(&format!(
            "---\nid: {id}\nfrom: {name} <{from}>\nsubject: {subject}\npreview: {preview}\n"
        ));
    }

    let history = vec![
        ChatMessage { role: "system".to_string(), content: Value::String(SYSTEM_PROMPT.to_string()) },
        ChatMessage {
            role: "user".to_string(),
            content: Value::String(format!("Classify these emails:\n\n{listing}")),
        },
    ];

    let raw = ModelRouter::complete(&provider, history).await?;
    let classifications = match parse_classifications(&raw) {
        Ok(c) => c,
        Err(e) => {
            // Don't record ids; let the next cycle retry.
            log_event(state, "error", format!("Classification parse failed: {e}"));
            summary.message = format!("Classification failed: {e}");
            return Ok(summary);
        }
    };

    let by_id: HashMap<&str, &Classification> =
        classifications.iter().map(|c| (c.id.as_str(), c)).collect();

    // Cache folder ids resolved within this run.
    let mut folder_ids: HashMap<&'static str, String> = HashMap::new();

    for m in &fresh {
        let Some(id) = m["id"].as_str() else { continue };
        let subject = m["subject"].as_str().unwrap_or("(no subject)");
        let category = by_id
            .get(id)
            .map(|c| c.category.trim().to_lowercase())
            .unwrap_or_else(|| "attention".to_string());

        match category.as_str() {
            "attention" | "none" | "" => {
                if category == "attention" {
                    match email::flag_message(state, id).await {
                        Ok(()) => {
                            summary.flagged += 1;
                            log_event(state, "info", format!("Flagged: {subject}"));
                        }
                        Err(e) => log_event(state, "error", format!("Flag failed for \"{subject}\": {e}")),
                    }
                } else {
                    summary.skipped += 1;
                }
            }
            other => {
                let Some(folder_name) = folder_for(other) else {
                    summary.skipped += 1;
                    continue;
                };
                // Resolve (and cache) the destination folder id.
                let folder_id = match folder_ids.get(folder_name) {
                    Some(fid) => fid.clone(),
                    None => match email::ensure_folder(state, folder_name).await {
                        Ok(fid) => {
                            folder_ids.insert(folder_name, fid.clone());
                            fid
                        }
                        Err(e) => {
                            log_event(state, "error", format!("Folder \"{folder_name}\" unavailable: {e}"));
                            summary.skipped += 1;
                            continue;
                        }
                    },
                };
                match email::move_message(state, id, &folder_id).await {
                    Ok(()) => {
                        summary.moved += 1;
                        log_event(state, "info", format!("Moved to {folder_name}: {subject}"));
                    }
                    Err(e) => log_event(state, "error", format!("Move failed for \"{subject}\": {e}")),
                }
            }
        }
    }

    // Mark every scanned message as processed (the model had its chance), and
    // bound the remembered set.
    for m in &fresh {
        if let Some(id) = m["id"].as_str() {
            st.processed_ids.push_back(id.to_string());
        }
    }
    while st.processed_ids.len() > MAX_PROCESSED {
        st.processed_ids.pop_front();
    }
    db::settings::set(&state.db, STATE_KEY, &st).await?;

    summary.message = format!(
        "Scanned {}, moved {}, flagged {}, left {}.",
        summary.scanned, summary.moved, summary.flagged, summary.skipped
    );
    Ok(summary)
}

async fn resolve_provider(state: &AppState, name: &str) -> Result<ProviderConfig> {
    let providers: Vec<ProviderConfig> =
        db::settings::get(&state.db, "providers").await?.unwrap_or_default();
    let chosen = if name.is_empty() {
        providers.into_iter().next()
    } else {
        providers.into_iter().find(|p| p.name == name)
    };
    chosen.ok_or_else(|| anyhow::anyhow!("no AI provider configured"))
}

#[derive(Debug, Deserialize)]
struct Classification {
    id: String,
    category: String,
}

/// Extract the JSON array from a model response, tolerating code fences or
/// surrounding prose by slicing between the first `[` and last `]`.
fn parse_classifications(raw: &str) -> Result<Vec<Classification>> {
    let start = raw.find('[').ok_or_else(|| anyhow::anyhow!("no JSON array in model output"))?;
    let end = raw.rfind(']').ok_or_else(|| anyhow::anyhow!("no JSON array in model output"))?;
    if end < start {
        anyhow::bail!("malformed JSON array in model output");
    }
    Ok(serde_json::from_str(&raw[start..=end])?)
}

// ── Logging ────────────────────────────────────────────────────────────────────

/// Persist a log entry and broadcast it to live Logs-window subscribers,
/// matching `routes::logs::create`.
fn log_event(state: &AppState, level: &str, message: String) {
    let entry = LogEntry {
        id: Uuid::new_v4().to_string(),
        ts: chrono::Utc::now().timestamp_millis(),
        category: "Categorizer".to_string(),
        level: level.to_string(),
        message,
    };
    let _ = state.log_tx.send(serde_json::to_string(&entry).unwrap_or_default());
    let pool = state.db.clone();
    tokio::spawn(async move {
        let _ = db::logs::insert(&pool, &entry).await;
    });
}

// ── Background worker ───────────────────────────────────────────────────────────

/// Spawn the polling loop. Reloads config each cycle so enable/interval changes
/// take effect without a restart; never lets an error kill the loop.
pub fn spawn_worker(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            let cfg = get_config(&state.db).await.unwrap_or_default();
            let interval = cfg.interval_secs.max(60);

            if cfg.enabled {
                match run_once(&state).await {
                    Ok(s) if s.scanned > 0 => {
                        tracing::info!("categorizer: {}", s.message);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("categorizer run failed: {e}");
                        log_event(&state, "error", format!("Run failed: {e}"));
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    });
}
