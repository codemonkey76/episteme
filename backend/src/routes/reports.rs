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

#[derive(serde::Deserialize)]
pub struct StartResearch {
    topic: String,
    #[serde(default)]
    depth: Option<String>,
    #[serde(default)]
    provider: Option<String>,
}

// POST /api/research — the Reports window's "New research" launcher.
pub async fn start_research(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<StartResearch>,
) -> AppResult<Json<serde_json::Value>> {
    let (job_id, session_id) = crate::research::launch(
        &state,
        &user.id,
        &body.topic,
        body.depth.as_deref().unwrap_or("standard"),
        body.provider.as_deref().unwrap_or(""),
    )
    .await
    // Launch failures are user-facing config problems (empty topic, unknown
    // provider) — surface them as 400s with the message.
    .map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(serde_json::json!({ "job_id": job_id, "session_id": session_id })))
}
