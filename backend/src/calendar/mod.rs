//! Microsoft 365 calendar operations, shared by the HTTP routes and the chat
//! agent's native tools. Reuses the shared Graph helpers and the Microsoft
//! token plumbing.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::integrations::microsoft;
use crate::integrations::graph::{graph_delete, graph_post};
use crate::state::AppState;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

#[derive(Debug, Serialize)]
pub struct CalendarEvent {
    pub id: String,
    pub subject: String,
    /// RFC3339 in UTC.
    pub start: String,
    pub end: String,
    pub location: String,
    pub is_all_day: bool,
    pub web_link: String,
}

/// Parse an RFC3339 timestamp (with offset) into UTC. The model/UI are asked to
/// supply an offset; if absent we assume the value is already UTC.
fn to_utc(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            // Tolerate a trailing 'Z'-less or space-separated form by appending Z.
            DateTime::parse_from_rfc3339(&format!("{}Z", s.trim_end_matches('Z')))
                .map(|dt| dt.with_timezone(&Utc))
        })
        .map_err(|_| anyhow!("invalid datetime '{s}' — expected RFC3339 e.g. 2026-06-03T15:00:00+10:00"))
}

/// Graph returns dateTime like "2026-06-03T15:00:00.0000000"; with the UTC Prefer
/// header it's UTC. Normalize to RFC3339 'Z'.
fn graph_dt_to_rfc3339(v: &Value) -> String {
    let raw = v["dateTime"].as_str().unwrap_or_default();
    let trimmed = raw.split('.').next().unwrap_or(raw);
    DateTime::parse_from_rfc3339(&format!("{trimmed}Z"))
        .map(|dt| dt.with_timezone(&Utc).to_rfc3339())
        .unwrap_or_else(|_| raw.to_string())
}

/// List events overlapping the [start, end] window (calendarView expands
/// recurring series). Times returned in UTC.
pub async fn list_events(
    state: &AppState,
    user_id: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<CalendarEvent>> {
    // Calendar uses the user's default Microsoft 365 account (no per-account
    // switcher yet).
    let token = microsoft::get_valid_token(state, user_id, None).await?;
    let url = format!("{GRAPH}/me/calendarView");

    let response = state
        .http_client
        .get(&url)
        .query(&[
            ("startDateTime", start.to_rfc3339()),
            ("endDateTime", end.to_rfc3339()),
            ("$select", "id,subject,start,end,location,isAllDay,webLink,type".to_string()),
            ("$orderby", "start/dateTime".to_string()),
            // calendarView expands recurring series into occurrences; allow plenty
            // so a recurring-heavy month isn't truncated.
            ("$top", "500".to_string()),
        ])
        .bearer_auth(&token)
        // Ask Graph to return start/end in UTC so we don't have to map Windows tz names.
        .header("Prefer", "outlook.timezone=\"UTC\"")
        .send()
        .await?;

    let status = response.status();
    let body: Value = response.json().await?;
    if !status.is_success() {
        let msg = body["error"]["message"].as_str().unwrap_or("Graph error");
        anyhow::bail!("calendar list failed ({status}): {msg}");
    }

    let events = body["value"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|e| CalendarEvent {
                    id: e["id"].as_str().unwrap_or_default().to_string(),
                    subject: e["subject"].as_str().unwrap_or("(no subject)").to_string(),
                    start: graph_dt_to_rfc3339(&e["start"]),
                    end: graph_dt_to_rfc3339(&e["end"]),
                    location: e["location"]["displayName"].as_str().unwrap_or_default().to_string(),
                    is_all_day: e["isAllDay"].as_bool().unwrap_or(false),
                    web_link: e["webLink"].as_str().unwrap_or_default().to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(events)
}

pub struct NewEvent {
    pub subject: String,
    pub start: String,
    pub end: Option<String>,
    pub is_all_day: bool,
    pub location: Option<String>,
    pub body: Option<String>,
    pub reminder_minutes_before: Option<i64>,
}

/// Create an event. `start`/`end` are RFC3339 (offset honored); end defaults to
/// one hour after start when omitted.
pub async fn create_event(state: &AppState, user_id: &str, ev: NewEvent) -> Result<CalendarEvent> {
    let start_utc = to_utc(&ev.start)?;
    let end_utc = match ev.end.as_deref() {
        Some(e) => to_utc(e)?,
        None => start_utc + chrono::Duration::hours(1),
    };

    // Graph wants a naive dateTime paired with a timeZone; we send UTC.
    let fmt = |dt: DateTime<Utc>| dt.format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut payload = serde_json::json!({
        "subject": ev.subject,
        "isAllDay": ev.is_all_day,
        "start": { "dateTime": fmt(start_utc), "timeZone": "UTC" },
        "end":   { "dateTime": fmt(end_utc),   "timeZone": "UTC" },
    });
    if let Some(loc) = ev.location.filter(|s| !s.is_empty()) {
        payload["location"] = serde_json::json!({ "displayName": loc });
    }
    if let Some(body) = ev.body.filter(|s| !s.is_empty()) {
        payload["body"] = serde_json::json!({ "contentType": "text", "content": body });
    }
    if let Some(mins) = ev.reminder_minutes_before {
        payload["isReminderOn"] = serde_json::json!(true);
        payload["reminderMinutesBeforeStart"] = serde_json::json!(mins);
    }

    let created = graph_post(state, user_id, None, &format!("{GRAPH}/me/events"), &payload)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;

    Ok(CalendarEvent {
        id: created["id"].as_str().unwrap_or_default().to_string(),
        subject: created["subject"].as_str().unwrap_or_default().to_string(),
        start: graph_dt_to_rfc3339(&created["start"]),
        end: graph_dt_to_rfc3339(&created["end"]),
        location: created["location"]["displayName"].as_str().unwrap_or_default().to_string(),
        is_all_day: created["isAllDay"].as_bool().unwrap_or(false),
        web_link: created["webLink"].as_str().unwrap_or_default().to_string(),
    })
}

pub async fn delete_event(state: &AppState, user_id: &str, id: &str) -> Result<()> {
    graph_delete(state, user_id, None, &format!("{GRAPH}/me/events/{id}"))
        .await
        .map_err(|e| anyhow!(e.to_string()))
}
