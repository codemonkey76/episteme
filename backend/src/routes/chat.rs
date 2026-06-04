use axum::{
    extract::{Path, State},
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use futures::stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::agent::{self, AgentEvent};
use crate::db;
use axum::Extension;

use crate::error::{AppError, AppResult};
use crate::model_router::ProviderConfig;
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub provider: String,
}

pub async fn stream(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(session_id): Path<String>,
    Json(req): Json<ChatRequest>,
) -> AppResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>> {
    // Ownership gate: the session must belong to the caller.
    db::sessions::get(&state.db, &user.id, &session_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
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

    let providers: Vec<ProviderConfig> = db::settings::get(&state.db, "providers")
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    let provider = providers
        .into_iter()
        .find(|p| p.name == req.provider)
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("provider '{}' not found", req.provider)))?;

    let (tx, rx) = mpsc::channel::<AgentEvent>(64);

    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            // Surface failures — a silently dropped Err here looks like a dead
            // stream to the client with no trace anywhere.
            if let Err(e) =
                agent::run_turn(Arc::clone(&state), user.id.clone(), session_id, provider, tx).await
            {
                tracing::error!("agent turn failed: {e}");
                state.log("agent", "error", format!("turn failed: {e}")).await;
            }
        });
    }

    let event_stream = stream::unfold(rx, |mut rx| async move {
        let event = match rx.recv().await? {
            AgentEvent::Token(text) => Event::default()
                .data(serde_json::json!({ "type": "token", "text": text }).to_string()),
            AgentEvent::ToolCall { name } => Event::default()
                .data(serde_json::json!({ "type": "tool", "name": name }).to_string()),
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

    // Keep-alive comments hold the connection open while a slow/thinking model
    // produces no visible tokens; without them the idle stream gets reset
    // ("NetworkError when attempting to fetch resource").
    Ok(Sse::new(event_stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(5))))
}
