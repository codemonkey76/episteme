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
    /// Optional pasted/attached images for vision-capable models.
    #[serde(default)]
    pub images: Vec<ImageUpload>,
}

#[derive(Deserialize)]
pub struct ImageUpload {
    pub mime: String,
    /// Raw image bytes, base64-encoded (no `data:` prefix).
    pub b64: String,
}

/// Keep a turn's image payload sane: most vision models take a handful of
/// images; oversized payloads just burn context and upload time.
const MAX_IMAGES: usize = 4;
const MAX_IMAGE_B64: usize = 8 * 1024 * 1024; // ~6 MB raw per image

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
    if req.images.len() > MAX_IMAGES {
        return Err(AppError::BadRequest(format!("at most {MAX_IMAGES} images per message")));
    }
    if req.images.iter().any(|i| i.b64.len() > MAX_IMAGE_B64) {
        return Err(AppError::BadRequest("image exceeds the 6 MB limit".into()));
    }

    // Plain text stays a JSON string (the common case); attached images switch
    // the stored content to the multimodal shape the model router understands.
    let content = if req.images.is_empty() {
        serde_json::to_string(&req.message).unwrap()
    } else {
        serde_json::json!({
            "type": "multimodal",
            "text": req.message,
            "images": req.images.iter().map(|i| serde_json::json!({ "mime": i.mime, "b64": i.b64 })).collect::<Vec<_>>(),
        })
        .to_string()
    };
    db::messages::insert(&state.db, &session_id, "user", &content, None, None)
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
                agent::run_turn(Arc::clone(&state), user.id.clone(), session_id, provider, tx, false)
                    .await
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
