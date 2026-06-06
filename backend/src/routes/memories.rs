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
    category: Option<String>,
    q: Option<String>,
    limit: Option<i64>,
}

// GET /api/memories?category=&q=&limit=
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    // Treat "All" / empty as no filter.
    let category = params.category.as_deref().filter(|c| !c.is_empty() && *c != "All");
    let q = params.q.as_deref().filter(|s| !s.is_empty());
    let memories =
        db::memories::list(&state.db, &user.id, category, q, params.limit.unwrap_or(500))
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "memories": memories })))
}

#[derive(Deserialize)]
pub struct CreateMemory {
    content: String,
    category: Option<String>,
}

// POST /api/memories
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<CreateMemory>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let category = body.category.as_deref().unwrap_or("other");
    let memory =
        db::memories::insert(&state.db, &user.id, &body.content, category, "manual", None)
        .await
        .map_err(AppError::Internal)?;
    crate::memory::embed_detached(&state, memory.id.clone(), memory.content.clone());
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "memory": memory }))))
}

#[derive(Deserialize)]
pub struct UpdateMemory {
    content: String,
    category: String,
}

// PUT /api/memories/:id
pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<UpdateMemory>,
) -> AppResult<StatusCode> {
    db::memories::update(&state.db, &user.id, &id, &body.content, &body.category)
        .await
        .map_err(AppError::Internal)?;
    crate::memory::embed_detached(&state, id, body.content);
    Ok(StatusCode::NO_CONTENT)
}

// DELETE /api/memories/:id
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::memories::delete(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
