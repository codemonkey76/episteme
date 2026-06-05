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
            "description": "List the user's to-do tasks. Use this to answer questions about what they need to do. Tasks live in named lists; omit `list` to see everything.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["open", "done", "all"], "description": "Filter by status. Default open." },
                    "list": { "type": "string", "description": "Optional list name to filter to (e.g. \"General\" or a project list)." }
                }
            }
        }),
        json!({
            "name": "list_task_lists",
            "description": "List the user's to-do lists (including the built-in \"General\" list).",
            "input_schema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "create_task",
            "description": "Add a task to the user's to-do list. Use for to-dos without a fixed appointment time; use create_calendar_event for scheduled events. Tasks go to the \"General\" list unless another list is named (created automatically if new).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short description of the task." },
                    "notes": { "type": "string", "description": "Optional details." },
                    "due": { "type": "string", "description": "Optional due time, RFC3339 with the user's UTC offset from the system message." },
                    "priority": { "type": "string", "enum": ["low", "normal", "high"], "description": "Default normal." },
                    "list": { "type": "string", "description": "Optional list name. Created if it doesn't exist." }
                },
                "required": ["title"]
            }
        }),
        json!({
            "name": "update_task",
            "description": "Change a task's title, notes, due time, priority, status, or list. Get ids from list_tasks first.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "title": { "type": "string" },
                    "notes": { "type": "string" },
                    "due": { "type": "string", "description": "RFC3339 with the user's UTC offset, or empty string to clear." },
                    "priority": { "type": "string", "enum": ["low", "normal", "high"] },
                    "status": { "type": "string", "enum": ["open", "done"] },
                    "list": { "type": "string", "description": "Move to this list (\"General\" for the default; created if it doesn't exist)." }
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
        "list_tasks" | "list_task_lists" | "create_task" | "update_task" | "complete_task" | "delete_task"
    )
}

/// Resolve a model-supplied list name to a `list_id`: None for the implicit
/// General list, otherwise an existing list (case-insensitive) or a freshly
/// created one.
async fn resolve_list(state: &AppState, user_id: &str, name: &str) -> Result<Option<String>> {
    let name = name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("general") {
        return Ok(None);
    }
    if let Some(list) = db::tasks::list_by_name(&state.db, user_id, name).await? {
        return Ok(Some(list.id));
    }
    Ok(Some(db::tasks::insert_list(&state.db, user_id, name).await?.id))
}

/// Parse a model-supplied due time (RFC3339 with offset) into UTC for storage.
fn due_to_utc(s: &str) -> Result<String> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc).to_rfc3339())
        .map_err(|_| anyhow!("invalid due time '{s}' — expected RFC3339 e.g. 2026-06-05T15:00:00+10:00"))
}

/// Serialize a task for the model: due_at localized + human display string,
/// and the raw list_id swapped for a readable list name.
fn localize_task(
    task: &db::tasks::Task,
    tz: chrono_tz::Tz,
    list_names: &std::collections::HashMap<String, String>,
) -> Value {
    let mut v = serde_json::to_value(task).unwrap_or_default();
    if let Some(due) = task.due_at.as_deref() {
        let (rfc, display) = localize(due, tz);
        v["due_at"] = Value::String(rfc);
        if let Some(d) = display {
            v["due_display"] = Value::String(d);
        }
    }
    let list_name = task
        .list_id
        .as_deref()
        .and_then(|id| list_names.get(id).cloned())
        .unwrap_or_else(|| "General".to_string());
    if let Some(obj) = v.as_object_mut() {
        obj.remove("list_id");
        obj.insert("list".to_string(), Value::String(list_name));
    }
    v
}

/// id → name map of the user's named lists.
async fn list_name_map(state: &AppState, user_id: &str) -> Result<std::collections::HashMap<String, String>> {
    Ok(db::tasks::lists(&state.db, user_id)
        .await?
        .into_iter()
        .map(|l| (l.id, l.name))
        .collect())
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
            let names = list_name_map(state, user_id).await?;
            // Resolve a list-name filter; an unknown name just means no tasks.
            let filter = match args["list"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                None => db::tasks::ListFilter::All,
                Some(n) if n.eq_ignore_ascii_case("general") => db::tasks::ListFilter::General,
                Some(n) => match names.iter().find(|(_, name)| name.eq_ignore_ascii_case(n)) {
                    Some((id, _)) => db::tasks::ListFilter::List(id.as_str()),
                    None => return Ok(json!({ "tasks": [], "note": format!("no list named '{n}'") })),
                },
            };
            let tasks = db::tasks::list(&state.db, user_id, status, None, filter, 200).await?;
            let tasks: Vec<Value> = tasks.iter().map(|t| localize_task(t, tz, &names)).collect();
            Ok(json!({ "tasks": tasks }))
        }
        "list_task_lists" => {
            let mut all = vec!["General".to_string()];
            all.extend(db::tasks::lists(&state.db, user_id).await?.into_iter().map(|l| l.name));
            Ok(json!({ "lists": all }))
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
            let list_id = match args["list"].as_str() {
                Some(n) => resolve_list(state, user_id, n).await?,
                None => None,
            };
            let task = db::tasks::insert(
                &state.db,
                user_id,
                title,
                args["notes"].as_str(),
                due_at.as_deref(),
                priority,
                list_id.as_deref(),
            )
            .await?;
            let names = list_name_map(state, user_id).await?;
            Ok(json!({ "created": localize_task(&task, tz, &names) }))
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
                    list_id: match args["list"].as_str() {
                        Some(n) => Some(resolve_list(state, user_id, n).await?),
                        None => None,
                    },
                }
            };
            let task = db::tasks::update(&state.db, user_id, id, patch)
                .await?
                .ok_or_else(|| anyhow!("no task with id '{id}'"))?;
            let names = list_name_map(state, user_id).await?;
            Ok(json!({ "updated": localize_task(&task, tz, &names) }))
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
