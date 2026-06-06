//! Background-task tool — lets the chat agent hand work off to an unattended
//! run in its own session, returning immediately. The launched job respects
//! per-tool approval policies: gated tools park in the approval queue and the
//! job suspends until the user decides.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::db;
use crate::model_router::ProviderConfig;
use crate::state::AppState;

pub fn schemas() -> Vec<Value> {
    vec![json!({
        "name": "start_background_task",
        "description": "Run a multi-step task unattended in the background, in its own session, and return immediately. Use when the user asks for something long-running ('do this in the background', 'go through my Processed folder and …'). The user is notified when it finishes; any tool that requires approval pauses the task and appears in their approval queue — it is never auto-approved.",
        "input_schema": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Short label for the task (shown in Jobs and notifications)." },
                "instructions": { "type": "string", "description": "Complete, self-contained instructions for the background agent — it cannot see this conversation." },
                "provider": { "type": "string", "description": "Optional provider name; defaults to the first configured." }
            },
            "required": ["name", "instructions"]
        }
    })]
}

pub fn handles(name: &str) -> bool {
    name == "start_background_task"
}

pub async fn execute(state: &Arc<AppState>, user_id: &str, name: &str, args: Value) -> Result<Value> {
    if name != "start_background_task" {
        return Err(anyhow!("unknown jobs tool '{name}'"));
    }
    let task_name = args["name"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("name is required"))?;
    let instructions = args["instructions"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("instructions are required"))?;
    let provider_arg = args["provider"].as_str().unwrap_or("").trim();

    // Validate the provider now so a typo fails the tool call, not the job.
    let providers: Vec<ProviderConfig> =
        db::settings::get(&state.db, "providers").await?.unwrap_or_default();
    if providers.is_empty() {
        return Err(anyhow!("no model providers configured"));
    }
    if !provider_arg.is_empty() && !providers.iter().any(|p| p.name == provider_arg) {
        return Err(anyhow!("provider '{provider_arg}' not found"));
    }

    let session = db::sessions::create(&state.db, user_id, &format!("⚙ {task_name}")).await?;
    db::messages::insert(
        &state.db,
        &session.id,
        "user",
        &serde_json::to_string(instructions).unwrap_or_default(),
        None,
        None,
    )
    .await?;

    let job =
        crate::jobs::start(state, user_id, &session.id, provider_arg, "background", task_name, None)
            .await?;
    // Enqueue rather than spawn: the queue worker (spawned in main) runs it,
    // which keeps this tool's future out of the agent loop's Send cycle.
    state
        .job_tx
        .send(job.clone())
        .map_err(|_| anyhow!("job queue unavailable"))?;

    Ok(json!({
        "started": true,
        "job_id": job.id,
        "session_id": session.id,
        "note": "Background task started — the user will be notified when it finishes, and any tool needing approval will appear in their approval queue.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_and_handles() {
        let s = schemas();
        assert_eq!(s[0]["name"], "start_background_task");
        let required: Vec<&str> = s[0]["input_schema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(required, ["name", "instructions"]);
        assert!(handles("start_background_task"));
        assert!(!handles("stop_background_task"));
    }
}
