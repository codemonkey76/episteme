use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use futures::stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::db;
use crate::db::logs::LogEntry;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

// POST /api/logs
pub async fn create(
    State(state): State<Arc<AppState>>,
    Json(entry): Json<LogEntry>,
) -> AppResult<StatusCode> {
    db::logs::insert(&state.db, &entry).await.map_err(AppError::Internal)?;
    let json = serde_json::to_string(&entry).unwrap_or_default();
    let _ = state.log_tx.send(json);
    Ok(StatusCode::CREATED)
}

// GET /api/logs
#[derive(Deserialize)]
pub struct ListQuery {
    limit: Option<i64>,
    offset: Option<i64>,
    category: Option<String>,
    level: Option<String>,
    q: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let entries = db::logs::list(
        &state.db,
        params.limit.unwrap_or(1000),
        params.offset.unwrap_or(0),
        params.category.as_deref(),
        params.level.as_deref(),
        params.q.as_deref(),
    )
    .await
    .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "entries": entries })))
}

// GET /api/logs/stream  — SSE stream of new log entries
pub async fn stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.log_tx.subscribe();
    let event_stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(json) => {
                    return Some((Ok::<Event, Infallible>(Event::default().data(json)), rx));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(event_stream).keep_alive(KeepAlive::default())
}

// DELETE /api/logs
pub async fn clear(
    State(state): State<Arc<AppState>>,
) -> AppResult<StatusCode> {
    db::logs::clear(&state.db).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
