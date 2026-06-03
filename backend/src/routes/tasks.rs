use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::db::{self, tasks::TaskPatch};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const PRIORITIES: [&str; 3] = ["low", "normal", "high"];
const STATUSES: [&str; 2] = ["open", "done"];

fn validate(value: &str, allowed: &[&str], what: &str) -> Result<(), AppError> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "invalid {what} '{value}' — expected one of: {}",
            allowed.join(", ")
        )))
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
    q: Option<String>,
    limit: Option<i64>,
}

// GET /api/tasks?status=&q=&limit=
pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    // Treat "all" / empty as no filter.
    let status = params
        .status
        .as_deref()
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
    let q = params.q.as_deref().filter(|s| !s.is_empty());
    let tasks = db::tasks::list(&state.db, status, q, params.limit.unwrap_or(500))
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "tasks": tasks })))
}

#[derive(Deserialize)]
pub struct CreateTask {
    title: String,
    notes: Option<String>,
    due_at: Option<String>,
    priority: Option<String>,
}

// POST /api/tasks
pub async fn create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateTask>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let priority = body.priority.as_deref().unwrap_or("normal");
    validate(priority, &PRIORITIES, "priority")?;
    let task = db::tasks::insert(
        &state.db,
        &body.title,
        body.notes.as_deref(),
        body.due_at.as_deref(),
        priority,
    )
    .await
    .map_err(AppError::Internal)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "task": task }))))
}

/// Distinguishes a JSON field that's absent (outer None — leave unchanged)
/// from one explicitly set to null (Some(None) — clear it).
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(de).map(Some)
}

/// Partial update — absent fields are unchanged; `notes`/`due_at` accept null
/// to clear.
#[derive(Deserialize)]
pub struct UpdateTask {
    title: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    notes: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    due_at: Option<Option<String>>,
    priority: Option<String>,
    status: Option<String>,
}

// PUT /api/tasks/:id
pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTask>,
) -> AppResult<Json<serde_json::Value>> {
    if let Some(p) = body.priority.as_deref() {
        validate(p, &PRIORITIES, "priority")?;
    }
    if let Some(s) = body.status.as_deref() {
        validate(s, &STATUSES, "status")?;
    }
    let patch = TaskPatch {
        title: body.title,
        notes: body.notes,
        due_at: body.due_at,
        priority: body.priority,
        status: body.status,
    };
    let task = db::tasks::update(&state.db, &id, patch)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    Ok(Json(serde_json::json!({ "task": task })))
}

// DELETE /api/tasks/:id
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::tasks::delete(&state.db, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
