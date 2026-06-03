//! Native tools the chat agent can call directly (no MCP). Currently calendar
//! management. Executed inline by `agent::run_turn`.

use anyhow::{anyhow, Result};
use chrono::{Duration, Utc};
use serde_json::{json, Value};

use crate::calendar::{self, NewEvent};
use crate::model_router::ChatMessage;
use crate::state::AppState;

/// JSON schemas advertised to the model.
pub fn schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "list_calendar_events",
            "description": "List the user's calendar events in a date range. Use this to answer questions about their schedule.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "start": { "type": "string", "description": "Range start, RFC3339 with the user's UTC offset from the system message. Defaults to now." },
                    "end":   { "type": "string", "description": "Range end, RFC3339. Defaults to `days` after start." },
                    "days":  { "type": "integer", "description": "If end is omitted, look this many days ahead. Default 7." }
                }
            }
        }),
        json!({
            "name": "create_calendar_event",
            "description": "Create a calendar event or reminder. For a reminder, create a short event and set reminder_minutes_before.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "subject": { "type": "string", "description": "Title of the event." },
                    "start": { "type": "string", "description": "Start time, RFC3339 with the user's UTC offset from the system message (e.g. 15:00 local with offset +10:00 is 2026-06-03T15:00:00+10:00)." },
                    "end": { "type": "string", "description": "End time, RFC3339. Defaults to one hour after start." },
                    "is_all_day": { "type": "boolean" },
                    "location": { "type": "string" },
                    "body": { "type": "string", "description": "Optional notes/description." },
                    "reminder_minutes_before": { "type": "integer", "description": "Minutes before start to alert. Omit for no reminder." }
                },
                "required": ["subject", "start"]
            }
        }),
        json!({
            "name": "delete_calendar_event",
            "description": "Delete a calendar event by id. Get ids from list_calendar_events first.",
            "input_schema": {
                "type": "object",
                "properties": { "event_id": { "type": "string" } },
                "required": ["event_id"]
            }
        }),
    ]
}

pub fn is_native(name: &str) -> bool {
    matches!(
        name,
        "list_calendar_events" | "create_calendar_event" | "delete_calendar_event"
    )
}

/// A system message giving the model fully resolved local "now" (weekday,
/// date, time, offset) plus tool guidance — so it never has to do timezone
/// or day-of-week arithmetic itself.
pub async fn system_preamble(state: &AppState) -> ChatMessage {
    let tz = state.home_tz().await;
    let now = Utc::now().with_timezone(&tz);
    let text = format!(
        "You are a helpful assistant with access to the user's Microsoft 365 calendar.\n\
The current date and time is {now_str} in the user's timezone ({tz_name}, UTC{offset}).\n\
Always present dates and times to the user in this timezone — never in UTC, and \
never show a timezone conversion.\n\
When the user asks to schedule, add an appointment, or set a reminder, call \
create_calendar_event. Resolve relative times (\"tomorrow\", \"next Friday at 3pm\") \
against the current time and output start/end as RFC3339 with the user's UTC offset \
({offset}). For reminders, set reminder_minutes_before. After acting, briefly \
confirm what you did in plain language.",
        now_str = now.format("%A, %-d %B %Y, %-I:%M %p"),
        tz_name = tz.name(),
        offset = now.format("%:z"),
    );
    ChatMessage { role: "system".to_string(), content: Value::String(text) }
}

/// Convert a UTC RFC3339 string into the home timezone: a machine-readable
/// RFC3339 with local offset plus a human form with weekday. Returns the
/// input unchanged (no display) if it doesn't parse.
fn localize(s: &str, tz: chrono_tz::Tz) -> (String, Option<String>) {
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
fn localize_event(mut ev: Value, tz: chrono_tz::Tz) -> Value {
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

/// Execute a native tool, returning a JSON result for the model.
pub async fn execute(state: &AppState, name: &str, args: Value) -> Result<Value> {
    match name {
        "list_calendar_events" => {
            let start = match args["start"].as_str() {
                Some(s) => chrono::DateTime::parse_from_rfc3339(s)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                None => Utc::now(),
            };
            let end = match args["end"].as_str() {
                Some(s) => chrono::DateTime::parse_from_rfc3339(s)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or(start + Duration::days(7)),
                None => {
                    let days = args["days"].as_i64().unwrap_or(7).clamp(1, 60);
                    start + Duration::days(days)
                }
            };
            let events = calendar::list_events(state, start, end).await?;
            // Localize times for the model — it should never see UTC.
            let tz = state.home_tz().await;
            let events: Vec<Value> = events
                .into_iter()
                .map(|e| localize_event(serde_json::to_value(e).unwrap_or_default(), tz))
                .collect();
            Ok(json!({ "events": events, "timezone": tz.name() }))
        }
        "create_calendar_event" => {
            let subject = args["subject"]
                .as_str()
                .ok_or_else(|| anyhow!("subject is required"))?
                .to_string();
            let start = args["start"]
                .as_str()
                .ok_or_else(|| anyhow!("start is required"))?
                .to_string();
            let ev = NewEvent {
                subject,
                start,
                end: args["end"].as_str().map(str::to_string),
                is_all_day: args["is_all_day"].as_bool().unwrap_or(false),
                location: args["location"].as_str().map(str::to_string),
                body: args["body"].as_str().map(str::to_string),
                reminder_minutes_before: args["reminder_minutes_before"].as_i64(),
            };
            let created = calendar::create_event(state, ev).await?;
            // Echo the result in local time so the model confirms correctly.
            let tz = state.home_tz().await;
            let created = localize_event(serde_json::to_value(created).unwrap_or_default(), tz);
            Ok(json!({ "created": created }))
        }
        "delete_calendar_event" => {
            let id = args["event_id"]
                .as_str()
                .ok_or_else(|| anyhow!("event_id is required"))?;
            calendar::delete_event(state, id).await?;
            Ok(json!({ "deleted": id }))
        }
        other => Err(anyhow!("unknown native tool '{other}'")),
    }
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
