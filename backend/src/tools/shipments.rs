//! Shipment tools — model-facing adapter over `crate::db::shipments` (what's
//! on the way). Lets the user say "I ordered a keyboard, arriving Tuesday" in
//! chat and have it tracked alongside the shipments the email categorizer
//! creates on its own. ETAs arrive as RFC3339 with the user's offset (per the
//! system preamble), are stored UTC, and are localized in every result.

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::{json, Value};

use crate::db::{self, shipments::ShipmentPatch};
use crate::state::AppState;

use super::localize;

/// Event history lines surfaced per shipment. The full timeline is in the UI;
/// the model only needs the recent shape of things.
const MAX_EVENTS: usize = 5;

const STATUS_DESC: &str = "One of: ordered, in_transit, out_for_delivery, delivered, exception (delayed/held/failed), cancelled.";

pub fn schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "list_shipments",
            "description": "List the user's tracked shipments — what they have on the way, where each is up to, and its ETA. Use for questions like \"what's arriving this week\" or \"where's my parcel\". Defaults to shipments not yet delivered.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "description": "Filter: \"active\" (default, anything not delivered or cancelled), \"all\", or a specific status. " },
                    "q": { "type": "string", "description": "Search term matched against the label, description, merchant and tracking number." }
                }
            }
        }),
        json!({
            "name": "create_shipment",
            "description": "Track something the user has on the way. Use when they mention having ordered something or expecting a delivery. Only the label is required — record whatever else they said and leave the rest out.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "label": { "type": "string", "description": "What is on the way, in a few words (\"Framework mainboard\")." },
                    "description": { "type": "string", "description": "Optional extra detail." },
                    "merchant": { "type": "string", "description": "Who it was ordered from." },
                    "carrier": { "type": "string", "description": "Who is carrying it (Australia Post, DHL…)." },
                    "tracking_number": { "type": "string", "description": "Carrier tracking number, exactly as given. Never invent one." },
                    "tracking_url": { "type": "string", "description": "Full https link to the carrier's tracking page, if known. Never construct one." },
                    "order_ref": { "type": "string", "description": "The merchant's order number." },
                    "status": { "type": "string", "description": STATUS_DESC },
                    "eta": { "type": "string", "description": "Expected delivery, RFC3339 with the user's UTC offset from the system message." }
                },
                "required": ["label"]
            }
        }),
        json!({
            "name": "update_shipment",
            "description": "Change a tracked shipment's details or status (e.g. mark it delivered, correct the ETA, add the tracking number). Get ids from list_shipments first.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "shipment_id": { "type": "string" },
                    "label": { "type": "string" },
                    "description": { "type": "string" },
                    "merchant": { "type": "string" },
                    "carrier": { "type": "string" },
                    "tracking_number": { "type": "string" },
                    "tracking_url": { "type": "string" },
                    "order_ref": { "type": "string" },
                    "status": { "type": "string", "description": STATUS_DESC },
                    "eta": { "type": "string", "description": "RFC3339 with the user's UTC offset, or empty string to clear." }
                },
                "required": ["shipment_id"]
            }
        }),
        json!({
            "name": "add_shipment_update",
            "description": "Add a line to a shipment's history (\"left with the neighbour\", \"driver couldn't find the address\"), optionally moving its status. Get ids from list_shipments first.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "shipment_id": { "type": "string" },
                    "detail": { "type": "string", "description": "One line describing the update." },
                    "status": { "type": "string", "description": STATUS_DESC }
                },
                "required": ["shipment_id", "detail"]
            }
        }),
        json!({
            "name": "delete_shipment",
            "description": "Stop tracking a shipment and delete its history. Prefer update_shipment with status \"delivered\" for parcels that arrived. Get ids from list_shipments first.",
            "input_schema": {
                "type": "object",
                "properties": { "shipment_id": { "type": "string" } },
                "required": ["shipment_id"]
            }
        }),
    ]
}

pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "list_shipments"
            | "create_shipment"
            | "update_shipment"
            | "add_shipment_update"
            | "delete_shipment"
    )
}

/// Parse a model-supplied ETA (RFC3339 with offset) into UTC for storage.
fn eta_to_utc(s: &str) -> Result<String> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc).to_rfc3339())
        .map_err(|_| {
            anyhow!("invalid eta '{s}' — expected RFC3339 e.g. 2026-06-05T15:00:00+10:00")
        })
}

