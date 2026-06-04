//! Admin-only editing of the model prompts in `crate::prompts`. The registry
//! (and its defaults) lives in code; these routes manage the DB overrides.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::error::{AppError, AppResult};
use crate::prompts;
use crate::state::AppState;

// GET /api/settings/prompts
pub async fn list(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    let mut rows = Vec::with_capacity(prompts::PROMPTS.len());
    for def in prompts::PROMPTS {
        let stored = prompts::get_override(&state.db, def.key).await;
        rows.push(json!({
            "key": def.key,
            "name": def.name,
            "description": def.description,
            "variables": def.variables,
            "default": def.default,
            "content": stored.as_deref().unwrap_or(def.default),
            "customized": stored.is_some(),
        }));
    }
    Ok(Json(json!({ "prompts": rows })))
}

#[derive(Deserialize)]
pub struct SavePrompt {
    content: String,
}

// PUT /api/settings/prompts/:key — saving the default (or blank) text clears
// the override, so the prompt tracks future stock improvements again.
pub async fn save(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(body): Json<SavePrompt>,
) -> AppResult<StatusCode> {
    let def = prompts::def(&key).ok_or(AppError::NotFound)?;
    let content = body.content.trim_end();
    if content.trim().is_empty() || content == def.default {
        prompts::clear_override(&state.db, def.key).await?;
    } else {
        prompts::set_override(&state.db, def.key, content).await?;
    }
    state.log("settings", "info", format!("prompt updated: {}", def.name)).await;
    Ok(StatusCode::NO_CONTENT)
}

// DELETE /api/settings/prompts/:key — reset to the compiled-in default.
pub async fn reset(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> AppResult<StatusCode> {
    let def = prompts::def(&key).ok_or(AppError::NotFound)?;
    prompts::clear_override(&state.db, def.key).await?;
    state.log("settings", "info", format!("prompt reset to default: {}", def.name)).await;
    Ok(StatusCode::NO_CONTENT)
}
