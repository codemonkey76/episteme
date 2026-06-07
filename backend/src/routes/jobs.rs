use axum::{
    extract::{Path, State},
    Json,
};
use std::sync::Arc;

use axum::Extension;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

// GET /api/jobs — the user's recent background/scheduled runs, newest first.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<serde_json::Value>> {
    let jobs = db::jobs::list_for_user(&state.db, &user.id, 50).await.map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "jobs": jobs })))
}

// POST /api/jobs/:id/cancel — fail an in-flight job (e.g. one wedged on an
// unresponsive provider). The worker task isn't killed, but it's bounded by
// the model-call timeout and harmless; this clears the row from the UI and
// drops any pending approvals it parked.
pub async fn cancel(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let job = db::jobs::get(&state.db, &id)
        .await
        .map_err(AppError::Internal)?
        .filter(|j| j.user_id == user.id)
        .ok_or(AppError::NotFound)?;
    let cancelled = db::jobs::cancel(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    if cancelled {
        // Clear anything it parked so the approval queue doesn't keep showing it.
        let _ = db::pending_actions::clear_for_session(&state.db, &job.session_id).await;
        state.log("jobs", "info", format!("'{}' cancelled by user", job.name)).await;
    }
    Ok(Json(serde_json::json!({ "cancelled": cancelled })))
}
