use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use std::sync::Arc;

use crate::calendar::{self, NewEvent};
use axum::Extension;

use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct RangeQuery {
    start: Option<String>,
    end: Option<String>,
}

// GET /api/calendar/events?start=&end=  (defaults: now .. now+30d)
pub async fn list_events(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(q): Query<RangeQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let start = q
        .start
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let end = q
        .end
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(start + Duration::days(30));

    let events = calendar::list_events(&state, &user.id, start, end)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "events": events })))
}

#[derive(Deserialize)]
pub struct CreateEvent {
    subject: String,
    start: String,
    end: Option<String>,
    #[serde(default)]
    is_all_day: bool,
    location: Option<String>,
    body: Option<String>,
    reminder_minutes_before: Option<i64>,
}

// POST /api/calendar/events
pub async fn create_event(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(b): Json<CreateEvent>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let event = calendar::create_event(
        &state,
        &user.id,
        NewEvent {
            subject: b.subject,
            start: b.start,
            end: b.end,
            is_all_day: b.is_all_day,
            location: b.location,
            body: b.body,
            reminder_minutes_before: b.reminder_minutes_before,
        },
    )
    .await
    .map_err(AppError::Internal)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "event": event }))))
}

// DELETE /api/calendar/events/:id
pub async fn delete_event(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    calendar::delete_event(&state, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
