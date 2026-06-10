//! PhoneUs tools — model-facing adapter over the user's PhoneUs accounts/comms
//! platform via its Sanctum API (see `integrations::phoneus`). Covers looking up
//! a customer, what they owe, their services/service-ids and contacts, and
//! sending an SMS to a contact. Customers are addressed by account_number;
//! resolve names via phoneus_list_customers first.

use anyhow::{anyhow, Result};
use reqwest::Method;
use serde_json::{json, Value};

use crate::integrations::phoneus;
use crate::state::AppState;

pub fn schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "phoneus_list_customers",
            "description": "Search PhoneUs customers by company name, contact name, or account number. Returns each match's account_number, display_name, and current total_outstanding. Use this first to resolve a customer before asking about their balance, services, or contacts. If multiple PhoneUs instances (portals) are configured, ask the user which one and pass its name as `integration`.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "search": { "type": "string", "description": "Company name, contact name, or account number to search for." },
                    "integration": { "type": "string", "description": "Which PhoneUs instance (portal) to use, by name. Optional when only one is configured." }
                },
                "required": ["search"]
            }
        }),
        json!({
            "name": "phoneus_customer_balance",
            "description": "How much a PhoneUs customer currently owes: total outstanding plus the open invoices making it up. Get the account_number from phoneus_list_customers first.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "account_number": { "type": "integer", "description": "The customer's PhoneUs account number." },
                    "integration": { "type": "string", "description": "Which PhoneUs instance (portal) to use, by name. Optional when only one is configured." }
                },
                "required": ["account_number"]
            }
        }),
        json!({
            "name": "phoneus_list_services",
            "description": "List a PhoneUs customer's services (plan items) with their service_id, description, and type — e.g. to find the service id of their internet connection. Get the account_number from phoneus_list_customers first.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "account_number": { "type": "integer", "description": "The customer's PhoneUs account number." },
                    "integration": { "type": "string", "description": "Which PhoneUs instance (portal) to use, by name. Optional when only one is configured." }
                },
                "required": ["account_number"]
            }
        }),
        json!({
            "name": "phoneus_list_contacts",
            "description": "List a PhoneUs customer's contacts with their phone numbers — use this to resolve a named person (e.g. \"Theresa\") to a contact_id and number before sending an SMS. Get the account_number from phoneus_list_customers first.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "account_number": { "type": "integer", "description": "The customer's PhoneUs account number." },
                    "integration": { "type": "string", "description": "Which PhoneUs instance (portal) to use, by name. Optional when only one is configured." }
                },
                "required": ["account_number"]
            }
        }),
        json!({
            "name": "phoneus_send_sms",
            "description": "Send an SMS via PhoneUs. Prefer contact_id (resolved via phoneus_list_contacts) so it goes to the right person; otherwise pass a raw destination number in `to`. The from-number is the connected user's preferred sender.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "contact_id": { "type": "integer", "description": "PhoneUs contact id (from phoneus_list_contacts) to send to." },
                    "to": { "type": "string", "description": "Raw destination number, if not sending to a known contact." },
                    "message": { "type": "string", "description": "The SMS text." },
                    "integration": { "type": "string", "description": "Which PhoneUs instance (portal) to send from, by name. Optional when only one is configured." }
                },
                "required": ["message"]
            }
        }),
    ]
}

pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "phoneus_list_customers"
            | "phoneus_customer_balance"
            | "phoneus_list_services"
            | "phoneus_list_contacts"
            | "phoneus_send_sms"
    )
}

fn account_number(args: &Value) -> Result<i64> {
    args["account_number"].as_i64().ok_or_else(|| anyhow!("account_number is required"))
}

pub async fn execute(state: &AppState, user_id: &str, name: &str, args: Value) -> Result<Value> {
    // Which PhoneUs instance to use (name); resolved against the registry.
    let instance = args["integration"].as_str();
    match name {
        "phoneus_list_customers" => {
            let search = args["search"].as_str().unwrap_or("");
            let path = format!("/customers?search={}", urlencoding::encode(search));
            let res = phoneus::request(state, user_id, instance, Method::GET, &path, None).await?;
            Ok(json!({ "customers": res["customers"] }))
        }
        "phoneus_customer_balance" => {
            let id = account_number(&args)?;
            let res = phoneus::request(state, user_id, instance, Method::GET, &format!("/customers/{id}/balance"), None).await?;
            Ok(res)
        }
        "phoneus_list_services" => {
            let id = account_number(&args)?;
            let res = phoneus::request(state, user_id, instance, Method::GET, &format!("/customers/{id}/services"), None).await?;
            Ok(res)
        }
        "phoneus_list_contacts" => {
            let id = account_number(&args)?;
            let res = phoneus::request(state, user_id, instance, Method::GET, &format!("/customers/{id}/contacts"), None).await?;
            Ok(res)
        }
        "phoneus_send_sms" => {
            let message = args["message"].as_str().ok_or_else(|| anyhow!("message is required"))?;
            let mut body = serde_json::Map::new();
            body.insert("message".into(), json!(message));
            if let Some(c) = args["contact_id"].as_i64() {
                body.insert("contact_id".into(), json!(c));
            }
            if let Some(t) = args["to"].as_str().filter(|s| !s.is_empty()) {
                body.insert("to".into(), json!(t));
            }
            if !body.contains_key("contact_id") && !body.contains_key("to") {
                return Err(anyhow!("provide a contact_id or a destination number (to)"));
            }
            let res = phoneus::request(state, user_id, instance, Method::POST, "/sms", Some(&Value::Object(body))).await?;
            state
                .log("phoneus", "info", format!(
                    "SMS queued to {}",
                    res["to"].as_str().unwrap_or("?")
                ))
                .await;
            Ok(res)
        }
        other => Err(anyhow!("unknown phoneus tool '{other}'")),
    }
}
