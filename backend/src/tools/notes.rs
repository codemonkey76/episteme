//! Note tools — model-facing adapter over `crate::db::notes` (freeform
//! markdown notes). List returns snippets to keep model context small;
//! `get_note` fetches the full body.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::db;
use crate::state::AppState;

/// Max characters of content surfaced per note in list results.
const SNIPPET_LEN: usize = 200;

pub fn schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "list_notes",
            "description": "List the user's notes (title + a short snippet). Optionally filter by a search term. Use get_note for the full text.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "q": { "type": "string", "description": "Search term matched against title and content." }
                }
            }
        }),
        json!({
            "name": "get_note",
            "description": "Fetch a note's full content by id. Get ids from list_notes first.",
            "input_schema": {
                "type": "object",
                "properties": { "note_id": { "type": "string" } },
                "required": ["note_id"]
            }
        }),
        json!({
            "name": "create_note",
            "description": "Save a new note. Use for freeform information the user wants to keep (ideas, references, snippets). Content is markdown.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short title for the note." },
                    "content": { "type": "string", "description": "The note body (markdown)." }
                },
                "required": ["title", "content"]
            }
        }),
        json!({
            "name": "update_note",
            "description": "Change a note's title or content, or append to it. Prefer append for adding new information without rewriting.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "note_id": { "type": "string" },
                    "title": { "type": "string", "description": "New title (replaces)." },
                    "content": { "type": "string", "description": "New content (replaces the whole body)." },
                    "append": { "type": "string", "description": "Text appended to the end of the existing content as a new paragraph." }
                },
                "required": ["note_id"]
            }
        }),
        json!({
            "name": "delete_note",
            "description": "Delete a note by id. Get ids from list_notes first.",
            "input_schema": {
                "type": "object",
                "properties": { "note_id": { "type": "string" } },
                "required": ["note_id"]
            }
        }),
    ]
}

pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "list_notes" | "get_note" | "create_note" | "update_note" | "delete_note"
    )
}

fn snippet(content: &str) -> String {
    if content.chars().count() <= SNIPPET_LEN {
        content.to_string()
    } else {
        let cut: String = content.chars().take(SNIPPET_LEN).collect();
        format!("{cut}…")
    }
}

pub async fn execute(state: &AppState, name: &str, args: Value) -> Result<Value> {
    match name {
        "list_notes" => {
            let q = args["q"].as_str().filter(|s| !s.is_empty());
            let notes = db::notes::list(&state.db, q, 100).await?;
            let notes: Vec<Value> = notes
                .iter()
                .map(|n| {
                    json!({
                        "id": n.id,
                        "title": n.title,
                        "snippet": snippet(&n.content),
                        "updated_at": n.updated_at,
                    })
                })
                .collect();
            Ok(json!({ "notes": notes }))
        }
        "get_note" => {
            let id = args["note_id"]
                .as_str()
                .ok_or_else(|| anyhow!("note_id is required"))?;
            let note = db::notes::get(&state.db, id)
                .await?
                .ok_or_else(|| anyhow!("no note with id '{id}'"))?;
            Ok(serde_json::to_value(note)?)
        }
        "create_note" => {
            let title = args["title"]
                .as_str()
                .ok_or_else(|| anyhow!("title is required"))?;
            let content = args["content"]
                .as_str()
                .ok_or_else(|| anyhow!("content is required"))?;
            let note = db::notes::insert(&state.db, title, content).await?;
            Ok(json!({ "created": note }))
        }
        "update_note" => {
            let id = args["note_id"]
                .as_str()
                .ok_or_else(|| anyhow!("note_id is required"))?;
            let content = if let Some(extra) = args["append"].as_str().filter(|s| !s.is_empty()) {
                let existing = db::notes::get(&state.db, id)
                    .await?
                    .ok_or_else(|| anyhow!("no note with id '{id}'"))?;
                Some(format!("{}\n\n{extra}", existing.content.trim_end()))
            } else {
                args["content"].as_str().map(str::to_string)
            };
            let note = db::notes::update(
                &state.db,
                id,
                args["title"].as_str().map(str::to_string),
                content,
            )
            .await?
            .ok_or_else(|| anyhow!("no note with id '{id}'"))?;
            Ok(json!({ "updated": note }))
        }
        "delete_note" => {
            let id = args["note_id"]
                .as_str()
                .ok_or_else(|| anyhow!("note_id is required"))?;
            db::notes::delete(&state.db, id).await?;
            Ok(json!({ "deleted": id }))
        }
        other => Err(anyhow!("unknown note tool '{other}'")),
    }
}
