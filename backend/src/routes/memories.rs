use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::db;
use crate::error::{AppError, AppResult};
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
    Query(params): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    // Treat "All" / empty as no filter.
    let category = params.category.as_deref().filter(|c| !c.is_empty() && *c != "All");
    let q = params.q.as_deref().filter(|s| !s.is_empty());
    let memories = db::memories::list(&state.db, category, q, params.limit.unwrap_or(500))
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
    Json(body): Json<CreateMemory>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let category = body.category.as_deref().unwrap_or("other");
    let memory = db::memories::insert(&state.db, &body.content, category, "manual", None)
        .await
        .map_err(AppError::Internal)?;
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
    Path(id): Path<String>,
    Json(body): Json<UpdateMemory>,
) -> AppResult<StatusCode> {
    db::memories::update(&state.db, &id, &body.content, &body.category)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// DELETE /api/memories/:id
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::memories::delete(&state.db, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
