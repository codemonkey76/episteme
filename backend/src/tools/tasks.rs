//! Task tools — model-facing adapter over `crate::db::tasks` (the user's
//! to-do list). Due times arrive as RFC3339 with the user's offset (per the
//! system preamble), are stored UTC, and are localized in every result.

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::{json, Value};

use crate::db::{self, tasks::TaskPatch};
use crate::state::AppState;

use super::localize;

pub fn schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "list_tasks",
            "description": "List the user's to-do tasks. Use this to answer questions about what they need to do.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["open", "done", "all"], "description": "Filter by status. Default open." }
                }
            }
        }),
        json!({
            "name": "create_task",
            "description": "Add a task to the user's to-do list. Use for to-dos without a fixed appointment time; use create_calendar_event for scheduled events.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short description of the task." },
                    "notes": { "type": "string", "description": "Optional details." },
                    "due": { "type": "string", "description": "Optional due time, RFC3339 with the user's UTC offset from the system message." },
                    "priority": { "type": "string", "enum": ["low", "normal", "high"], "description": "Default normal." }
                },
                "required": ["title"]
            }
        }),
        json!({
            "name": "update_task",
            "description": "Change a task's title, notes, due time, priority, or status. Get ids from list_tasks first.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "title": { "type": "string" },
                    "notes": { "type": "string" },
                    "due": { "type": "string", "description": "RFC3339 with the user's UTC offset, or empty string to clear." },
                    "priority": { "type": "string", "enum": ["low", "normal", "high"] },
                    "status": { "type": "string", "enum": ["open", "done"] }
                },
                "required": ["task_id"]
            }
        }),
        json!({
            "name": "complete_task",
            "description": "Mark a task as done. Get ids from list_tasks first.",
            "input_schema": {
                "type": "object",
                "properties": { "task_id": { "type": "string" } },
                "required": ["task_id"]
            }
        }),
        json!({
            "name": "delete_task",
            "description": "Delete a task entirely (prefer complete_task for finished work). Get ids from list_tasks first.",
            "input_schema": {
                "type": "object",
                "properties": { "task_id": { "type": "string" } },
                "required": ["task_id"]
            }
        }),
    ]
}

pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "list_tasks" | "create_task" | "update_task" | "complete_task" | "delete_task"
    )
}

/// Parse a model-supplied due time (RFC3339 with offset) into UTC for storage.
fn due_to_utc(s: &str) -> Result<String> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc).to_rfc3339())
        .map_err(|_| anyhow!("invalid due time '{s}' — expected RFC3339 e.g. 2026-06-05T15:00:00+10:00"))
}

/// Serialize a task for the model: due_at localized + human display string.
fn localize_task(task: &db::tasks::Task, tz: chrono_tz::Tz) -> Value {
    let mut v = serde_json::to_value(task).unwrap_or_default();
    if let Some(due) = task.due_at.as_deref() {
        let (rfc, display) = localize(due, tz);
        v["due_at"] = Value::String(rfc);
        if let Some(d) = display {
            v["due_display"] = Value::String(d);
        }
    }
    v
}

pub async fn execute(state: &AppState, user_id: &str, name: &str, args: Value) -> Result<Value> {
    let tz = state.home_tz(user_id).await;
    match name {
        "list_tasks" => {
            let status = match args["status"].as_str() {
                None => Some("open"),
                Some("all") => None,
                Some(s) => Some(s),
            };
            let tasks = db::tasks::list(&state.db, user_id, status, None, 200).await?;
            let tasks: Vec<Value> = tasks.iter().map(|t| localize_task(t, tz)).collect();
            Ok(json!({ "tasks": tasks }))
        }
        "create_task" => {
            let title = args["title"]
                .as_str()
                .ok_or_else(|| anyhow!("title is required"))?;
            let due_at = match args["due"].as_str().filter(|s| !s.is_empty()) {
                Some(s) => Some(due_to_utc(s)?),
                None => None,
            };
            let priority = match args["priority"].as_str() {
                Some(p @ ("low" | "normal" | "high")) => p,
                _ => "normal",
            };
            let task = db::tasks::insert(
                &state.db,
                user_id,
                title,
                args["notes"].as_str(),
                due_at.as_deref(),
                priority,
            )
            .await?;
            Ok(json!({ "created": localize_task(&task, tz) }))
        }
        "update_task" | "complete_task" => {
            let id = args["task_id"]
                .as_str()
                .ok_or_else(|| anyhow!("task_id is required"))?;
            let patch = if name == "complete_task" {
                TaskPatch { status: Some("done".to_string()), ..Default::default() }
            } else {
                TaskPatch {
                    title: args["title"].as_str().map(str::to_string),
                    notes: args["notes"].as_str().map(|n| Some(n.to_string())),
                    due_at: match args["due"].as_str() {
                        Some("") => Some(None), // explicit clear
                        Some(s) => Some(Some(due_to_utc(s)?)),
                        None => None,
                    },
                    priority: args["priority"]
                        .as_str()
                        .filter(|p| ["low", "normal", "high"].contains(p))
                        .map(str::to_string),
                    status: args["status"]
                        .as_str()
                        .filter(|s| ["open", "done"].contains(s))
                        .map(str::to_string),
                }
            };
            let task = db::tasks::update(&state.db, user_id, id, patch)
                .await?
                .ok_or_else(|| anyhow!("no task with id '{id}'"))?;
            Ok(json!({ "updated": localize_task(&task, tz) }))
        }
        "delete_task" => {
            let id = args["task_id"]
                .as_str()
                .ok_or_else(|| anyhow!("task_id is required"))?;
            db::tasks::delete(&state.db, user_id, id).await?;
            Ok(json!({ "deleted": id }))
        }
        other => Err(anyhow!("unknown task tool '{other}'")),
    }
}