/// Validate a model-supplied status against what the database accepts.
fn status_arg(args: &Value) -> Result<Option<&'static str>> {
    match args["status"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => db::shipments::normalize_status(s).map(Some).ok_or_else(|| {
            anyhow!("invalid status '{s}' — expected one of: ordered, in_transit, out_for_delivery, delivered, exception, cancelled")
        }),
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args[key].as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// Serialize a shipment for the model: ETA localized with a human display
/// string, the photo flag and provenance dropped (neither means anything to the
/// model), and the history trimmed to the most recent lines.
fn localize_shipment(s: &db::shipments::Shipment, tz: chrono_tz::Tz) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or_default();
    for (field, display_field) in [("eta", "eta_display"), ("delivered_at", "delivered_display")] {
        if let Some(raw) = v[field].as_str().map(str::to_string) {
            let (rfc, display) = localize(&raw, tz);
            v[field] = Value::String(rfc);
            if let Some(d) = display {
                v[display_field] = Value::String(d);
            }
        }
    }
    let events: Vec<Value> = s
        .events
        .iter()
        .take(MAX_EVENTS)
        .map(|e| {
            let (_, display) = localize(&e.occurred_at, tz);
            json!({
                "detail": e.detail,
                "status": e.status,
                "when": display.unwrap_or_else(|| e.occurred_at.clone()),
            })
        })
        .collect();
    if let Some(obj) = v.as_object_mut() {
        obj.remove("has_photo");
        obj.remove("conversation_id");
        obj.insert("history".to_string(), Value::Array(events));
        obj.remove("events");
    }
    v
}

pub async fn execute(state: &AppState, user_id: &str, name: &str, args: Value) -> Result<Value> {
    let tz = state.home_tz(user_id).await;
    match name {
        "list_shipments" => {
            let status = str_arg(&args, "status")
                .filter(|s| !s.eq_ignore_ascii_case("all"))
                .unwrap_or("active");
            let q = str_arg(&args, "q");
            let shipments = db::shipments::list(&state.db, user_id, Some(status), q, 100).await?;
            let shipments: Vec<Value> =
                shipments.iter().map(|s| localize_shipment(s, tz)).collect();
            Ok(json!({ "shipments": shipments }))
        }
        "create_shipment" => {
            let label = str_arg(&args, "label").ok_or_else(|| anyhow!("label is required"))?;
            let eta = match str_arg(&args, "eta") {
                Some(s) => Some(eta_to_utc(s)?),
                None => None,
            };
            let shipment = db::shipments::insert(
                &state.db,
                user_id,
                label,
                str_arg(&args, "description"),
                str_arg(&args, "carrier"),
                str_arg(&args, "tracking_number"),
                str_arg(&args, "tracking_url"),
                str_arg(&args, "merchant"),
                str_arg(&args, "order_ref"),
                status_arg(&args)?.unwrap_or("ordered"),
                eta.as_deref(),
                "agent",
                None,
            )
            .await?;
            Ok(json!({ "created": localize_shipment(&shipment, tz) }))
        }
        "update_shipment" => {
            let id = str_arg(&args, "shipment_id")
                .ok_or_else(|| anyhow!("shipment_id is required"))?;
            // An empty string clears a field; an absent one leaves it alone.
            fn clearable(args: &Value, key: &str) -> Option<Option<String>> {
                args[key].as_str().map(|s| {
                    let t = s.trim();
                    (!t.is_empty()).then(|| t.to_string())
                })
            }
            let patch = ShipmentPatch {
                label: str_arg(&args, "label").map(str::to_string),
                description: clearable(&args, "description"),
                carrier: clearable(&args, "carrier"),
                tracking_number: clearable(&args, "tracking_number"),
                tracking_url: clearable(&args, "tracking_url"),
                merchant: clearable(&args, "merchant"),
                order_ref: clearable(&args, "order_ref"),
                status: status_arg(&args)?.map(str::to_string),
                eta: match args["eta"].as_str() {
                    None => None,
                    Some(s) if s.trim().is_empty() => Some(None),
                    Some(s) => Some(Some(eta_to_utc(s.trim())?)),
                },
            };
            let shipment = db::shipments::update(&state.db, user_id, id, patch)
                .await?
                .ok_or_else(|| anyhow!("no shipment with id '{id}'"))?;
            Ok(json!({ "updated": localize_shipment(&shipment, tz) }))
        }
        "add_shipment_update" => {
            let id = str_arg(&args, "shipment_id")
                .ok_or_else(|| anyhow!("shipment_id is required"))?;
            let detail = str_arg(&args, "detail").ok_or_else(|| anyhow!("detail is required"))?;
            // Confirms the shipment is this user's before writing a child row.
            db::shipments::get(&state.db, user_id, id)
                .await?
                .ok_or_else(|| anyhow!("no shipment with id '{id}'"))?;
            let status = status_arg(&args)?;
            db::shipments::add_event(
                &state.db,
                id,
                status,
                detail,
                &Utc::now().to_rfc3339(),
                "agent",
            )
            .await?;
            if let Some(s) = status {
                db::shipments::update(
                    &state.db,
                    user_id,
                    id,
                    ShipmentPatch { status: Some(s.to_string()), ..Default::default() },
                )
                .await?;
            }
            let shipment = db::shipments::get(&state.db, user_id, id)
                .await?
                .ok_or_else(|| anyhow!("no shipment with id '{id}'"))?;
            Ok(json!({ "updated": localize_shipment(&shipment, tz) }))
        }
        "delete_shipment" => {
            let id = str_arg(&args, "shipment_id")
                .ok_or_else(|| anyhow!("shipment_id is required"))?;
            db::shipments::delete(&state.db, user_id, id).await?;
            Ok(json!({ "deleted": id }))
        }
        other => Err(anyhow!("unknown shipment tool '{other}'")),
    }
}
