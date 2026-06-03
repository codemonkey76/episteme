use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
    response::Response,
    Json,
};
use futures::stream;
use serde::Deserialize;
use serde_json::Value;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::integrations::microsoft;
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig, StreamChunk};
use crate::state::AppState;
use tokio::sync::mpsc;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

pub async fn graph_get(
    state: &AppState,
    url: &str,
    params: &[(&str, &str)],
) -> AppResult<Value> {
    let token = microsoft::get_valid_token(state)
        .await
        .map_err(AppError::Internal)?;

    let response = state
        .http_client
        .get(url)
        .query(params)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    if !status.is_success() {
        let msg = body["error"]["message"]
            .as_str()
            .unwrap_or("Graph API error")
            .to_string();
        tracing::error!("Graph API {status}: {msg}");
        return Err(AppError::Internal(anyhow::anyhow!("Graph API {status}: {msg}")));
    }

    Ok(body)
}

/// POST a JSON body to Graph and return the parsed response. Used by the
/// categorizer worker for folder creation and message moves.
pub async fn graph_post(state: &AppState, url: &str, body: &Value) -> AppResult<Value> {
    let token = microsoft::get_valid_token(state)
        .await
        .map_err(AppError::Internal)?;

    let response = state
        .http_client
        .post(url)
        .bearer_auth(&token)
        .json(body)
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    let status = response.status();
    let parsed: Value = response.json().await.unwrap_or(Value::Null);

    if !status.is_success() {
        let msg = parsed["error"]["message"]
            .as_str()
            .unwrap_or("Graph API error")
            .to_string();
        tracing::error!("Graph POST {status}: {msg}");
        return Err(AppError::Internal(anyhow::anyhow!("Graph POST {status}: {msg}")));
    }

    Ok(parsed)
}

/// DELETE a Graph resource. Treats any 2xx as success.
pub async fn graph_delete(state: &AppState, url: &str) -> AppResult<()> {
    let token = microsoft::get_valid_token(state)
        .await
        .map_err(AppError::Internal)?;

    let response = state
        .http_client
        .delete(url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    if !response.status().is_success() {
        let status = response.status();
        tracing::error!("Graph DELETE {status}");
        return Err(AppError::Internal(anyhow::anyhow!("Graph DELETE {status}")));
    }
    Ok(())
}

/// Return the id of the mail folder named `name`, creating it under the mailbox
/// root if it doesn't already exist. Matching is case-insensitive on displayName.
pub async fn ensure_folder(state: &AppState, name: &str) -> AppResult<String> {
    let existing = graph_get(
        state,
        &format!("{GRAPH}/me/mailFolders"),
        &[("$top", "100"), ("$select", "id,displayName")],
    )
    .await?;

    if let Some(folders) = existing["value"].as_array() {
        for f in folders {
            if f["displayName"].as_str().map(|d| d.eq_ignore_ascii_case(name)).unwrap_or(false) {
                if let Some(id) = f["id"].as_str() {
                    return Ok(id.to_string());
                }
            }
        }
    }

    let created = graph_post(
        state,
        &format!("{GRAPH}/me/mailFolders"),
        &serde_json::json!({ "displayName": name }),
    )
    .await?;

    created["id"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("created folder missing id")))
}

/// Move a message into the destination folder.
pub async fn move_message(state: &AppState, message_id: &str, dest_folder_id: &str) -> AppResult<()> {
    graph_post(
        state,
        &format!("{GRAPH}/me/messages/{message_id}/move"),
        &serde_json::json!({ "destinationId": dest_folder_id }),
    )
    .await?;
    Ok(())
}

/// Set the follow-up flag on a message, leaving it in place.
pub async fn flag_message(state: &AppState, message_id: &str) -> AppResult<()> {
    let token = microsoft::get_valid_token(state)
        .await
        .map_err(AppError::Internal)?;

    let res = state
        .http_client
        .patch(format!("{GRAPH}/me/messages/{message_id}"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "flag": { "flagStatus": "flagged" } }))
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    if !res.status().is_success() {
        let status = res.status();
        return Err(AppError::Internal(anyhow::anyhow!("flag failed: {status}")));
    }
    Ok(())
}

// GET /api/email/folders
pub async fn list_folders(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    let res = graph_get(
        &state,
        &format!("{GRAPH}/me/mailFolders"),
        &[
            ("$top", "30"),
            ("$select", "id,displayName,unreadItemCount,totalItemCount"),
        ],
    )
    .await?;
    Ok(Json(res))
}

// GET /api/email/search?q=...&next_link=...&top=30
#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    next_link: Option<String>,
    top: Option<u32>,
}

