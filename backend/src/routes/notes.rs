use axum::{
    extract::{Path, Query, State},
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
pub struct ListQuery {
    q: Option<String>,
    limit: Option<i64>,
}

// GET /api/notes?q=&limit=
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let q = params.q.as_deref().filter(|s| !s.is_empty());
    let notes = db::notes::list(&state.db, &user.id, q, params.limit.unwrap_or(500))
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "notes": notes })))
}

#[derive(Deserialize)]
pub struct CreateNote {
    title: String,
    content: String,
}

// POST /api/notes
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<CreateNote>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let note = db::notes::insert(&state.db, &user.id, &body.title, &body.content)
        .await
        .map_err(AppError::Internal)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "note": note }))))
}

#[derive(Deserialize)]
pub struct UpdateNote {
    title: Option<String>,
    content: Option<String>,
}

// PUT /api/notes/:id — partial; absent fields unchanged
pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<UpdateNote>,
) -> AppResult<Json<serde_json::Value>> {
    let note = db::notes::update(&state.db, &user.id, &id, body.title, body.content)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    Ok(Json(serde_json::json!({ "note": note })))
}

// DELETE /api/notes/:id
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::notes::delete(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
