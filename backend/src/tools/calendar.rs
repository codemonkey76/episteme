//! Calendar tools — model-facing adapter over `crate::calendar` (Microsoft
//! Graph). Schemas, arg parsing, and result localization only; domain logic
//! lives in the calendar module.

use anyhow::{anyhow, Result};
use chrono::{Duration, Utc};
use serde_json::{json, Value};

use crate::calendar::{self, NewEvent};
use crate::state::AppState;

use super::localize_event;

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

pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "list_calendar_events" | "create_calendar_event" | "delete_calendar_event"
    )
}

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
        other => Err(anyhow!("unknown calendar tool '{other}'")),
    }
}
