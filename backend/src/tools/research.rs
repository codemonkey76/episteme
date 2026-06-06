//! Deep-research tool — hands a topic to the research orchestrator, which
//! runs as a background job and produces a self-contained HTML report in the
//! Reports window.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::db;
use crate::model_router::ProviderConfig;
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
    let topic = args["topic"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("topic is required"))?;
    let depth = match args["depth"].as_str().unwrap_or("standard") {
        d @ ("quick" | "deep") => d,
        _ => "standard",
    };
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

    let session = db::sessions::create(&state.db, user_id, &format!("🔎 {topic}")).await?;
    db::messages::insert(
        &state.db,
        &session.id,
        "user",
        &serde_json::to_string(topic).unwrap_or_default(),
        None,
        None,
    )
    .await?;

    let name_clipped: String = topic.chars().take(60).collect();
    let meta = json!({ "topic": topic, "depth": depth }).to_string();
    let job = crate::jobs::start(
        state,
        user_id,
        &session.id,
        provider_arg,
        "research",
        &format!("Research: {name_clipped}"),
        Some(&meta),
    )
    .await?;
    state.job_tx.send(job.clone()).map_err(|_| anyhow!("job queue unavailable"))?;

    Ok(json!({
        "started": true,
        "job_id": job.id,
        "session_id": session.id,
        "depth": depth,
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
