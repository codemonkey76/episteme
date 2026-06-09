//! Memory consolidation — the "dreaming" pass.
//!
//! Like sleep consolidating the day's experiences, this reviews a user's stored
//! memories and: merges redundant ones, resolves conflicts, and synthesises
//! higher-order LESSONS that generalise beyond any single memory. Runs nightly
//! (scheduler) or on demand. Every change is applied via SOFT delete, so a
//! consolidation is fully reversible from the Memories archive.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::db;
use crate::db::memories::Memory;
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig};
use crate::state::AppState;

/// Per-group cap on memories sent to the model in one call, so a large store
/// can't blow the context window. Consolidation converges over several runs.
const MAX_PER_GROUP: usize = 100;
/// Memory pool pulled per run.
const POOL: i64 = 500;
/// Categories whose memories feed the lesson-synthesis step.
const LESSON_SOURCES: [&str; 3] = ["feedback", "preference", "project"];

/// What a consolidation run changed — surfaced to the manual trigger and logs.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Summary {
    /// Memories merged away into consolidated ones.
    pub merged: usize,
    /// Memories dropped as redundant/conflicting.
    pub dropped: usize,
    /// New lessons synthesised.
    pub lessons: usize,
    /// Category groups the model reviewed.
    pub groups: usize,
}

#[derive(Debug, Default, Deserialize)]
struct Ops {
    #[serde(default)]
    merges: Vec<Merge>,
    #[serde(default)]
    drops: Vec<DropOp>,
}

#[derive(Debug, Deserialize)]
struct Merge {
    #[serde(default)]
    ids: Vec<usize>,
    content: String,
    #[serde(default)]
    category: String,
}

#[derive(Debug, Deserialize)]
struct DropOp {
    id: usize,
    #[serde(default)]
    reason: String,
}

/// Pick the model for consolidation: an explicit `requested` provider name, else
/// the saved default (`memory_consolidation_provider`), else the first configured
/// provider. None when no provider is configured at all. Memories deserve the
/// smartest model available — point this at Sonnet/Opus rather than a local model.
pub async fn resolve_provider(state: &AppState, requested: Option<&str>) -> Option<ProviderConfig> {
    let providers: Vec<ProviderConfig> =
        db::settings::get(&state.db, "providers").await.ok().flatten().unwrap_or_default();
    if providers.is_empty() {
        return None;
    }
    let saved: Option<String> =
        db::settings::get(&state.db, "memory_consolidation_provider").await.ok().flatten();
    let name = requested
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or(saved);
    let chosen = name.and_then(|n| providers.iter().find(|p| p.name == n).cloned());
    Some(chosen.unwrap_or_else(|| providers[0].clone()))
}

/// Run a full consolidation pass for one user with the given model. Best-effort
/// per group/step: a failure in one category is logged and skipped, never
/// aborting the whole run.
pub async fn run(state: &AppState, user_id: &str, provider: &ProviderConfig) -> Result<Summary> {
    let mut summary = Summary::default();

    let memories = db::memories::list_recent(&state.db, user_id, POOL).await?;
    if memories.len() < 2 {
        return Ok(summary);
    }

    // Merge/drop within each category — most redundancy and conflict is
    // same-category, and it keeps each prompt focused.
    let mut by_cat: BTreeMap<&str, Vec<&Memory>> = BTreeMap::new();
    for m in &memories {
        by_cat.entry(m.category.as_str()).or_default().push(m);
    }

    for (category, group) in &by_cat {
        if group.len() < 2 {
            continue;
        }
        let slice = &group[..group.len().min(MAX_PER_GROUP)];
        let ops = match consolidate_group(state, user_id, provider, category, slice).await {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("consolidation of '{category}' failed: {e}");
                continue;
            }
        };
        summary.groups += 1;

        for merge in ops.merges {
            // Map the model's 1-based numbers back to memories; a merge needs ≥2.
            let targets: Vec<&Memory> =
                merge.ids.iter().filter_map(|&i| i.checked_sub(1).and_then(|i| slice.get(i)).copied()).collect();
            let content = merge.content.trim();
            if targets.len() < 2 || content.is_empty() {
                continue;
            }
            let cat = super::normalize_category(if merge.category.is_empty() { category } else { &merge.category });
            match db::memories::insert(&state.db, user_id, content, &cat, "consolidated", None).await {
                Ok(new) => {
                    super::embed_detached(state, new.id.clone(), new.content.clone());
                    for t in &targets {
                        let _ = db::memories::soft_delete(&state.db, user_id, &t.id, Some(&new.id)).await;
                        summary.merged += 1;
                    }
                    super::log_event(
                        state,
                        format!("Consolidated {} memories → {content}", targets.len()),
                    );
                }
                Err(e) => tracing::warn!("failed to save consolidated memory: {e}"),
            }
        }

        for drop in ops.drops {
            let Some(m) = drop.id.checked_sub(1).and_then(|i| slice.get(i)).copied() else {
                continue;
            };
            let _ = db::memories::soft_delete(&state.db, user_id, &m.id, None).await;
            summary.dropped += 1;
            super::log_event(state, format!("Dropped memory ({}): {}", drop.reason, m.content));
        }
    }

    // Reflection: synthesise lessons from the (now-consolidated) experiential
    // memories. Reload so merged-away rows aren't reconsidered.
    let after = db::memories::list_recent(&state.db, user_id, POOL).await?;
    let sources: Vec<&Memory> = after
        .iter()
        .filter(|m| LESSON_SOURCES.contains(&m.category.as_str()))
        .take(MAX_PER_GROUP)
        .collect();
    if sources.len() >= 3 {
        match synth_lessons(state, user_id, provider, &sources).await {
            Ok(lessons) => {
                let existing: Vec<Memory> =
                    after.iter().filter(|m| m.category == "lesson").cloned().collect();
                for content in lessons {
                    let content = content.trim();
                    if content.is_empty() || super::is_duplicate(content, &existing) {
                        continue;
                    }
                    match db::memories::insert(&state.db, user_id, content, "lesson", "lesson", None).await {
                        Ok(new) => {
                            super::embed_detached(state, new.id.clone(), new.content.clone());
                            summary.lessons += 1;
                            super::log_event(state, format!("Learned lesson: {content}"));
                        }
                        Err(e) => tracing::warn!("failed to save lesson: {e}"),
                    }
                }
            }
            Err(e) => tracing::warn!("lesson synthesis failed: {e}"),
        }
    }

    super::log_event(
        state,
        format!(
            "Dreaming complete — merged {}, dropped {}, lessons {} (across {} categories)",
            summary.merged, summary.dropped, summary.lessons, summary.groups
        ),
    );
    Ok(summary)
}

