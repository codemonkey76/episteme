//! Shipment extraction: the second pass over mail the classifier put in the
//! `deliveries` category. Pulls carrier, tracking number, ETA and status out of
//! each shipping email and folds it into the user's shipment list, so "what's
//! on the way" is a list you can look at rather than a folder you have to read.
//!
//! Runs inline with the sort (like `handle_ticket_update`) — delivery mail is
//! low volume, and a shipment that appears minutes after its email is no use if
//! the sort itself has already been reported as finished.

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::db;
use crate::db::shipments::Extracted;
use crate::integrations::graph;
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig};
use crate::state::AppState;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
/// Plain-text body characters fed to the extractor. Shipping mail is mostly
/// boilerplate and the useful details sit near the top.
const BODY_LIMIT: usize = 3000;

/// The extractor's reply. Everything but `is_shipment` is best-effort: a
/// dispatch notice that names no carrier still produces a usable shipment.
#[derive(Debug, Default, Deserialize)]
struct Details {
    /// False for delivery-adjacent mail that tracks nothing (a promo from a
    /// courier, a review request for a parcel already received).
    #[serde(default)]
    is_shipment: bool,
    #[serde(default)]
    label: String,
    #[serde(default)]
    merchant: String,
    #[serde(default)]
    carrier: String,
    #[serde(default)]
    tracking_number: String,
    #[serde(default)]
    tracking_url: String,
    #[serde(default)]
    order_ref: String,
    #[serde(default)]
    status: String,
    /// Estimated delivery, RFC3339 with the user's offset.
    #[serde(default)]
    eta: String,
    /// One line describing this update, for the shipment's timeline.
    #[serde(default)]
    summary: String,
}

fn some_trimmed(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Keep only http(s) links — the model occasionally returns a bare domain or a
/// "N/A", and a junk href in the UI is worse than no button.
fn some_url(s: &str) -> Option<String> {
    some_trimmed(s).filter(|u| u.starts_with("http://") || u.starts_with("https://"))
}

/// Normalize a model-supplied ETA to UTC. Accepts a full RFC3339 timestamp or a
/// bare `YYYY-MM-DD` (carriers usually promise a day, not a time), which is
/// pinned to 17:00 local — an all-day ETA sorted at midnight reads as overdue
/// for the whole day it's actually due.
fn parse_eta(raw: &str, tz: chrono_tz::Tz) -> Option<String> {
    use chrono::TimeZone;
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&chrono::Utc).to_rfc3339());
    }
    let date = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    let naive = date.and_hms_opt(17, 0, 0)?;
    tz.from_local_datetime(&naive)
        .single()
        .map(|dt| dt.with_timezone(&chrono::Utc).to_rfc3339())
}

