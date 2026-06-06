use axum::{extract::State, Json};
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
