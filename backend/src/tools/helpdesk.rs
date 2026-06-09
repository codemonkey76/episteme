//! Helpdesk tools — model-facing adapter over the user's custom helpdesk
//! platform via its Sanctum API (see `integrations::helpdesk`). Covers the
//! agent workflow: find/create tickets, reply (or leave internal notes), log
//! work time, and update status/priority/assignment. Ticket ids are numeric
//! helpdesk ids; references (TKT-00042)
//! resolve via list/search first.

use anyhow::{anyhow, Result};
use reqwest::Method;
use serde_json::{json, Value};

use crate::integrations::helpdesk;
use crate::state::AppState;

pub fn schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "helpdesk_list_tickets",
            "description": "List open helpdesk tickets (excludes closed), most urgent first. Supports filtering by status and searching subject/reference (e.g. TKT-00042). Use this to find a ticket's id before acting on it.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["open", "in_progress", "pending_user", "resolved"], "description": "Optional status filter." },
                    "search": { "type": "string", "description": "Optional subject or reference search." }
                }
            }
        }),
        json!({
            "name": "helpdesk_get_ticket",
            "description": "Fetch a helpdesk ticket in full: description, conversation messages, and logged time entries. Get the id from helpdesk_list_tickets first.",
            "input_schema": {
                "type": "object",
                "properties": { "ticket_id": { "type": "integer" } },
                "required": ["ticket_id"]
            }
        }),
        json!({
            "name": "helpdesk_list_clients",
            "description": "List helpdesk clients with their users (contacts). Needed to pick client_id and user_id when creating a ticket. Supports a name search.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "search": { "type": "string", "description": "Optional client name filter." }
                }
            }
        }),
        json!({
            "name": "helpdesk_create_user",
            "description": "Create a new helpdesk customer contact (always role 'User') belonging to one or more clients. Resolve the client id(s) via helpdesk_list_clients first. Only name and email are required; mobile is optional (the user can add their own later). The new user is emailed an invite to set their password. Use this when asked to add a contact/person to a client.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "email": { "type": "string" },
                    "mobile": { "type": "string", "description": "Optional mobile number." },
                    "client_ids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "Ids of the client(s) this user belongs to (from helpdesk_list_clients). At least one."
                    },
                    "default_client_id": { "type": "integer", "description": "Optional primary client; defaults to the first of client_ids." }
                },
                "required": ["name", "email", "client_ids"]
            }
        }),
        json!({
            "name": "helpdesk_create_ticket",
            "description": "Create a new helpdesk ticket. Resolve client_id and user_id (the requesting contact at that client) via helpdesk_list_clients first; ask the user if the contact is ambiguous. Set silent=true for a silent ticket: the customer is never emailed about it (no creation, reply, or status email) — use for work tracked internally that shouldn't notify the customer.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "client_id": { "type": "integer" },
                    "user_id": { "type": "integer", "description": "The requesting contact's user id (from helpdesk_list_clients)." },
                    "subject": { "type": "string" },
                    "description": { "type": "string", "description": "Full description of the issue." },
                    "priority": { "type": "string", "enum": ["low", "medium", "high", "critical"] },
                    "silent": { "type": "boolean", "description": "If true, suppress all customer-facing email for this ticket. Default false." }
                },
                "required": ["client_id", "user_id", "subject", "description", "priority"]
            }
        }),
        json!({
            "name": "helpdesk_reply_ticket",
            "description": "Add a response to a helpdesk ticket. type \"reply\" emails the customer; \"internal_note\" is agent-only. Get the ticket id from helpdesk_list_tickets first.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "ticket_id": { "type": "integer" },
                    "body": { "type": "string" },
                    "type": { "type": "string", "enum": ["reply", "internal_note"], "description": "Default reply." }
                },
                "required": ["ticket_id", "body"]
            }
        }),
        json!({
            "name": "helpdesk_log_time",
            "description": "Log work time against a helpdesk ticket, in 15-minute blocks (15–480 minutes).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "ticket_id": { "type": "integer" },
                    "duration_minutes": { "type": "integer", "description": "Multiple of 15, between 15 and 480." },
                    "work_type": { "type": "string", "enum": ["remote", "on_site"] },
                    "description": { "type": "string", "description": "What the time was spent on." },
                    "logged_at": { "type": "string", "description": "Date the work was done, YYYY-MM-DD (today or earlier). Defaults to today." }
                },
                "required": ["ticket_id", "duration_minutes", "work_type", "description"]
            }
        }),
        json!({
            "name": "helpdesk_delete_time",
            "description": "Delete a logged time entry from a ticket. Get the time entry's id (and its ticket_id) from helpdesk_get_ticket first. Locked (invoiced) entries cannot be deleted. This is permanent.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "ticket_id": { "type": "integer" },
                    "time_entry_id": { "type": "integer", "description": "The id of the time entry to delete (from helpdesk_get_ticket)." }
                },
                "required": ["ticket_id", "time_entry_id"]
            }
        }),
        json!({
            "name": "helpdesk_update_ticket",
            "description": "Change a helpdesk ticket's status and/or priority (e.g. mark resolved after logging time), assign it to yourself, and/or toggle its silent flag. Set assign_to_me to true to assign the ticket to the connected agent (you). Set silent to true to suppress all customer-facing email for this ticket, or false to re-enable customer email on a previously silent ticket.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "ticket_id": { "type": "integer" },
                    "status": { "type": "string", "enum": ["open", "in_progress", "pending_user", "resolved", "closed"] },
                    "priority": { "type": "string", "enum": ["low", "medium", "high", "critical"] },
                    "assign_to_me": { "type": "boolean", "description": "Assign the ticket to the connected agent (yourself)." },
                    "silent": { "type": "boolean", "description": "Set true to suppress all customer-facing email for this ticket, or false to re-enable it." }
                },
                "required": ["ticket_id"]
            }
        }),
    ]
}

pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "helpdesk_list_tickets"
            | "helpdesk_get_ticket"
            | "helpdesk_list_clients"
            | "helpdesk_create_user"
            | "helpdesk_create_ticket"
            | "helpdesk_reply_ticket"
            | "helpdesk_log_time"
            | "helpdesk_delete_time"
            | "helpdesk_update_ticket"
    )
}

fn ticket_id(args: &Value) -> Result<i64> {
    args["ticket_id"].as_i64().ok_or_else(|| anyhow!("ticket_id is required"))
}

/// Trim a detail ticket for the model: drop attachment noise from messages,
/// keep the conversation and time entries readable.
fn slim_detail(t: &Value) -> Value {
    let mut out = t.clone();
    if let Some(msgs) = out["messages"].as_array_mut() {
        for m in msgs {
            if let Some(obj) = m.as_object_mut() {
                obj.remove("attachments");
            }
        }
    }
    out
}

pub async fn execute(state: &AppState, user_id: &str, name: &str, args: Value) -> Result<Value> {
    match name {
        "helpdesk_list_tickets" => {
            let mut path = "/tickets?per_page=25".to_string();
            if let Some(s) = args["status"].as_str().filter(|s| !s.is_empty()) {
                path.push_str(&format!("&status={}", urlencoding::encode(s)));
            }
            if let Some(s) = args["search"].as_str().filter(|s| !s.is_empty()) {
                path.push_str(&format!("&search={}", urlencoding::encode(s)));
            }
            let res = helpdesk::request(state, user_id, Method::GET, &path, None).await?;
            Ok(json!({ "tickets": res["data"] }))
        }
        "helpdesk_get_ticket" => {
            let id = ticket_id(&args)?;
            let res = helpdesk::request(state, user_id, Method::GET, &format!("/tickets/{id}"), None).await?;
            Ok(json!({ "ticket": slim_detail(&res["data"]) }))
        }
        "helpdesk_list_clients" => {
            let mut path = "/clients".to_string();
            if let Some(s) = args["search"].as_str().filter(|s| !s.is_empty()) {
                path.push_str(&format!("?search={}", urlencoding::encode(s)));
            }
            let res = helpdesk::request(state, user_id, Method::GET, &path, None).await?;
            Ok(json!({ "clients": res["data"] }))
        }
        "helpdesk_create_user" => {
            let client_ids: Vec<i64> = args["client_ids"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
                .unwrap_or_default();
            if client_ids.is_empty() {
                return Err(anyhow!("client_ids must list at least one client"));
            }
            let mut body = serde_json::Map::new();
            body.insert("name".into(), json!(args["name"].as_str().ok_or_else(|| anyhow!("name is required"))?));
            body.insert("email".into(), json!(args["email"].as_str().ok_or_else(|| anyhow!("email is required"))?));
            body.insert("client_ids".into(), json!(client_ids));
            if let Some(m) = args["mobile"].as_str().filter(|s| !s.is_empty()) {
                body.insert("mobile".into(), json!(m));
            }
            if let Some(d) = args["default_client_id"].as_i64() {
                body.insert("default_client_id".into(), json!(d));
            }
            let res = helpdesk::request(state, user_id, Method::POST, "/users", Some(&Value::Object(body))).await?;
            state
                .log("helpdesk", "info", format!(
                    "User created: {} <{}>",
                    res["data"]["name"].as_str().unwrap_or("?"),
                    res["data"]["email"].as_str().unwrap_or("")
                ))
                .await;
            Ok(json!({ "created": res["data"] }))
        }
        "helpdesk_create_ticket" => {
            let body = json!({
                "client_id": args["client_id"].as_i64().ok_or_else(|| anyhow!("client_id is required"))?,
                "user_id": args["user_id"].as_i64().ok_or_else(|| anyhow!("user_id is required"))?,
                "subject": args["subject"].as_str().ok_or_else(|| anyhow!("subject is required"))?,
                "description": args["description"].as_str().ok_or_else(|| anyhow!("description is required"))?,
                "priority": args["priority"].as_str().unwrap_or("medium"),
                "silent": args["silent"].as_bool().unwrap_or(false),
                "source": "api",
            });
            let res = helpdesk::request(state, user_id, Method::POST, "/tickets", Some(&body)).await?;
            state
                .log("helpdesk", "info", format!(
                    "Ticket created: {} — {}",
                    res["data"]["reference"].as_str().unwrap_or("?"),
                    res["data"]["subject"].as_str().unwrap_or("")
                ))
                .await;
            Ok(json!({ "created": slim_detail(&res["data"]) }))
        }
        "helpdesk_reply_ticket" => {
            let id = ticket_id(&args)?;
            let kind = match args["type"].as_str() {
                Some(t @ ("reply" | "internal_note")) => t,
                _ => "reply",
            };
            let body = json!({
                "body": args["body"].as_str().ok_or_else(|| anyhow!("body is required"))?,
                "type": kind,
            });
            let res = helpdesk::request(state, user_id, Method::POST, &format!("/tickets/{id}/messages"), Some(&body)).await?;
            state
                .log("helpdesk", "info", format!("{kind} added to ticket #{id}"))
                .await;
            Ok(json!({ "message": res["data"] }))
        }
        "helpdesk_log_time" => {
            let id = ticket_id(&args)?;
            let minutes = args["duration_minutes"]
                .as_i64()
                .ok_or_else(|| anyhow!("duration_minutes is required"))?;
            if minutes < 15 || minutes > 480 || minutes % 15 != 0 {
                return Err(anyhow!("duration_minutes must be a multiple of 15 between 15 and 480"));
            }
            // Default logged_at to today in the user's home timezone.
            let logged_at = match args["logged_at"].as_str().filter(|s| !s.is_empty()) {
                Some(s) => s.to_string(),
                None => {
                    let tz = state.home_tz(user_id).await;
                    chrono::Utc::now().with_timezone(&tz).format("%Y-%m-%d").to_string()
                }
            };
            let body = json!({
                "duration_minutes": minutes,
                "work_type": args["work_type"].as_str().ok_or_else(|| anyhow!("work_type is required"))?,
                "description": args["description"].as_str().ok_or_else(|| anyhow!("description is required"))?,
                "logged_at": logged_at,
            });
            let res = helpdesk::request(state, user_id, Method::POST, &format!("/tickets/{id}/time-entries"), Some(&body)).await?;
            state
                .log("helpdesk", "info", format!("Logged {minutes}m against ticket #{id}"))
                .await;
            Ok(json!({ "time_entry": res["data"] }))
        }
        "helpdesk_delete_time" => {
            let id = ticket_id(&args)?;
            let entry_id = args["time_entry_id"]
                .as_i64()
                .ok_or_else(|| anyhow!("time_entry_id is required"))?;
            helpdesk::request(
                state,
                user_id,
                Method::DELETE,
                &format!("/tickets/{id}/time-entries/{entry_id}"),
                None,
            )
            .await?;
            state
                .log("helpdesk", "info", format!("Deleted time entry #{entry_id} from ticket #{id}"))
                .await;
            Ok(json!({ "deleted": true, "time_entry_id": entry_id }))
        }
        "helpdesk_update_ticket" => {
            let id = ticket_id(&args)?;
            let mut body = serde_json::Map::new();
            if let Some(s) = args["status"].as_str().filter(|s| !s.is_empty()) {
                body.insert("status".into(), json!(s));
            }
            if let Some(p) = args["priority"].as_str().filter(|s| !s.is_empty()) {
                body.insert("priority".into(), json!(p));
            }
            if args["assign_to_me"].as_bool().unwrap_or(false) {
                let me = helpdesk::request(state, user_id, Method::GET, "/user", None).await?;
                let agent_id = me["data"]["id"]
                    .as_i64()
                    .ok_or_else(|| anyhow!("could not determine the connected agent's id"))?;
                body.insert("assigned_agent_id".into(), json!(agent_id));
            }
            if let Some(silent) = args["silent"].as_bool() {
                body.insert("silent".into(), json!(silent));
            }
            if body.is_empty() {
                return Err(anyhow!("provide status, priority, silent, and/or assign_to_me"));
            }
            let res = helpdesk::request(state, user_id, Method::PATCH, &format!("/tickets/{id}"), Some(&Value::Object(body))).await?;
            Ok(json!({ "updated": slim_detail(&res["data"]) }))
        }
        other => Err(anyhow!("unknown helpdesk tool '{other}'")),
    }
}
