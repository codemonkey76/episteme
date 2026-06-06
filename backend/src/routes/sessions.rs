use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use axum::Extension;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateSession {
    pub title: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateSession {
    pub title: String,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<serde_json::Value>> {
    let sessions = db::sessions::list(&state.db, &user.id).await.map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "sessions": sessions })))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<CreateSession>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let title = body.title.as_deref().unwrap_or("New conversation");
    let session =
        db::sessions::create(&state.db, &user.id, title).await.map_err(AppError::Internal)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "session": session }))))
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let session = db::sessions::get(&state.db, &user.id, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    Ok(Json(serde_json::json!({ "session": session })))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSession>,
) -> AppResult<Json<serde_json::Value>> {
    db::sessions::update_title(&state.db, &user.id, &id, &body.title)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::sessions::delete(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn messages(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    // Ownership gate before exposing the transcript.
    db::sessions::get(&state.db, &user.id, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    let msgs = db::messages::list_for_session(&state.db, &id)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "messages": msgs })))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    limit: Option<i64>,
}

// GET /api/sessions/search?q=… — full-text search across the user's chats.
pub async fn search(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    axum::extract::Query(params): axum::extract::Query<SearchQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let q = params.q.as_deref().unwrap_or("").trim();
    if q.is_empty() {
        return Ok(Json(serde_json::json!({ "hits": [] })));
    }
    let limit = params.limit.unwrap_or(30).clamp(1, 100);
    let hits = db::messages::search(&state.db, &user.id, q, limit)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "hits": hits })))
}
