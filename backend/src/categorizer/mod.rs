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
use crate::integrations::graph;
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig};
use crate::routes::email;
use crate::state::AppState;

pub mod deliveries;

const CONFIG_KEY: &str = "email_categorizer";
const STATE_KEY: &str = "email_categorizer_state";

fn config_key(user_id: &str) -> String {
    format!("{CONFIG_KEY}:{user_id}")
}
/// Processed-id state is tracked per mailbox so each sorts independently.
/// The own mailbox uses an empty suffix to match the pre per-mailbox key.
fn state_key(user_id: &str, mailbox: &str) -> String {
    if mailbox.is_empty() {
        format!("{STATE_KEY}:{user_id}")
    } else {
        format!("{STATE_KEY}:{user_id}:{mailbox}")
    }
}
const GRAPH: &str = "https://graph.microsoft.com/v1.0";
/// Cap on remembered message ids so flagged/left-in-inbox mail isn't re-scanned.
const MAX_PROCESSED: usize = 1000;

/// Guards against overlapping runs (manual "Run now" racing the background
/// worker, or rapid double-clicks). Two concurrent runs would classify the same
/// inbox and both try to move the same messages — the loser hitting 404s on
/// already-moved ids.
static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// ── Config & persisted state ───────────────────────────────────────────────────

/// One auto-sort task. Each connected mailbox (the own mailbox, identified by
/// an empty `mailbox`, plus any shared mailboxes) sorts independently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategorizerTask {
    /// Shared mailbox address, or "" for the user's own mailbox.
    #[serde(default)]
    pub mailbox: String,
    #[serde(default)]
    pub enabled: bool,
    /// Provider name to use; empty → first configured provider.
    #[serde(default)]
    pub provider: String,
    /// Extra sorting instructions for this mailbox, appended to the base prompt.
    #[serde(default)]
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategorizerConfig {
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    #[serde(default = "default_batch")]
    pub batch_limit: u32,
    /// Per-mailbox sort tasks.
    #[serde(default)]
    pub tasks: Vec<CategorizerTask>,
}

fn default_interval() -> u64 { 300 }
fn default_batch() -> u32 { 25 }

