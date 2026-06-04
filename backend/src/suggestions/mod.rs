//! Commitment detection: after the user sends an email, scan what THEY wrote
//! for time-bound promises ("I'll do the maintenance Saturday 9pm") and store
//! accept/dismiss suggestions for the Email window. Detached and best-effort —
//! never affects the send.

use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;

use crate::db;
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig};
use crate::state::AppState;

const BODY_LIMIT: usize = 4000;

const DETECT_SYSTEM: &str = "The user just SENT the email below. Find commitments THE USER \
made to do something at or by a specific time — promises of future action with a stated or \
clearly implied date/time. Ignore commitments made by other people, past events, and vague \
intentions with no timeframe.\n\n\
Classify each commitment:\n\
- \"event\": appointment-like, happens at a specific start time (e.g. performing maintenance \
at 9pm, attending a meeting). Include \"start\" and, when stated, \"end\".\n\
- \"task\": deadline-like, something to finish by a time (e.g. sending a quote by Friday). \
Include \"start\" as the due time when one is stated, otherwise omit it.\n\n\
Times must be RFC3339 with the user's UTC offset, resolved against the current date/time \
given below.\n\n\
Respond with ONLY a JSON array, no prose, no code fences. Each element: \
{\"kind\": \"task\"|\"event\", \"title\": \"<short imperative description>\", \
\"start\": \"<RFC3339>\"?, \"end\": \"<RFC3339>\"?}. If there are no commitments, return [].";

#[derive(Debug, Deserialize)]
struct Detected {
    #[serde(default)]
    kind: String,
    title: String,
    start: Option<String>,
    end: Option<String>,
}

/// Parse a model-supplied RFC3339 time into UTC for storage; None if invalid.
fn to_utc(s: &Option<String>) -> Option<String> {
    s.as_deref().and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc).to_rfc3339())
    })
}

/// An event needs a start time to be schedulable; downgrade to a task if the
/// model classified something as an event without one.
fn normalize_kind(kind: &str, start: &Option<String>) -> &'static str {
    match (kind, start.is_some()) {
        ("event", true) => "event",
        _ => "task",
    }
}

/// Extract the JSON array, tolerating code fences / surrounding prose.
fn parse(raw: &str) -> anyhow::Result<Vec<Detected>> {
    let start = raw.find('[').ok_or_else(|| anyhow::anyhow!("no JSON array in model output"))?;
    let end = raw.rfind(']').ok_or_else(|| anyhow::anyhow!("no JSON array in model output"))?;
    if end < start {
        anyhow::bail!("malformed JSON array");
    }
    Ok(serde_json::from_str(&raw[start..=end])?)
}

/// Scan a sent email for commitments and store pending suggestions.
pub async fn detect_commitments(
    state: &AppState,
    provider: ProviderConfig,
    body: String,
    context: String,
) {
    if body.trim().is_empty() {
        return;
    }

    let tz = state.home_tz().await;
    let now = Utc::now().with_timezone(&tz);
    let body_capped: String = body.chars().take(BODY_LIMIT).collect();
    let user = format!(
        "The current date and time is {} in the user's timezone ({}, UTC{}).\n\n\
Email the user sent ({context}):\n\n{body_capped}",
        now.format("%A, %-d %B %Y, %-I:%M %p"),
        tz.name(),
        now.format("%:z"),
    );
    let history = vec![
        ChatMessage { role: "system".to_string(), content: Value::String(DETECT_SYSTEM.to_string()) },
        ChatMessage { role: "user".to_string(), content: Value::String(user) },
    ];

    let raw = match ModelRouter::complete(&provider, history).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("commitment detection failed: {e}");
            return;
        }
    };
    let items = match parse(&raw) {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!("commitment detection parse failed: {e}");
            return;
        }
    };

    for item in items {
        let title = item.title.trim();
        if title.is_empty() {
            continue;
        }
        let start_at = to_utc(&item.start);
        let end_at = to_utc(&item.end);
        let kind = normalize_kind(&item.kind, &start_at);

        match db::suggestions::insert(
            &state.db,
            kind,
            title,
            start_at.as_deref(),
            end_at.as_deref(),
            Some(&context),
        )
        .await
        {
            Ok(_) => {
                state
                    .log("suggestions", "info", format!("detected commitment ({kind}): {title}"))
                    .await
            }
            Err(e) => tracing::warn!("failed to save suggestion: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_without_start_downgrades_to_task() {
        assert_eq!(normalize_kind("event", &None), "task");
        assert_eq!(normalize_kind("event", &Some("2026-06-06T11:00:00+00:00".into())), "event");
        assert_eq!(normalize_kind("task", &Some("x".into())), "task");
        assert_eq!(normalize_kind("banana", &Some("x".into())), "task");
    }

    #[test]
    fn to_utc_converts_offsets() {
        assert_eq!(
            to_utc(&Some("2026-06-06T21:00:00+10:00".into())).as_deref(),
            Some("2026-06-06T11:00:00+00:00")
        );
        assert_eq!(to_utc(&Some("garbage".into())), None);
        assert_eq!(to_utc(&None), None);
    }

    #[test]
    fn parse_tolerates_prose_and_fences() {
        let raw = "Here:\n```json\n[{\"kind\":\"event\",\"title\":\"Server maintenance\",\"start\":\"2026-06-06T21:00:00+10:00\"}]\n```";
        let items = parse(raw).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "event");
    }
}
