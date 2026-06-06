//! Memory search tool — lets the model pull stored memories on demand instead
//! of relying only on what was injected into the system prompt. Semantic
//! (cosine over Ollama embeddings) with a substring fallback when embeddings
//! are unavailable.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::db;
use crate::integrations::embeddings;
use crate::state::AppState;

const RESULTS_DEFAULT: usize = 10;
const RESULTS_MAX: usize = 25;
/// Candidate pool scored per search.
const POOL: i64 = 2000;
/// Below this cosine similarity a hit is noise, not a match.
const MIN_SCORE: f32 = 0.3;

pub fn schemas() -> Vec<Value> {
    vec![json!({
        "name": "search_memories",
        "description": "Search the user's stored long-term memories (facts, preferences, feedback, project context, writing style) by meaning. Use when you need context that isn't already in this conversation — e.g. \"what do I know about the user's home network?\".",
        "input_schema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "What to look for, phrased as a topic or question." },
                "category": { "type": "string", "enum": ["preference", "fact", "feedback", "project", "style", "other"], "description": "Optional category filter." },
                "limit": { "type": "integer", "description": "Max results, default 10, max 25." }
            },
            "required": ["query"]
        }
    })]
}

pub fn handles(name: &str) -> bool {
    name == "search_memories"
}

fn shape(m: &db::memories::Memory) -> Value {
    json!({
        "id": m.id,
        "category": m.category,
        "content": m.content,
        "saved": m.created_at,
    })
}

pub async fn execute(state: &AppState, user_id: &str, name: &str, args: Value) -> Result<Value> {
    if name != "search_memories" {
        return Err(anyhow!("unknown memory tool '{name}'"));
    }
    let query = args["query"]
        .as_str()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or_else(|| anyhow!("query is required"))?;
    let category = args["category"].as_str().filter(|c| !c.is_empty());
    let limit = args["limit"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(RESULTS_DEFAULT)
        .clamp(1, RESULTS_MAX);

    let memories = db::memories::list(&state.db, user_id, category, None, POOL).await?;
    if memories.is_empty() {
        return Ok(json!({ "memories": [], "note": "no stored memories match" }));
    }

    // Semantic path: embed the query, rank by cosine.
    if let Ok(qvec) = embeddings::embed(&state.db, &state.http_client, query).await {
        let mut scored: Vec<(&db::memories::Memory, f32)> = memories
            .iter()
            .filter_map(|m| {
                let blob = m.embedding.as_deref()?;
                Some((m, embeddings::cosine(&qvec, &embeddings::from_blob(blob))))
            })
            .filter(|(_, score)| *score >= MIN_SCORE)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        if !scored.is_empty() {
            let hits: Vec<Value> = scored.iter().take(limit).map(|(m, _)| shape(m)).collect();
            return Ok(json!({ "memories": hits }));
        }
    }

    // Fallback: substring match (embeddings unavailable or nothing scored).
    let needle = query.to_lowercase();
    let hits: Vec<Value> = memories
        .iter()
        .filter(|m| m.content.to_lowercase().contains(&needle))
        .take(limit)
        .map(shape)
        .collect();
    Ok(json!({ "memories": hits }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_and_handles() {
        let schemas = schemas();
        assert_eq!(schemas[0]["name"], "search_memories");
        assert!(handles("search_memories"));
        assert!(!handles("list_memories"));
    }

    #[test]
    fn shape_omits_embedding() {
        let m = db::memories::Memory {
            id: "m1".into(),
            content: "Prefers Brisbane time".into(),
            category: "preference".into(),
            source: "auto".into(),
            session_id: None,
            created_at: "2026-06-06T00:00:00Z".into(),
            updated_at: "2026-06-06T00:00:00Z".into(),
            embedding: Some(vec![0u8; 8]),
        };
        let v = shape(&m);
        assert_eq!(v["content"], "Prefers Brisbane time");
        assert!(v.get("embedding").is_none());
    }
}