impl Default for CategorizerConfig {
    fn default() -> Self {
        Self {
            interval_secs: default_interval(),
            batch_limit: default_batch(),
            tasks: Vec::new(),
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
    /// Messages recognised as updates to a linked helpdesk ticket.
    pub ticket_updates: usize,
    /// Delivery mail that created or advanced a tracked shipment.
    pub shipments: usize,
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

// ── Config accessors (used by HTTP routes) ─────────────────────────────────────

pub async fn get_config(pool: &sqlx::SqlitePool, user_id: &str) -> Result<CategorizerConfig> {
    let raw: Option<Value> = db::settings::get(pool, &config_key(user_id)).await?;
    let Some(raw) = raw else { return Ok(CategorizerConfig::default()) };
    // Migrate the pre per-mailbox shape ({enabled, provider, …} with no `tasks`)
    // into a single own-mailbox task so existing setups keep working.
    if raw.get("tasks").is_none() {
        return Ok(CategorizerConfig {
            interval_secs: raw.get("interval_secs").and_then(Value::as_u64).unwrap_or_else(default_interval),
            batch_limit: raw.get("batch_limit").and_then(Value::as_u64).unwrap_or(default_batch() as u64) as u32,
            tasks: vec![CategorizerTask {
                mailbox: String::new(),
                enabled: raw.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                provider: raw.get("provider").and_then(Value::as_str).unwrap_or("").to_string(),
                instructions: String::new(),
            }],
        });
    }
    Ok(serde_json::from_value(raw)?)
}

/// Move a user's processed-id state (own + the given shared mailboxes) from the
/// legacy per-user owner to a per-instance owner, and drop the legacy config
/// key (config now lives in the integration row). Used by the M365 migration so
/// the first post-migration run doesn't re-flag already-handled mail.
pub async fn migrate_state_owner<'a>(
    pool: &sqlx::SqlitePool,
    old_owner: &str,
    new_owner: &str,
    shared_addrs: impl Iterator<Item = &'a str>,
) {
    let mut mailboxes = vec![String::new()];
    mailboxes.extend(shared_addrs.map(str::to_string));
    for mb in mailboxes {
        let old = state_key(old_owner, &mb);
        if let Ok(Some(v)) = db::settings::get::<PersistState>(pool, &old).await {
            let _ = db::settings::set(pool, &state_key(new_owner, &mb), &v).await;
            let _ = db::settings::delete(pool, &old).await;
        }
    }
    let _ = db::settings::delete(pool, &config_key(old_owner)).await;
}

// ── Core run ───────────────────────────────────────────────────────────────────

/// Scan one mailbox's inbox and apply categorization once. `mailbox` is a
/// shared mailbox address, or "" for the user's own mailbox. Runs regardless of
/// the `enabled` flag (the worker gates on it; manual "Run now" does not).
pub async fn run_mailbox(
    state: &AppState,
    user_id: &str,
    account_id: &str,
    mailbox: &str,
    provider_name: &str,
    instructions: &str,
    batch_limit: u32,
) -> Result<RunSummary> {
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

    let provider = resolve_provider(state, provider_name).await?;

    // `me` for the own mailbox, `users/{address}` for a shared one. `mailbox_opt`
    // is what the email helpers (flag/ensure/move) take.
    let mailbox_opt = (!mailbox.is_empty()).then_some(mailbox);
    let seg = match mailbox_opt {
        Some(addr) => format!("users/{addr}"),
        None => "me".to_string(),
    };

    // Fetch the most recent inbox messages.
    let top = batch_limit.clamp(1, 50).to_string();
    let inbox = graph::graph_get(
        state,
        user_id,
        Some(account_id),
        &format!("{GRAPH}/{seg}/mailFolders/inbox/messages"),
        &[
            ("$select", "id,subject,from,bodyPreview,receivedDateTime,isRead,conversationId"),
            ("$orderby", "receivedDateTime desc"),
            ("$top", &top),
        ],
    )
    .await?;

    let messages = inbox["value"].as_array().cloned().unwrap_or_default();

    // Semantic email index rides along: embed whatever this fetch surfaced
    // (idempotent per id, detached so sorting never waits on Ollama).
    tokio::spawn(crate::email_index::index_messages(
        state.db.clone(),
        state.http_client.clone(),
        user_id.to_string(),
        mailbox.to_string(),
        messages.clone(),
    ));

    let mut st: PersistState =
        db::settings::get(&state.db, &state_key(account_id, mailbox)).await?.unwrap_or_default();
    let seen: std::collections::HashSet<&str> =
        st.processed_ids.iter().map(String::as_str).collect();

    // Keep only messages we haven't processed before.
    let all_fresh: Vec<&Value> = messages
        .iter()
        .filter(|m| m["id"].as_str().map(|id| !seen.contains(id)).unwrap_or(false))
        .collect();

    // Pull out messages in a thread we've linked to a helpdesk ticket — these
    // are handled as ticket updates (notify + draft a customer reply), never
    // sorted away by the classifier.
    let mut linked: Vec<(&Value, db::ticket_links::TicketLink)> = Vec::new();
    let mut fresh: Vec<&Value> = Vec::new();
    for m in all_fresh {
        let conv = m["conversationId"].as_str().unwrap_or("");
        let link = if conv.is_empty() {
            None
        } else {
            db::ticket_links::find(&state.db, user_id, conv).await?
        };
        match link {
            Some(l) => linked.push((m, l)),
            None => fresh.push(m),
        }
    }

    let mut summary = RunSummary { scanned: linked.len() + fresh.len(), ..Default::default() };

    // Ticket-update threads first — independent of the sort classifier.
    for (m, link) in &linked {
        match handle_ticket_update(state, &provider, user_id, account_id, &seg, mailbox_opt, m, link).await {
            Ok(()) => summary.ticket_updates += 1,
            Err(e) => {
                let subj = m["subject"].as_str().unwrap_or("(no subject)");
                log_event(state, "error", format!("Ticket-update handling failed for \"{subj}\": {e}"));
            }
        }
    }

    if fresh.is_empty() {
        // Still remember the linked ids we just handled so they aren't reprocessed.
        for (m, _) in &linked {
            if let Some(id) = m["id"].as_str() {
                st.processed_ids.push_back(id.to_string());
            }
        }
        while st.processed_ids.len() > MAX_PROCESSED {
            st.processed_ids.pop_front();
        }
        db::settings::set(&state.db, &state_key(account_id, mailbox), &st).await?;
        summary.message = if linked.is_empty() {
            "No new mail to categorize.".to_string()
        } else {
            format!("Handled {} ticket update(s); no other new mail.", summary.ticket_updates)
        };
        return Ok(summary);
    }

    // Build the classification request. The model is shown a short 1-based index
    // as the id, never the real Microsoft Graph message id (~150 opaque chars):
    // echoing those back for a full batch bloats the reply until it overruns the
    // model's output budget and the JSON array is truncated mid-stream (the very
    // failure this replaces). We map the index back to the real id afterwards.
    let mut listing = String::new();
    for (i, m) in fresh.iter().enumerate() {
        let idx = i + 1;
        let from = m["from"]["emailAddress"]["address"].as_str().unwrap_or("");
        let name = m["from"]["emailAddress"]["name"].as_str().unwrap_or("");
        let subject = m["subject"].as_str().unwrap_or("(no subject)");
        let preview: String = m["bodyPreview"].as_str().unwrap_or("").chars().take(200).collect();
        listing.push_str(&format!(
            "---\nid: {idx}\nfrom: {name} <{from}>\nsubject: {subject}\npreview: {preview}\n"
        ));
    }

    let mut system = crate::prompts::get(&state.db, "email_categorizer").await;
    // Per-mailbox custom instructions tailor sorting for this mailbox.
    if !instructions.trim().is_empty() {
        system.push_str("\n\nAdditional instructions for this mailbox:\n");
        system.push_str(instructions.trim());
    }
    let history = vec![
        ChatMessage { role: "system".to_string(), content: Value::String(system) },
        ChatMessage {
            role: "user".to_string(),
            content: Value::String(format!("Classify these emails:\n\n{listing}")),
        },
    ];

    // Pin the reply to an array-of-objects schema. Legacy JSON mode only forces
    // *valid* JSON, letting thinking-family models (e.g. qwen3) return a single
    // object and silently drop every email after the first; the schema forces the
    // full array. `id` may come back as an int or a string (see `Classification`).
    let schema = serde_json::json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "id": { "type": ["integer", "string"] },
                "category": { "type": "string" },
                "folder": { "type": "string" },
            },
            "required": ["id", "category"],
        },
    });
    let (raw, used) =
        ModelRouter::complete_json_schema_with_usage(&provider, history, schema).await?;
    db::usage::record(&state.db, user_id, &provider, "auto-sort", used).await;
    let classifications = match parse_classifications(&raw) {
        Ok(c) => c,
        Err(e) => {
            // Don't record ids; let the next cycle retry. Log a clip of the raw
            // model output so an unparseable reply (prose, empty, truncated) is
            // diagnosable instead of opaque.
            let clip: String = raw.chars().take(500).collect();
            log_event(
                state,
                "error",
                format!("Classification parse failed: {e} — model returned {} chars: {clip}", raw.len()),
            );
            summary.message = format!("Classification failed: {e}");
            return Ok(summary);
        }
    };

    // Keyed by the short index we showed the model (1-based, as a string).
    let by_id: HashMap<&str, &Classification> =
        classifications.iter().map(|c| (c.id.as_str(), c)).collect();

    // Cache folder ids resolved within this run.
    let mut folder_ids: HashMap<String, String> = HashMap::new();

    for (i, m) in fresh.iter().enumerate() {
        let Some(id) = m["id"].as_str() else { continue };
        let idx = (i + 1).to_string();
        let subject = m["subject"].as_str().unwrap_or("(no subject)");
        let category = by_id
            .get(idx.as_str())
            .map(|c| c.category.trim().to_lowercase())
            .unwrap_or_else(|| "attention".to_string());

        match category.as_str() {
            "attention" | "none" | "" => {
                if category == "attention" {
                    match email::flag_message(state, user_id, Some(account_id), mailbox_opt, id).await {
                        Ok(()) => {
                            summary.flagged += 1;
                            log_event(state, "info", format!("Flagged: {subject}"));
                            crate::integrations::push::notify_linked(
                                state,
                                user_id,
                                "Email needs attention",
                                subject,
                                "email",
                                None,
                            )
                            .await;
                        }
                        Err(e) => log_event(state, "error", format!("Flag failed for \"{subject}\": {e}")),
                    }
                } else {
                    summary.skipped += 1;
                }
            }
            other => {
                // Shipping mail gets a second pass that turns it into a tracked
                // shipment. It must run BEFORE the move: Graph issues a fresh id
                // when a message changes folder, so the body fetch would 404
                // afterwards.
                if other == "deliveries" {
                    match deliveries::handle(state, &provider, user_id, account_id, &seg, m).await {
                        Ok(true) => {
                            summary.shipments += 1;
                            log_event(state, "info", format!("Tracking shipment: {subject}"));
                        }
                        Ok(false) => {}
                        Err(e) => log_event(
                            state,
                            "error",
                            format!("Shipment extraction failed for \"{subject}\": {e}"),
                        ),
                    }
                }
                // "folder" carries a custom destination named by the per-mailbox
                // instructions; everything else maps through the fixed categories.
                let folder_name = if other == "folder" {
                    match by_id.get(idx.as_str()).and_then(|c| c.folder.as_deref()).map(str::trim) {
                        Some(name) if !name.is_empty() && name.len() <= 100 => name.to_string(),
                        _ => {
                            summary.skipped += 1;
                            continue;
                        }
                    }
                } else {
                    let Some(name) = folder_for(other) else {
                        summary.skipped += 1;
                        continue;
                    };
                    name.to_string()
                };
                // Resolve (and cache) the destination folder id.
                let folder_id = match folder_ids.get(&folder_name) {
                    Some(fid) => fid.clone(),
                    None => match email::ensure_folder(state, user_id, Some(account_id), mailbox_opt, &folder_name).await {
                        Ok(fid) => {
                            folder_ids.insert(folder_name.clone(), fid.clone());
                            fid
                        }
                        Err(e) => {
                            log_event(state, "error", format!("Folder \"{folder_name}\" unavailable: {e}"));
                            summary.skipped += 1;
                            continue;
                        }
                    },
                };
                match email::move_message(state, user_id, Some(account_id), mailbox_opt, id, &folder_id).await {
                    Ok(()) => {
                        summary.moved += 1;
                        log_event(state, "info", format!("Moved to {folder_name}: {subject}"));
                    }
                    Err(e) => log_event(state, "error", format!("Move failed for \"{subject}\": {e}")),
                }
            }
        }
    }

    // Mark every scanned message as processed (the model had its chance, and
    // ticket-update threads were handled above), and bound the remembered set.
    for m in fresh.iter().chain(linked.iter().map(|(m, _)| m)) {
        if let Some(id) = m["id"].as_str() {
            st.processed_ids.push_back(id.to_string());
        }
    }
    while st.processed_ids.len() > MAX_PROCESSED {
        st.processed_ids.pop_front();
    }
    db::settings::set(&state.db, &state_key(account_id, mailbox), &st).await?;

    summary.message = format!(
        "Scanned {}, moved {}, flagged {}, ticket updates {}, shipments {}, left {}.",
        summary.scanned,
        summary.moved,
        summary.flagged,
        summary.ticket_updates,
        summary.shipments,
        summary.skipped
    );
    Ok(summary)
}

