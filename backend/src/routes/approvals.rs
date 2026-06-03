use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub async fn list_pending(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let actions = db::pending_actions::list_pending(&state.db, &session_id)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "pending_actions": actions })))
}

pub async fn approve(
    State(state): State<Arc<AppState>>,
    Path(action_id): Path<String>,
) -> AppResult<StatusCode> {
    decide(&state, &action_id, true).await
}

pub async fn reject(
    State(state): State<Arc<AppState>>,
    Path(action_id): Path<String>,
) -> AppResult<StatusCode> {
    decide(&state, &action_id, false).await
}

async fn decide(state: &Arc<AppState>, action_id: &str, approved: bool) -> AppResult<StatusCode> {
    db::pending_actions::resolve(&state.db, action_id, approved)
        .await
        .map_err(AppError::Internal)?;
    // Wake the paused agent turn, if it's still in flight. An absent sender
    // means the wait already ended (timeout/disconnect/restart) — DB update only.
    if let Some(tx) = state.pending_approvals.lock().await.remove(action_id) {
        let _ = tx.send(approved);
    }
    Ok(StatusCode::NO_CONTENT)
}
