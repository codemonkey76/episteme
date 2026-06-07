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

/// Drop legal-disclaimer / signature boilerplate paragraphs (Exclaimer-style
/// footers repeated through a thread) — pure noise for commitment detection.
fn strip_boilerplate(text: &str) -> String {
    const MARKERS: [&str; 5] = [
        "confidential and intended solely",
        "if you have received this email in error",
        "received it by mistake",
        "presence of viruses",
        "accepts no liability",
    ];
    text.split("\n\n")
        .filter(|para| {
            let p = para.to_lowercase();
            !MARKERS.iter().any(|m| p.contains(m))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
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
/// `reply_context` is the message being replied to, so terse replies
/// ("I'll get this done tonight") resolve to something specific.
pub async fn detect_commitments(
    state: &AppState,
    user_id: &str,
    provider: ProviderConfig,
    body: String,
    context: String,
    reply_context: Option<String>,
) {
    if body.trim().is_empty() {
        return;
    }

    let tz = state.home_tz(user_id).await;
    let now = Utc::now().with_timezone(&tz);
    let body_capped: String = body.chars().take(BODY_LIMIT).collect();
    let replied = reply_context
        .as_deref()
        .map(strip_boilerplate)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let capped: String = s.chars().take(BODY_LIMIT / 2).collect();
            format!("\n\nThe message being replied to (context only — do NOT extract the other party's commitments from it):\n\n{capped}")
        })
        .unwrap_or_default();
    let user = format!(
        "The current date and time is {} in the user's timezone ({}, UTC{}).\n\n\
Email the user sent ({context}):\n\n{body_capped}{replied}",
        now.format("%A, %-d %B %Y, %-I:%M %p"),
        tz.name(),
        now.format("%:z"),
    );
    let system = crate::prompts::get(&state.db, "commitment_detect").await;
    let history = vec![
        ChatMessage { role: "system".to_string(), content: Value::String(system) },
        ChatMessage { role: "user".to_string(), content: Value::String(user) },
    ];

    let raw = match ModelRouter::complete_with_usage(&provider, history).await {
        Ok((r, used)) => {
            db::usage::record(&state.db, user_id, &provider, "commitments", used).await;
            r
        }
        Err(e) => {
            state
                .log("suggestions", "error", format!("commitment detection failed: {e}"))
                .await;
            return;
        }
    };
    let items = match parse(&raw) {
        Ok(i) => i,
        Err(e) => {
            state
                .log("suggestions", "error", format!("commitment detection parse failed: {e}"))
                .await;
            return;
        }
    };
    if items.is_empty() {
        // Visible in the Logs window so "nothing happened" is diagnosable.
        state
            .log("suggestions", "info", format!("no commitments detected ({context})"))
            .await;
        return;
    }

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
            user_id,
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
                    .await;
                crate::integrations::push::notify(
                    state,
                    user_id,
                    "Commitment detected",
                    &format!("{title} — open episteme to add it as a {kind}"),
                )
                .await;
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
    fn strips_disclaimer_paragraphs() {
        let text = "Sorry Shane, he wants it done after hours.\n\n\
This email and any files transmitted with it are confidential and intended solely for the \
use of the individual or entity to whom they are addressed. The company accepts no liability \
for any damage caused by any virus transmitted by this email.";
        let cleaned = strip_boilerplate(text);
        assert_eq!(cleaned, "Sorry Shane, he wants it done after hours.");
    }

    #[test]
    fn parse_tolerates_prose_and_fences() {
        let raw = "Here:\n```json\n[{\"kind\":\"event\",\"title\":\"Server maintenance\",\"start\":\"2026-06-06T21:00:00+10:00\"}]\n```";
        let items = parse(raw).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "event");
    }
}
