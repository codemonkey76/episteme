use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use axum::Extension;

use crate::db::{self, tasks::TaskPatch};
use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
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
    /// "general" = the implicit General list; a list id = that list; absent/empty = all.
    list: Option<String>,
}

fn list_filter(param: Option<&str>) -> db::tasks::ListFilter<'_> {
    match param.filter(|s| !s.is_empty()) {
        None => db::tasks::ListFilter::All,
        Some(s) if s.eq_ignore_ascii_case("general") => db::tasks::ListFilter::General,
        Some(id) => db::tasks::ListFilter::List(id),
    }
}

// GET /api/tasks?status=&q=&limit=&list=
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    // Treat "all" / empty as no filter.
    let status = params
        .status
        .as_deref()
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
    let q = params.q.as_deref().filter(|s| !s.is_empty());
    let tasks = db::tasks::list(
        &state.db,
        &user.id,
        status,
        q,
        list_filter(params.list.as_deref()),
        params.limit.unwrap_or(500),
    )
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
    /// Target list id; absent/null = the implicit General list.
    list_id: Option<String>,
}

// POST /api/tasks
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<CreateTask>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let priority = body.priority.as_deref().unwrap_or("normal");
    validate(priority, &PRIORITIES, "priority")?;
    let task = db::tasks::insert(
        &state.db,
        &user.id,
        &body.title,
        body.notes.as_deref(),
        body.due_at.as_deref(),
        priority,
        body.list_id.as_deref().filter(|s| !s.is_empty()),
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
    /// null moves the task back to the implicit General list.
    #[serde(default, deserialize_with = "double_option")]
    list_id: Option<Option<String>>,
}

// PUT /api/tasks/:id
pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
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
        list_id: body.list_id,
    };
    let task = db::tasks::update(&state.db, &user.id, &id, patch)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    Ok(Json(serde_json::json!({ "task": task })))
}

// DELETE /api/tasks/:id
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::tasks::delete(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── To-do lists ─────────────────────────────────────────────────────────────────

// GET /api/tasks/lists — the user's named lists (the implicit "General" list
// is not stored; the frontend renders it as the first entry).
pub async fn list_lists(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<serde_json::Value>> {
    let lists = db::tasks::lists(&state.db, &user.id).await.map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "lists": lists })))
}

#[derive(Deserialize)]
pub struct ListBody {
    name: String,
}

fn clean_list_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() || name.len() > 60 {
        return Err(AppError::BadRequest("list name must be 1–60 characters".into()));
    }
    if name.eq_ignore_ascii_case("general") {
        return Err(AppError::BadRequest("\"General\" is the built-in default list".into()));
    }
    Ok(name.to_string())
}

// POST /api/tasks/lists
pub async fn create_list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<ListBody>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let name = clean_list_name(&body.name)?;
    if db::tasks::list_by_name(&state.db, &user.id, &name)
        .await
        .map_err(AppError::Internal)?
        .is_some()
    {
        return Err(AppError::BadRequest(format!("a list named \"{name}\" already exists")));
    }
    let list = db::tasks::insert_list(&state.db, &user.id, &name)
        .await
        .map_err(AppError::Internal)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "list": list }))))
}

// PUT /api/tasks/lists/:id — rename.
pub async fn rename_list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<ListBody>,
) -> AppResult<StatusCode> {
    let name = clean_list_name(&body.name)?;
    let found = db::tasks::rename_list(&state.db, &user.id, &id, &name)
        .await
        .map_err(AppError::Internal)?;
    if found { Ok(StatusCode::NO_CONTENT) } else { Err(AppError::NotFound) }
}

// DELETE /api/tasks/lists/:id — its tasks fall back to General.
pub async fn delete_list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::tasks::delete_list(&state.db, &user.id, &id)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
