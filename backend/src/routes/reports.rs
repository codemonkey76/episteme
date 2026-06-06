use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
    Json,
};
use std::sync::Arc;

use axum::Extension;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

// GET /api/reports — the user's reports, newest first (metadata only).
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<serde_json::Value>> {
    let reports =
        db::reports::list_for_user(&state.db, &user.id, 100).await.map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "reports": reports })))
}

// GET /api/reports/:id/html — the self-contained report document, served raw
// for iframes / open-in-new-tab (cookie-authed like every /api route).
pub async fn html(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let html = db::reports::get_html(&state.db, &user.id, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(html))
        .map_err(|e| AppError::Internal(e.into()))
}

// DELETE /api/reports/:id
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::reports::delete(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