/// Extract one shipping email's details and fold them into the shipment list.
/// Returns true when a shipment was created or advanced.
pub async fn handle(
    state: &AppState,
    provider: &ProviderConfig,
    user_id: &str,
    account_id: &str,
    seg: &str,
    msg: &Value,
) -> Result<bool> {
    let id = msg["id"].as_str().ok_or_else(|| anyhow::anyhow!("message missing id"))?;
    let subject = msg["subject"].as_str().unwrap_or("(no subject)");
    let from = msg["from"]["emailAddress"]["address"].as_str().unwrap_or("");
    let conversation_id = msg["conversationId"].as_str().filter(|s| !s.is_empty());
    let received = msg["receivedDateTime"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc).to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    // Tracking numbers and ETAs live in the body, not the preview the list
    // fetch carried, so this pass pays for the full message.
    let full = graph::graph_get(
        state,
        user_id,
        Some(account_id),
        &format!("{GRAPH}/{seg}/messages/{id}"),
        &[("$select", "body")],
    )
    .await?;
    let body_raw = full["body"]["content"].as_str().unwrap_or("");
    let is_html = full["body"]["contentType"]
        .as_str()
        .map(|c| c.eq_ignore_ascii_case("html"))
        .unwrap_or(false);
    let body_text = if is_html { graph::html_to_text(body_raw) } else { body_raw.to_string() };
    let body_text: String = body_text.chars().take(BODY_LIMIT).collect();

    let tz = state.home_tz(user_id).await;
    let now = chrono::Utc::now().with_timezone(&tz);
    let system = crate::prompts::get(&state.db, "shipment_extract").await;
    let user_msg = format!(
        "The current date and time is {} in the user's timezone ({}, UTC{}).\n\n\
         From: {from}\nSubject: {subject}\n\n{body_text}",
        now.format("%A, %-d %B %Y, %-I:%M %p"),
        tz.name(),
        now.format("%:z"),
    );

    // Pin the shape: a local model given only "reply with JSON" will sometimes
    // answer with prose about the parcel instead of the object.
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "is_shipment": { "type": "boolean" },
            "label": { "type": "string" },
            "merchant": { "type": "string" },
            "carrier": { "type": "string" },
            "tracking_number": { "type": "string" },
            "tracking_url": { "type": "string" },
            "order_ref": { "type": "string" },
            "status": { "type": "string" },
            "eta": { "type": "string" },
            "summary": { "type": "string" },
        },
        "required": ["is_shipment", "summary"],
    });
    let (raw, used) = ModelRouter::complete_json_schema_with_usage(
        provider,
        vec![
            ChatMessage { role: "system".to_string(), content: Value::String(system) },
            ChatMessage { role: "user".to_string(), content: Value::String(user_msg) },
        ],
        schema,
    )
    .await?;
    db::usage::record(&state.db, user_id, provider, "shipment-extract", used).await;

    let details = parse_details(&raw)?;
    if !details.is_shipment {
        return Ok(false);
    }

    let label = some_trimmed(&details.label)
        .or_else(|| some_trimmed(&details.merchant).map(|m| format!("Order from {m}")))
        .unwrap_or_else(|| subject.to_string());
    let extracted = Extracted {
        label,
        carrier: some_trimmed(&details.carrier),
        tracking_number: some_trimmed(&details.tracking_number),
        tracking_url: some_url(&details.tracking_url),
        merchant: some_trimmed(&details.merchant),
        order_ref: some_trimmed(&details.order_ref),
        status: some_trimmed(&details.status),
        eta: parse_eta(&details.eta, tz),
        detail: some_trimmed(&details.summary).unwrap_or_else(|| subject.to_string()),
    };

    let result =
        db::shipments::upsert_from_email(&state.db, user_id, conversation_id, &extracted, &received)
            .await?;

    // Only speak up when something actually moved. A carrier that emails the
    // same "in transit" line daily shouldn't buzz the user's phone daily.
    if result.created || result.changed {
        let title = if result.created {
            format!("Tracking: {}", result.shipment.label)
        } else {
            result.shipment.label.clone()
        };
        crate::integrations::push::notify_linked(
            state,
            user_id,
            &title,
            &extracted.detail,
            "shipment",
            Some(crate::integrations::push::Link { kind: "shipment", id: &result.shipment.id }),
        )
        .await;
    }
    Ok(result.created || result.changed)
}

/// Pull the details object out of a model reply, tolerating code fences or
/// surrounding prose.
fn parse_details(raw: &str) -> Result<Details> {
    let start = raw.find('{').ok_or_else(|| anyhow::anyhow!("no JSON object in model output"))?;
    let end = raw.rfind('}').ok_or_else(|| anyhow::anyhow!("no JSON object in model output"))?;
    if end < start {
        anyhow::bail!("malformed JSON object in model output");
    }
    Ok(serde_json::from_str(&raw[start..=end])?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_object_in_code_fence() {
        let raw = "```json\n{\"is_shipment\":true,\"carrier\":\"Australia Post\",\
                   \"summary\":\"Dispatched\"}\n```";
        let d = parse_details(raw).unwrap();
        assert!(d.is_shipment);
        assert_eq!(d.carrier, "Australia Post");
    }

    #[test]
    fn parse_tolerates_missing_optional_fields() {
        let d = parse_details("{\"is_shipment\":false,\"summary\":\"Marketing email\"}").unwrap();
        assert!(!d.is_shipment);
        assert_eq!(d.tracking_number, "");
    }

    #[test]
    fn eta_date_only_pins_to_local_evening() {
        // 2026-08-20 17:00 in Adelaide (UTC+9:30) is 07:30Z the same day.
        let tz: chrono_tz::Tz = "Australia/Adelaide".parse().unwrap();
        let eta = parse_eta("2026-08-20", tz).unwrap();
        assert!(eta.starts_with("2026-08-20T07:30:00"), "got {eta}");
    }

    #[test]
    fn eta_rfc3339_converts_to_utc() {
        let tz: chrono_tz::Tz = "Australia/Adelaide".parse().unwrap();
        let eta = parse_eta("2026-08-20T14:00:00+09:30", tz).unwrap();
        assert!(eta.starts_with("2026-08-20T04:30:00"), "got {eta}");
    }

    #[test]
    fn eta_rejects_junk() {
        let tz = chrono_tz::Tz::UTC;
        assert!(parse_eta("unknown", tz).is_none());
        assert!(parse_eta("", tz).is_none());
    }

    #[test]
    fn tracking_url_must_be_http() {
        assert!(some_url("auspost.com.au/track").is_none());
        assert_eq!(
            some_url("https://auspost.com.au/track/ABC"),
            Some("https://auspost.com.au/track/ABC".to_string())
        );
    }
}
