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
    db::pending_actions::resolve(&state.db, &action_id, true)
        .await
        .map_err(AppError::Internal)?;
    // TODO: signal the paused agent turn to resume
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reject(
    State(state): State<Arc<AppState>>,
    Path(action_id): Path<String>,
) -> AppResult<StatusCode> {
    db::pending_actions::resolve(&state.db, &action_id, false)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
