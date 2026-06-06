//! Document search tool — retrieval over the user's uploaded documents
//! (Documents window). Semantic over chunk embeddings, substring fallback.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::db;
use crate::integrations::embeddings;
use crate::state::AppState;

const RESULTS_DEFAULT: usize = 6;
const RESULTS_MAX: usize = 12;
/// Chunk candidate cap pulled per search.
const POOL: i64 = 20_000;
/// Below this cosine similarity a chunk is noise, not a match.
const MIN_SCORE: f32 = 0.3;

pub fn schemas() -> Vec<Value> {
    vec![json!({
        "name": "search_documents",
        "description": "Search the user's uploaded documents (PDFs, notes, references in the Documents window) by meaning and return the most relevant passages with their source filename. Use when a question likely refers to material the user has uploaded.",
        "input_schema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "What to look for, phrased as a topic or question." },
                "limit": { "type": "integer", "description": "Max passages, default 6, max 12." }
            },
            "required": ["query"]
        }
    })]
}

pub fn handles(name: &str) -> bool {
    name == "search_documents"
}

pub async fn execute(state: &AppState, user_id: &str, name: &str, args: Value) -> Result<Value> {
    if name != "search_documents" {
        return Err(anyhow!("unknown documents tool '{name}'"));
    }
    let query = args["query"]
        .as_str()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or_else(|| anyhow!("query is required"))?;
    let limit = args["limit"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(RESULTS_DEFAULT)
        .clamp(1, RESULTS_MAX);

    let chunks = db::documents::search_pool(&state.db, user_id, POOL).await?;
    if chunks.is_empty() {
        return Ok(json!({
            "passages": [],
            "note": "no documents uploaded yet — the user can add some in the Documents window"
        }));
    }

    let shape = |c: &db::documents::SearchChunk| {
        json!({ "document": c.filename, "passage": c.content, "position": c.seq })
    };

    // Semantic path.
    if let Ok(qvec) = embeddings::embed(&state.db, &state.http_client, query).await {
        let mut scored: Vec<(&db::documents::SearchChunk, f32)> = chunks
            .iter()
            .filter_map(|c| {
                let blob = c.embedding.as_deref()?;
                Some((c, embeddings::cosine(&qvec, &embeddings::from_blob(blob))))
            })
            .filter(|(_, s)| *s >= MIN_SCORE)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        if !scored.is_empty() {
            let passages: Vec<Value> = scored.iter().take(limit).map(|(c, _)| shape(c)).collect();
            return Ok(json!({ "passages": passages }));
        }
    }

    // Fallback: substring match over chunk text.
    let needle = query.to_lowercase();
    let passages: Vec<Value> = chunks
        .iter()
        .filter(|c| c.content.to_lowercase().contains(&needle))
        .take(limit)
        .map(shape)
        .collect();
    Ok(json!({ "passages": passages }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_and_handles() {
        assert_eq!(schemas()[0]["name"], "search_documents");
        assert!(handles("search_documents"));
        assert!(!handles("search_memories"));
    }
}
