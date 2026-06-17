//! Notify tool — lets the chat agent proactively flag the user's attention with
//! a push notification (mobile app + browser), also recorded in the
//! Notifications window. Built for unattended runs (e.g. a scheduled portal
//! health check) where the agent should otherwise stay silent: it calls this
//! only when there's genuinely something the user needs to see.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::integrations::push;
use crate::state::AppState;

pub fn schemas() -> Vec<Value> {
    vec![json!({
        "name": "notify_user",
        "description": "Send the user a push notification (mobile app + browser), also saved in their Notifications window, to flag their attention. Use this to proactively alert them when something needs them — e.g. a scheduled portal health check found something amiss. Be sparing: only notify when there is something wrong or actionable. In an interactive chat where the user is already reading your reply, you normally don't need this — just answer in the chat.",
        "input_schema": {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Short headline, e.g. 'Portal health: queue worker down'." },
                "body": { "type": "string", "description": "One or two sentences with the detail the user needs to act." },
                "level": {
                    "type": "string",
                    "enum": ["alert", "info"],
                    "description": "'alert' (default) for something wrong or urgent — shown with a red alert icon; 'info' for a neutral heads-up."
                }
            },
            "required": ["title", "body"]
        }
    })]
}

pub fn handles(name: &str) -> bool {
    name == "notify_user"
}

pub async fn execute(state: &AppState, user_id: &str, name: &str, args: Value) -> Result<Value> {
    match name {
        "notify_user" => {
            let title = args["title"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("title is required"))?;
            let body = args["body"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("body is required"))?;
            // Map the model's vocabulary onto the notification categories the
            // Notifications UI already styles: "alert" → the red `error` icon.
            let category = match args["level"].as_str() {
                Some("info") => "info",
                _ => "error",
            };
            push::notify_linked(state, user_id, title, body, category, None).await;
            state.log("notify", "info", format!("Flagged user: {title}")).await;
            Ok(json!({ "notified": true, "title": title }))
        }
        other => Err(anyhow!("unknown notify tool '{other}'")),
    }
}
