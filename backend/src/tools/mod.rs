//! Native tools the chat agent can call directly (no MCP), executed inline by
//! `agent::run_turn`. This module is a thin registry: each integration lives
//! in its own file exposing `schemas()`, `handles(name)`, and `execute()`.

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::Value;

use crate::model_router::ChatMessage;
use crate::state::AppState;

pub mod calendar;
pub mod helpdesk;
pub mod notes;
pub mod tasks;

/// JSON schemas advertised to the model, across all integrations.
pub fn schemas() -> Vec<Value> {
    let mut all = Vec::new();
    all.extend(calendar::schemas());
    all.extend(tasks::schemas());
    all.extend(notes::schemas());
    all.extend(helpdesk::schemas());
    all
}

pub fn is_native(name: &str) -> bool {
    calendar::handles(name) || tasks::handles(name) || notes::handles(name) || helpdesk::handles(name)
}

/// Native tools grouped by integration, for the settings Tools page.
pub fn catalog() -> Vec<(&'static str, Vec<Value>)> {
    vec![
        ("calendar", calendar::schemas()),
        ("tasks", tasks::schemas()),
        ("notes", notes::schemas()),
        ("helpdesk", helpdesk::schemas()),
    ]
}

/// Default approval policy when the user hasn't configured one in Settings →
/// Tools. Outward-facing helpdesk writes (replies email the customer; time
/// entries feed billing) start gated; everything else auto-executes.
pub fn default_policy(name: &str) -> &'static str {
    match name {
        "helpdesk_create_ticket"
        | "helpdesk_reply_ticket"
        | "helpdesk_log_time"
        | "helpdesk_update_ticket" => "ask",
        _ => "auto",
    }
}

/// Execute a native tool, returning a JSON result for the model.
pub async fn execute(state: &AppState, user_id: &str, name: &str, args: Value) -> Result<Value> {
    if calendar::handles(name) {
        return calendar::execute(state, user_id, name, args).await;
    }
    if tasks::handles(name) {
        return tasks::execute(state, user_id, name, args).await;
    }
    if notes::handles(name) {
        return notes::execute(state, user_id, name, args).await;
    }
    if helpdesk::handles(name) {
        return helpdesk::execute(state, user_id, name, args).await;
    }
    Err(anyhow!("unknown native tool '{name}'"))
}

/// A system message giving the model fully resolved local "now" (weekday,
/// date, time, offset) plus tool guidance — so it never has to do timezone
/// or day-of-week arithmetic itself.
pub async fn system_preamble(state: &AppState, user_id: &str) -> ChatMessage {
    let tz = state.home_tz(user_id).await;
    let now = Utc::now().with_timezone(&tz);
    let text = crate::prompts::get(&state.db, "chat_system")
        .await
        .replace("{now}", &now.format("%A, %-d %B %Y, %-I:%M %p").to_string())
        .replace("{timezone}", tz.name())
        .replace("{offset}", &now.format("%:z").to_string());
    ChatMessage { role: "system".to_string(), content: Value::String(text) }
}

/// Convert a UTC RFC3339 string into the home timezone: a machine-readable
/// RFC3339 with local offset plus a human form with weekday. Returns the
/// input unchanged (no display) if it doesn't parse.
pub(crate) fn localize(s: &str, tz: chrono_tz::Tz) -> (String, Option<String>) {
    match chrono::DateTime::parse_from_rfc3339(s) {
        Ok(dt) => {
            let local = dt.with_timezone(&tz);
            (
                local.to_rfc3339(),
                Some(local.format("%a %-d %b %Y, %-I:%M %p").to_string()),
            )
        }
        Err(_) => (s.to_string(), None),
    }
}

/// Rewrite a serialized event's start/end into the home timezone and attach
/// human-readable display strings, so the model never sees a UTC offset.
pub(crate) fn localize_event(mut ev: Value, tz: chrono_tz::Tz) -> Value {
    for (field, display_field) in [("start", "start_display"), ("end", "end_display")] {
        if let Some(s) = ev[field].as_str() {
            let (rfc, display) = localize(s, tz);
            ev[field] = Value::String(rfc);
            if let Some(d) = display {
                ev[display_field] = Value::String(d);
            }
        }
    }
    ev
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localize_converts_utc_to_home_tz_with_weekday() {
        // The endoscopy bug: 12:45 PM Brisbane was stored as 12:45 UTC.
        // Correctly stored (02:45 UTC) it must render as 12:45 PM local.
        let (rfc, display) = localize("2026-06-04T02:45:00Z", chrono_tz::Tz::Australia__Brisbane);
        assert_eq!(rfc, "2026-06-04T12:45:00+10:00");
        assert_eq!(display.as_deref(), Some("Thu 4 Jun 2026, 12:45 PM"));
    }

    #[test]
    fn localize_passes_through_unparseable() {
        let (rfc, display) = localize("not-a-date", chrono_tz::Tz::Australia__Brisbane);
        assert_eq!(rfc, "not-a-date");
        assert!(display.is_none());
    }
}