/// AI-extracted from a provider's reply: a one-line notification summary plus a
/// customer-facing draft reply to post on the ticket.
#[derive(Debug, Deserialize)]
struct TicketUpdate {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    reply: String,
}

/// Handle an inbox message that belongs to a thread linked to a helpdesk ticket:
/// draft a customer-facing reply, flag the mail in place, and raise an
/// actionable `ticket_update` notification carrying the draft.
#[allow(clippy::too_many_arguments)]
async fn handle_ticket_update(
    state: &AppState,
    provider: &ProviderConfig,
    user_id: &str,
    account_id: &str,
    seg: &str,
    mailbox_opt: Option<&str>,
    msg: &Value,
    link: &db::ticket_links::TicketLink,
) -> Result<()> {
    let id = msg["id"].as_str().ok_or_else(|| anyhow::anyhow!("message missing id"))?;
    let subject = msg["subject"].as_str().unwrap_or("(no subject)");
    let from = msg["from"]["emailAddress"]["address"].as_str().unwrap_or("");

    // The list fetch only carried a preview; pull the full body for a faithful draft.
    let full = graph::graph_get(
        state,
        user_id,
        Some(account_id),
        &format!("{GRAPH}/{seg}/messages/{id}"),
        &[("$select", "body")],
    )
    .await?;
    let body_raw = full["body"]["content"].as_str().unwrap_or("");
    let body_text = if full["body"]["contentType"].as_str().map(|c| c.eq_ignore_ascii_case("html")).unwrap_or(false) {
        graph::html_to_text(body_raw)
    } else {
        body_raw.to_string()
    };
    let body_text: String = body_text.chars().take(4000).collect();

    let system = crate::prompts::get(&state.db, "ticket_update_reply").await;
    let user_msg = format!("From: {from}\nSubject: {subject}\n\n{body_text}");
    let (raw, used) = ModelRouter::complete_json_with_usage(
        provider,
        vec![
            ChatMessage { role: "system".to_string(), content: Value::String(system) },
            ChatMessage { role: "user".to_string(), content: Value::String(user_msg) },
        ],
    )
    .await?;
    db::usage::record(&state.db, user_id, provider, "ticket-update", used).await;

    // Tolerate code fences / prose around the JSON object.
    let start = raw.find('{').ok_or_else(|| anyhow::anyhow!("no JSON in model output"))?;
    let end = raw.rfind('}').ok_or_else(|| anyhow::anyhow!("no JSON in model output"))?;
    let parsed: TicketUpdate = serde_json::from_str(&raw[start..=end])
        .map_err(|e| anyhow::anyhow!("ticket-update output unparseable: {e}"))?;
    let summary_line = parsed.summary.trim();
    let summary_line = if summary_line.is_empty() { subject } else { summary_line };

    // It's actionable — flag in place rather than sorting it into a folder.
    if let Err(e) = email::flag_message(state, user_id, Some(account_id), mailbox_opt, id).await {
        log_event(state, "error", format!("Flag failed for ticket-update \"{subject}\": {e}"));
    }

    let ticket_id_str = link.ticket_id.to_string();
    let data = serde_json::json!({
        "ticket_id": link.ticket_id,
        "integration": link.integration,
        "draft_reply": parsed.reply,
        "subject": subject,
    })
    .to_string();

    crate::integrations::push::notify_action(
        state,
        user_id,
        &format!("Ticket #{} update", link.ticket_id),
        summary_line,
        "ticket_update",
        Some(crate::integrations::push::Link { kind: "helpdesk_ticket", id: &ticket_id_str }),
        Some(&data),
    )
    .await;

    log_event(state, "info", format!("Ticket #{} update: {summary_line}", link.ticket_id));
    Ok(())
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
    /// The short index we showed the model. Accept a JSON number too — models
    /// often emit `"id": 1` for a numeric-looking id rather than `"id": "1"`.
    #[serde(deserialize_with = "de_string_or_number")]
    id: String,
    category: String,
    /// Custom destination folder, only honoured with category "folder" —
    /// directed by the per-mailbox instructions.
    #[serde(default)]
    folder: Option<String>,
}

