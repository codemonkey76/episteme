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
use crate::scheduler::{self, ScheduledAgent};
use crate::state::AppState;

// GET /api/scheduled-agents
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<serde_json::Value>> {
    let agents = scheduler::get_agents(&state.db, &user.id).await.map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "agents": agents })))
}

// PUT /api/scheduled-agents — replaces the whole list (the UI edits in place).
// Missing ids are minted; times are validated; last_run survives round-trips.
pub async fn put(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(mut agents): Json<Vec<ScheduledAgent>>,
) -> AppResult<Json<serde_json::Value>> {
    for a in agents.iter_mut() {
        if a.id.is_empty() {
            a.id = scheduler::new_id();
        }
        let valid_time = a
            .time
            .split_once(':')
            .and_then(|(h, m)| Some((h.parse::<u32>().ok()?, m.parse::<u32>().ok()?)))
            .is_some_and(|(h, m)| h < 24 && m < 60);
        if !valid_time {
            return Err(AppError::BadRequest(format!("'{}': time must be HH:MM", a.name)));
        }
        if a.name.trim().is_empty() || a.instructions.trim().is_empty() {
            return Err(AppError::BadRequest("agents need a name and instructions".into()));
        }
    }
    scheduler::set_agents(&state.db, &user.id, &agents).await.map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "agents": agents })))
}

// POST /api/scheduled-agents/:id/run — manual "Run now"; returns the session.
pub async fn run(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let agents = scheduler::get_agents(&state.db, &user.id).await.map_err(AppError::Internal)?;
    let agent = agents.into_iter().find(|a| a.id == id).ok_or(AppError::NotFound)?;
    let session_id = scheduler::run_now(&state, &user.id, &agent)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "session_id": session_id })))
}

#[derive(Deserialize)]
pub struct RegisterToken {
    token: String,
    #[serde(default)]
    platform: Option<String>,
}

// GET /api/push/vapid — the public key browsers pass to pushManager.subscribe.
pub async fn vapid_public(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let key = crate::integrations::webpush::public_key(&state.db)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "public_key": key })))
}

// POST /api/push/register — a device reports its push credential: the mobile
// app's FCM token, or a browser's Web Push subscription JSON (platform "web").
pub async fn register_push(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<RegisterToken>,
) -> AppResult<StatusCode> {
    if body.token.trim().is_empty() {
        return Err(AppError::BadRequest("token is required".into()));
    }
    db::push_tokens::register(
        &state.db,
        &user.id,
        body.token.trim(),
        body.platform.as_deref().unwrap_or("android"),
    )
    .await
    .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
