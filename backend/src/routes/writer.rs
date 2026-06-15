use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    Extension, Json,
};
use futures::stream;
use serde::Deserialize;
use serde_json::Value;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::mpsc;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig, StreamChunk};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ListQuery {
    q: Option<String>,
    limit: Option<i64>,
}

// GET /api/writer/docs?q=&limit=
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let q = params.q.as_deref().filter(|s| !s.is_empty());
    let docs = db::writer::list(&state.db, &user.id, q, params.limit.unwrap_or(500))
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "docs": docs })))
}

#[derive(Deserialize)]
pub struct CreateDoc {
    title: String,
    content: String,
}

// POST /api/writer/docs
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<CreateDoc>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let doc = db::writer::insert(&state.db, &user.id, &body.title, &body.content)
        .await
        .map_err(AppError::Internal)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "doc": doc }))))
}

#[derive(Deserialize)]
pub struct UpdateDoc {
    title: Option<String>,
    content: Option<String>,
}

// PUT /api/writer/docs/:id — partial; absent fields unchanged
pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<UpdateDoc>,
) -> AppResult<Json<serde_json::Value>> {
    let doc = db::writer::update(&state.db, &user.id, &id, body.title, body.content)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    Ok(Json(serde_json::json!({ "doc": doc })))
}

// DELETE /api/writer/docs/:id
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::writer::delete(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// POST /api/writer/improve — rewrite the whole document or a selected passage,
// streaming the improved text back so the client can show accept/revert.
#[derive(Deserialize)]
pub struct ImproveBody {
    provider: String,
    /// The text to improve — either the whole document or a selected passage.
    text: String,
    /// When a passage is selected, the full document for context (so the
    /// rewrite stays consistent). Empty/absent when improving the whole thing.
    #[serde(default)]
    context: String,
}

pub async fn improve(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(_user)): Extension<CurrentUser>,
    Json(payload): Json<ImproveBody>,
) -> AppResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>> {
    let providers: Vec<ProviderConfig> = db::settings::get(&state.db, "providers")
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    let provider = providers
        .into_iter()
        .find(|p| p.name == payload.provider)
        .ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("provider '{}' not found", payload.provider))
        })?;

    let system = crate::prompts::get(&state.db, "writer_improve").await;

    // When improving a selection, give the model the whole document for context
    // but tell it to rewrite only the excerpt.
    let user = if payload.context.trim().is_empty() {
        format!("Improve the following text:\n\n{}", payload.text)
    } else {
        format!(
            "Here is a document, for context only — do not rewrite it:\n\n{}\n\n---\n\n\
Now improve ONLY this excerpt from it, keeping it consistent with the document. \
Output just the improved excerpt:\n\n{}",
            payload.context, payload.text
        )
    };

    let history = vec![
        ChatMessage { role: "system".to_string(), content: Value::String(system) },
        ChatMessage { role: "user".to_string(), content: Value::String(user) },
    ];

    let (ev_tx, ev_rx) = mpsc::channel::<String>(64);
    tokio::spawn(async move {
        let (tx, mut rx) = mpsc::channel::<StreamChunk>(64);
        let model_task =
            tokio::spawn(async move { ModelRouter::stream(&provider, history, Vec::new(), false, tx).await });

        while let Some(chunk) = rx.recv().await {
            let data = if chunk.done {
                serde_json::json!({ "type": "done" }).to_string()
            } else {
                serde_json::json!({ "type": "token", "text": chunk.text }).to_string()
            };
            if ev_tx.send(data).await.is_err() {
                return; // client disconnected
            }
        }

        if let Ok(Err(e)) = model_task.await {
            tracing::error!("writer improve stream error: {e}");
            let _ = ev_tx
                .send(serde_json::json!({ "type": "error", "message": e.to_string() }).to_string())
                .await;
        }
    });

    let event_stream = stream::unfold(ev_rx, |mut rx| async move {
        let data = rx.recv().await?;
        Some((Ok::<Event, Infallible>(Event::default().data(data)), rx))
    });

    Ok(Sse::new(event_stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(5))))
}