pub async fn search_messages(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchQuery>,
) -> AppResult<Json<Value>> {
    let token = microsoft::get_valid_token(&state)
        .await
        .map_err(AppError::Internal)?;

    let url = if let Some(ref next_link) = params.next_link {
        if !next_link.starts_with("https://graph.microsoft.com/") {
            return Err(AppError::Internal(anyhow::anyhow!("invalid next_link")));
        }
        next_link.clone()
    } else {
        let q = params.q.as_deref().unwrap_or("").replace('"', "");
        let top = params.top.unwrap_or(30).min(50);
        format!(
            "{GRAPH}/me/messages?$search=%22{}%22&$select=id,subject,from,toRecipients,bodyPreview,receivedDateTime,isRead,hasAttachments,flag&$expand=singleValueExtendedProperties($filter=id%20eq%20'Integer%200x1081')&$top={top}",
            q
        )
    };

    let response = state
        .http_client
        .get(&url)
        .bearer_auth(&token)
        .header("ConsistencyLevel", "eventual")
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    let status = response.status();
    let body: Value = response.json().await.map_err(|e| AppError::Internal(e.into()))?;

    if !status.is_success() {
        let msg = body["error"]["message"]
            .as_str()
            .unwrap_or("Graph API error")
            .to_string();
        tracing::error!("Graph search {status}: {msg}");
        return Err(AppError::Internal(anyhow::anyhow!("Graph search {status}: {msg}")));
    }

    tracing::debug!("Graph search returned {} results", body["value"].as_array().map_or(0, |a| a.len()));

    let next_link = body["@odata.nextLink"].as_str().map(|s| s.to_string());
    Ok(Json(serde_json::json!({
        "value": body["value"].as_array().cloned().unwrap_or_default(),
        "next_link": next_link,
    })))
}

// GET /api/email/folders/:id/messages?skip=0&top=30
#[derive(Deserialize)]
pub struct MessagesQuery {
    skip: Option<u32>,
    top: Option<u32>,
}

pub async fn list_messages(
    State(state): State<Arc<AppState>>,
    Path(folder_id): Path<String>,
    Query(q): Query<MessagesQuery>,
) -> AppResult<Json<Value>> {
    let skip = q.skip.unwrap_or(0).to_string();
    let top = q.top.unwrap_or(30).min(50).to_string();

    let res = graph_get(
        &state,
        &format!("{GRAPH}/me/mailFolders/{folder_id}/messages"),
        &[
            ("$select", "id,subject,from,toRecipients,bodyPreview,receivedDateTime,isRead,hasAttachments,flag"),
            // PidTagLastVerbExecuted (0x1081) tells us whether the message was
            // last replied to / forwarded — the signal Outlook uses for its arrow.
            ("$expand", "singleValueExtendedProperties($filter=id eq 'Integer 0x1081')"),
            ("$orderby", "receivedDateTime desc"),
            ("$top", &top),
            ("$skip", &skip),
        ],
    )
    .await?;
    Ok(Json(res))
}

// GET /api/email/messages/:id
pub async fn get_message(
    State(state): State<Arc<AppState>>,
    Path(message_id): Path<String>,
) -> AppResult<Json<Value>> {
    let res = graph_get(
        &state,
        &format!("{GRAPH}/me/messages/{message_id}"),
        &[("$select", "id,subject,from,toRecipients,ccRecipients,body,receivedDateTime,isRead,hasAttachments")],
    )
    .await?;
    Ok(Json(res))
}

// GET /api/email/messages/:id/attachments — metadata only (no bytes).
pub async fn list_attachments(
    State(state): State<Arc<AppState>>,
    Path(message_id): Path<String>,
) -> AppResult<Json<Value>> {
    let res = graph_get(
        &state,
        &format!("{GRAPH}/me/messages/{message_id}/attachments"),
        &[("$select", "id,name,contentType,size,isInline")],
    )
    .await?;
    Ok(Json(res))
}

// GET /api/email/messages/:id/attachments/:att_id/raw — stream the file bytes
// through with their content type so the browser can render or download them.
pub async fn get_attachment_raw(
    State(state): State<Arc<AppState>>,
    Path((message_id, att_id)): Path<(String, String)>,
) -> AppResult<Response> {
    let token = microsoft::get_valid_token(&state)
        .await
        .map_err(AppError::Internal)?;

    let url = format!("{GRAPH}/me/messages/{message_id}/attachments/{att_id}/$value");
    let upstream = state
        .http_client
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    if !upstream.status().is_success() {
        let status = upstream.status();
        tracing::error!("attachment fetch failed: {status}");
        return Err(AppError::Internal(anyhow::anyhow!("attachment fetch failed: {status}")));
    }

    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let bytes = upstream
        .bytes()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_DISPOSITION, "inline")
        .body(Body::from(bytes))
        .map_err(|e| AppError::Internal(e.into()))
}