/// Ask the model to merge/drop redundant & conflicting memories within one group.
async fn consolidate_group(
    state: &AppState,
    user_id: &str,
    provider: &ProviderConfig,
    category: &str,
    group: &[&Memory],
) -> Result<Ops> {
    let system = crate::prompts::get(&state.db, "memory_consolidate").await;
    let mut user = format!("Category: {category}\nMemories:\n");
    for (i, m) in group.iter().enumerate() {
        user.push_str(&format!("{}. {}\n", i + 1, m.content));
    }
    let raw = complete(state, user_id, provider, system, user).await?;
    parse_ops(&raw)
}

/// Ask the model for generalised lessons drawn from experiential memories.
async fn synth_lessons(
    state: &AppState,
    user_id: &str,
    provider: &ProviderConfig,
    sources: &[&Memory],
) -> Result<Vec<String>> {
    let system = crate::prompts::get(&state.db, "memory_lessons").await;
    let mut user = String::from("Memories (feedback, preferences, projects):\n");
    for (i, m) in sources.iter().enumerate() {
        user.push_str(&format!("{}. [{}] {}\n", i + 1, m.category, m.content));
    }
    let raw = complete(state, user_id, provider, system, user).await?;
    Ok(super::parse(&raw)?.into_iter().map(|e| e.content).collect())
}

/// One model round-trip, recording token usage under the "consolidate" feature.
async fn complete(
    state: &AppState,
    user_id: &str,
    provider: &ProviderConfig,
    system: String,
    user: String,
) -> Result<String> {
    let history = vec![
        ChatMessage { role: "system".to_string(), content: serde_json::Value::String(system) },
        ChatMessage { role: "user".to_string(), content: serde_json::Value::String(user) },
    ];
    let (raw, used) = ModelRouter::complete_with_usage(provider, history).await?;
    db::usage::record(&state.db, user_id, provider, "consolidate", used).await;
    Ok(raw)
}

/// Extract the `{...}` ops object, tolerating code fences / surrounding prose.
fn parse_ops(raw: &str) -> Result<Ops> {
    let start = raw.find('{').ok_or_else(|| anyhow::anyhow!("no JSON object in model output"))?;
    let end = raw.rfind('}').ok_or_else(|| anyhow::anyhow!("no JSON object in model output"))?;
    if end < start {
        anyhow::bail!("malformed JSON object");
    }
    Ok(serde_json::from_str(&raw[start..=end])?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ops_tolerates_fences_and_prose() {
        let raw = "Here:\n```json\n{\"merges\": [{\"ids\": [1,2], \"content\": \"x\", \"category\": \"fact\"}], \"drops\": [{\"id\": 3, \"reason\": \"old\"}]}\n```";
        let ops = parse_ops(raw).unwrap();
        assert_eq!(ops.merges.len(), 1);
        assert_eq!(ops.merges[0].ids, vec![1, 2]);
        assert_eq!(ops.drops.len(), 1);
        assert_eq!(ops.drops[0].id, 3);
    }

    #[test]
    fn parse_ops_empty() {
        let ops = parse_ops("{\"merges\": [], \"drops\": []}").unwrap();
        assert!(ops.merges.is_empty() && ops.drops.is_empty());
    }
}
