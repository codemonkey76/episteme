use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;

use crate::calendar::{self, NewEvent};
use crate::db;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

// GET /api/suggestions
pub async fn list_pending(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let suggestions = db::suggestions::list_pending(&state.db)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "suggestions": suggestions })))
}

// POST /api/suggestions/:id/accept — create the task/event, resolve the row.
pub async fn accept(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let s = db::suggestions::get(&state.db, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    if s.status != "pending" {
        return Err(AppError::Conflict(format!("suggestion already {}", s.status)));
    }

    let created = if s.kind == "event" {
        // Detector guarantees events have a start; guard anyway.
        let start = s.start_at.clone().ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("event suggestion missing start time"))
        })?;
        let ev = calendar::create_event(
            &state,
            NewEvent {
                subject: s.title.clone(),
                start,
                end: s.end_at.clone(),
                is_all_day: false,
                location: None,
                body: s.context.clone(),
                reminder_minutes_before: None,
            },
        )
        .await
        .map_err(AppError::Internal)?;
        serde_json::json!({ "kind": "event", "event": ev })
    } else {
        let task = db::tasks::insert(
            &state.db,
            &s.title,
            s.context.as_deref(),
            s.start_at.as_deref(),
            "normal",
        )
        .await
        .map_err(AppError::Internal)?;
        serde_json::json!({ "kind": "task", "task": task })
    };

    db::suggestions::resolve(&state.db, &id, "accepted")
        .await
        .map_err(AppError::Internal)?;
    state
        .log("suggestions", "info", format!("accepted ({}): {}", s.kind, s.title))
        .await;
    Ok(Json(created))
}

// POST /api/suggestions/:id/dismiss
pub async fn dismiss(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::suggestions::resolve(&state.db, &id, "dismissed")
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