// PATCH /api/email/messages/:id/read
pub async fn mark_read(
    State(state): State<Arc<AppState>>,
    Path(message_id): Path<String>,
) -> AppResult<StatusCode> {
    let token = microsoft::get_valid_token(&state)
        .await
        .map_err(AppError::Internal)?;

    let res = state
        .http_client
        .patch(format!("{GRAPH}/me/messages/{message_id}"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "isRead": true }))
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    if !res.status().is_success() {
        tracing::warn!("mark_read failed: {}", res.status());
    }

    Ok(StatusCode::NO_CONTENT)
}

// POST /api/email/send
#[derive(Deserialize)]
pub struct SendBody {
    to: Vec<String>,
    subject: Option<String>,
    body: String,
    reply_to_message_id: Option<String>,
    /// "reply" | "replyAll" | "forward" — only "forward" changes the Graph
    /// endpoint; reply/replyAll both send via /reply with explicit recipients.
    #[serde(default)]
    action: Option<String>,
}

pub async fn send_email(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SendBody>,
) -> AppResult<StatusCode> {
    let token = microsoft::get_valid_token(&state)
        .await
        .map_err(AppError::Internal)?;

    let recipients = payload
        .to
        .iter()
        .map(|addr| serde_json::json!({ "emailAddress": { "address": addr } }))
        .collect::<Vec<_>>();

    if let Some(reply_id) = payload.reply_to_message_id {
        // /forward carries the original message; /reply (used for both reply and
        // reply-all) takes the recipient list the frontend computed.
        let (url, body) = if payload.action.as_deref() == Some("forward") {
            (
                format!("{GRAPH}/me/messages/{reply_id}/forward"),
                serde_json::json!({ "toRecipients": recipients, "comment": payload.body }),
            )
        } else {
            (
                format!("{GRAPH}/me/messages/{reply_id}/reply"),
                serde_json::json!({ "message": { "toRecipients": recipients }, "comment": payload.body }),
            )
        };
        state
            .http_client
            .post(url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
    } else {
        let body = serde_json::json!({
            "message": {
                "subject": payload.subject.unwrap_or_default(),
                "body": { "contentType": "Text", "content": payload.body },
                "toRecipients": recipients,
            },
        });
        state
            .http_client
            .post(format!("{GRAPH}/me/sendMail"))
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
    }

    Ok(StatusCode::NO_CONTENT)
}

// POST /api/email/ai-draft — produce a draft reply for the user to edit/send.
#[derive(Deserialize)]
pub struct AiDraftBody {
    provider: String,
    from: String,
    subject: String,
    /// Plain-text body of the email being replied to (HTML stripped client-side).
    body: String,
}

pub async fn ai_draft(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AiDraftBody>,
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

    let system = "You draft email replies on behalf of the user. Output only the body of the \
reply — no subject line, no quoted original message, and no placeholder tokens like [Name]. \
Keep it clear, polite, and concise in a professional tone, and directly address anything the \
email asks.";

    let user = format!(
        "Draft a reply to this email.\n\nFrom: {}\nSubject: {}\n\n{}",
        payload.from, payload.subject, payload.body
    );

    let history = vec![
        ChatMessage { role: "system".to_string(), content: Value::String(system.to_string()) },
        ChatMessage { role: "user".to_string(), content: Value::String(user) },
    ];

    // Stream the model output to the client token-by-token (SSE), like chat.
    // A wrapper task forwards tokens and, if the model call fails (e.g. the
    // provider is unreachable), emits an `error` event — the failure happens
    // after the 200 response has begun, so it can't be reported via the status.
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
            tracing::error!("ai_draft stream error: {e}");
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

// ── Auto-categorizer config / manual run ───────────────────────────────────────

// GET /api/email/categorizer
pub async fn get_categorizer(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<crate::categorizer::CategorizerConfig>> {
    let cfg = crate::categorizer::get_config(&state.db)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(cfg))
}

// PUT /api/email/categorizer
pub async fn put_categorizer(
    State(state): State<Arc<AppState>>,
    Json(cfg): Json<crate::categorizer::CategorizerConfig>,
) -> AppResult<Json<crate::categorizer::CategorizerConfig>> {
    crate::categorizer::set_config(&state.db, &cfg)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(cfg))
}

// POST /api/email/categorizer/run — run categorization immediately.
pub async fn run_categorizer(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<crate::categorizer::RunSummary>> {
    let summary = crate::categorizer::run_once(&state)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(summary))
}
