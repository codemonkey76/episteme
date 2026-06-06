//! Conversation-history search tool — full-text (FTS5) over the user's past
//! chat sessions, so the model can recall earlier discussions on demand.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::db;
use crate::state::AppState;

const RESULTS_DEFAULT: i64 = 10;
const RESULTS_MAX: i64 = 25;

pub fn schemas() -> Vec<Value> {
    vec![json!({
        "name": "search_history",
        "description": "Full-text search across the user's past chat conversations. Use when they refer to something discussed before (\"that chat about the router config\", \"what did we decide last week?\").",
        "input_schema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Keywords to search for." },
                "limit": { "type": "integer", "description": "Max results, default 10, max 25." }
            },
            "required": ["query"]
        }
    })]
}

pub fn handles(name: &str) -> bool {
    name == "search_history"
}

pub async fn execute(state: &AppState, user_id: &str, name: &str, args: Value) -> Result<Value> {
    if name != "search_history" {
        return Err(anyhow!("unknown history tool '{name}'"));
    }
    let query = args["query"]
        .as_str()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or_else(|| anyhow!("query is required"))?;
    let limit = args["limit"].as_i64().unwrap_or(RESULTS_DEFAULT).clamp(1, RESULTS_MAX);

    let tz = state.home_tz(user_id).await;
    let hits = db::messages::search(&state.db, user_id, query, limit).await?;
    let results: Vec<Value> = hits
        .iter()
        .map(|h| {
            let (_, when) = super::localize(&h.created_at, tz);
            json!({
                "conversation": h.session_title,
                "speaker": h.role,
                "snippet": h.snippet,
                "when": when.unwrap_or_else(|| h.created_at.clone()),
            })
        })
        .collect();
    Ok(json!({ "matches": results }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_and_handles() {
        assert_eq!(schemas()[0]["name"], "search_history");
        assert!(handles("search_history"));
        assert!(!handles("search_memories"));
    }
}
