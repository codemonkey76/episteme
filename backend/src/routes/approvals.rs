use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;

use axum::Extension;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

pub async fn list_pending(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(session_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    db::sessions::get(&state.db, &user.id, &session_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    let actions = db::pending_actions::list_pending(&state.db, &session_id)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "pending_actions": actions })))
}

pub async fn approve(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(action_id): Path<String>,
) -> AppResult<StatusCode> {
    decide(&state, &user.id, &action_id, true).await
}

pub async fn reject(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(action_id): Path<String>,
) -> AppResult<StatusCode> {
    decide(&state, &user.id, &action_id, false).await
}

async fn decide(
    state: &Arc<AppState>,
    user_id: &str,
    action_id: &str,
    approved: bool,
) -> AppResult<StatusCode> {
    // Only the owner of the session may decide its tool calls.
    if !db::pending_actions::owned_by(&state.db, action_id, user_id)
        .await
        .map_err(AppError::Internal)?
    {
        return Err(AppError::NotFound);
    }
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
