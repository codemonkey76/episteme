use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ListQuery {
    limit: Option<i64>,
}

// GET /api/notifications?limit= — most recent first, with the unread count.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let items = db::notifications::list(&state.db, &user.id, params.limit.unwrap_or(100))
        .await
        .map_err(AppError::Internal)?;
    let unread = db::notifications::unread_count(&state.db, &user.id)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "notifications": items, "unread": unread })))
}

// POST /api/notifications/:id/read — mark one read.
pub async fn mark_read(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::notifications::mark_read(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// POST /api/notifications/read-all — mark every notification read.
pub async fn mark_all_read(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<StatusCode> {
    db::notifications::mark_all_read(&state.db, &user.id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// DELETE /api/notifications/:id — remove one.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::notifications::delete(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// DELETE /api/notifications — clear them all.
pub async fn clear_all(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<serde_json::Value>> {
    let removed = db::notifications::clear_all(&state.db, &user.id)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "removed": removed })))
}
