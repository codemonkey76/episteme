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

#[derive(Deserialize)]
pub struct ConsolidateBody {
    /// Provider to dream with; omitted = the saved default (or first configured).
    #[serde(default)]
    provider: Option<String>,
}

// POST /api/memories/consolidate — run the "dream" pass now and return a summary.
pub async fn consolidate(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<ConsolidateBody>,
) -> AppResult<Json<serde_json::Value>> {
    let provider = crate::memory::consolidate::resolve_provider(&state, body.provider.as_deref())
        .await
        .ok_or_else(|| AppError::BadRequest("no AI provider configured".into()))?;
    // Remember an explicit choice so the nightly run uses the same model.
    if let Some(name) = body.provider.as_deref().filter(|s| !s.is_empty()) {
        let _ = db::settings::set(&state.db, "memory_consolidation_provider", &name).await;
    }
    let summary = crate::memory::consolidate::run(&state, &user.id, &provider)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "summary": summary, "provider": provider.name })))
}

// GET /api/memories/deleted — archived (soft-deleted) memories, for restore.
pub async fn list_deleted(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<serde_json::Value>> {
    let memories =
        db::memories::list_deleted(&state.db, &user.id, 500).await.map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "memories": memories })))
}

// POST /api/memories/:id/restore — undo a consolidation by restoring a memory.
pub async fn restore(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::memories::restore(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
