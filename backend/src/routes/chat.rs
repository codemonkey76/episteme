use axum::{
    extract::{Path, State},
    response::sse::{Event, Sse},
    Json,
};
use futures::stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::agent::{self, AgentEvent};
use crate::db;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub provider: String,
}

pub async fn stream(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(req): Json<ChatRequest>,
) -> AppResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>> {
    db::messages::insert(
        &state.db,
        &session_id,
        "user",
        &serde_json::to_string(&req.message).unwrap(),
        None,
        None,
    )
    .await
    .map_err(AppError::Internal)?;

    db::sessions::touch(&state.db, &session_id)
        .await
        .map_err(AppError::Internal)?;

    let (tx, rx) = mpsc::channel::<AgentEvent>(64);

    tokio::spawn(agent::run_turn(
        Arc::clone(&state),
        session_id,
        req.provider,
        tx,
    ));

    let event_stream = stream::unfold(rx, |mut rx| async move {
        let event = match rx.recv().await? {
            AgentEvent::Token(text) => Event::default()
                .data(serde_json::json!({ "type": "token", "text": text }).to_string()),
            AgentEvent::Done => {
                Event::default().data(serde_json::json!({ "type": "done" }).to_string())
            }
            AgentEvent::AwaitingApproval { action_id, tool_name, tool_args } => {
                Event::default().data(
                    serde_json::json!({
                        "type": "awaiting_approval",
                        "action_id": action_id,
                        "tool_name": tool_name,
                        "tool_args": tool_args,
                    })
                    .to_string(),
                )
            }
        };
        Some((Ok::<Event, Infallible>(event), rx))
    });

    Ok(Sse::new(event_stream))
}
