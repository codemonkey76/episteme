use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateSession {
    pub title: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateSession {
    pub title: String,
}

pub async fn list(State(state): State<Arc<AppState>>) -> AppResult<Json<serde_json::Value>> {
    let sessions = db::sessions::list(&state.db).await.map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "sessions": sessions })))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSession>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let title = body.title.as_deref().unwrap_or("New conversation");
    let session = db::sessions::create(&state.db, title).await.map_err(AppError::Internal)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "session": session }))))
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let session = db::sessions::get(&state.db, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    Ok(Json(serde_json::json!({ "session": session })))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSession>,
) -> AppResult<Json<serde_json::Value>> {
    db::sessions::update_title(&state.db, &id, &body.title)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::sessions::delete(&state.db, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn messages(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let msgs = db::messages::list_for_session(&state.db, &id)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "messages": msgs })))
}
