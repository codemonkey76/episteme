//! Deep-research tool — hands a topic to the research orchestrator, which
//! runs as a background job and produces a self-contained HTML report in the
//! Reports window.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::state::AppState;

pub fn schemas() -> Vec<Value> {
    vec![json!({
        "name": "deep_research",
        "description": "Start an in-depth research investigation into a topic: plans search queries, reads web sources AND the user's own documents/email/memories/chat history, and produces a polished report with citations, comparison tables, charts, and images in the Reports window. Runs in the background — the user is notified when the report is ready. Use for substantive questions deserving real research, not quick lookups (use web_search for those).",
        "input_schema": {
            "type": "object",
            "properties": {
                "topic": { "type": "string", "description": "The research question or topic, stated completely — the researcher cannot see this conversation." },
                "depth": { "type": "string", "enum": ["quick", "standard", "deep"], "description": "quick ≈ a few sources, standard (default) ≈ a dozen, deep ≈ twenty with extra follow-up rounds." },
                "provider": { "type": "string", "description": "Optional provider name; defaults to the first configured." }
            },
            "required": ["topic"]
        }
    })]
}

pub fn handles(name: &str) -> bool {
    name == "deep_research"
}

pub async fn execute(state: &Arc<AppState>, user_id: &str, name: &str, args: Value) -> Result<Value> {
    if name != "deep_research" {
        return Err(anyhow!("unknown research tool '{name}'"));
    }
    let topic = args["topic"].as_str().unwrap_or("");
    let depth = args["depth"].as_str().unwrap_or("standard");
    let provider_arg = args["provider"].as_str().unwrap_or("").trim();

    let (job_id, session_id) =
        crate::research::launch(state, user_id, topic, depth, provider_arg).await?;

    Ok(json!({
        "started": true,
        "job_id": job_id,
        "session_id": session_id,
        "note": "Deep research started in the background — the user will be notified when the report appears in their Reports window.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_and_handles() {
        let s = schemas();
        assert_eq!(s[0]["name"], "deep_research");
        assert_eq!(
            s[0]["input_schema"]["required"].as_array().unwrap().len(),
            1
        );
        assert!(handles("deep_research"));
        assert!(!handles("research"));
    }
}