/// Deserialize a field that may arrive as a JSON string or number into a String.
fn de_string_or_number<'de, D>(d: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    })
}

/// Extract the classification array from a model response, tolerating code
/// fences or surrounding prose. Prefers a top-level JSON array (slicing between
/// the first `[` and last `]`); falls back to the first array of objects inside
/// a wrapper object (e.g. `{"classifications": [...]}`), which JSON-mode local
/// models sometimes emit instead of a bare array.
fn parse_classifications(raw: &str) -> Result<Vec<Classification>> {
    if let (Some(start), Some(end)) = (raw.find('['), raw.rfind(']')) {
        if end > start {
            if let Ok(v) = serde_json::from_str::<Vec<Classification>>(&raw[start..=end]) {
                return Ok(v);
            }
        }
    }
    if let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}')) {
        if end > start {
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&raw[start..=end]) {
                for (_k, val) in map {
                    if val.is_array() {
                        if let Ok(v) = serde_json::from_value::<Vec<Classification>>(val) {
                            return Ok(v);
                        }
                    }
                }
            }
        }
    }
    anyhow::bail!("no JSON array in model output")
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
            // Iterate every account: each user has their own config, mailbox
            // connection, and processed-id state. The shortest enabled
            // interval drives the loop cadence.
            let users = crate::db::auth::list_users(&state.db).await.unwrap_or_default();
            let mut next_interval: u64 = 300;

            for user in &users {
                // Each microsoft account has its own categorizer config + state.
                let integs = db::integrations::list_by_kind(&state.db, &user.id, "microsoft")
                    .await
                    .unwrap_or_default();
                for integ in &integs {
                    let cfg = integ
                        .parse_config::<crate::integrations::microsoft::MicrosoftConfig>()
                        .unwrap_or_default()
                        .categorizer;
                    let enabled_tasks: Vec<&CategorizerTask> =
                        cfg.tasks.iter().filter(|t| t.enabled).collect();
                    if enabled_tasks.is_empty() {
                        continue;
                    }
                    next_interval = next_interval.min(cfg.interval_secs.max(60));
                    // Each enabled mailbox in this account sorts independently.
                    for task in enabled_tasks {
                        match run_mailbox(
                            &state,
                            &user.id,
                            &integ.id,
                            &task.mailbox,
                            &task.provider,
                            &task.instructions,
                            cfg.batch_limit,
                        )
                        .await
                        {
                            Ok(s) if s.scanned > 0 => {
                                tracing::info!("categorizer[{}/{}/{}]: {}", user.username, integ.name, task.mailbox, s.message);
                            }
                            Ok(_) => {}
                            // not_connected just means this account isn't linked.
                            Err(e) if e.to_string().contains("not_connected") => {}
                            Err(e) => {
                                tracing::warn!("categorizer run failed for {}: {e}", user.username);
                                log_event(&state, "error", format!("Run failed ({}): {e}", user.username));
                            }
                        }
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(next_interval)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bare_array_with_surrounding_prose() {
        let raw = "Sure, here you go:\n[{\"id\":\"a\",\"category\":\"promotions\"}]\nLet me know!";
        let v = parse_classifications(raw).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].category, "promotions");
    }

    #[test]
    fn parse_array_in_code_fence() {
        let raw = "```json\n[{\"id\":\"x\",\"category\":\"invoices\"}]\n```";
        let v = parse_classifications(raw).unwrap();
        assert_eq!(v[0].id, "x");
    }

    #[test]
    fn parse_array_wrapped_in_object() {
        // JSON-mode local models sometimes wrap the array in an object.
        let raw = "{\"classifications\":[{\"id\":\"1\",\"category\":\"notifications\"},\
                   {\"id\":\"2\",\"category\":\"none\"}]}";
        let v = parse_classifications(raw).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].category, "none");
    }

    #[test]
    fn parse_numeric_ids() {
        // Models often emit the short index as a JSON number, not a string.
        let raw = "[{\"id\":1,\"category\":\"promotions\"},{\"id\":2,\"category\":\"attention\"}]";
        let v = parse_classifications(raw).unwrap();
        assert_eq!(v[0].id, "1");
        assert_eq!(v[1].id, "2");
        assert_eq!(v[1].category, "attention");
    }

    #[test]
    fn parse_pure_prose_errors() {
        assert!(parse_classifications("I cannot classify these emails.").is_err());
    }
}
