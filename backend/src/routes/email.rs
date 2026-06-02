use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::error::{AppError, AppResult};
use crate::integrations::microsoft;
use crate::state::AppState;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

async fn graph_get(
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
            ("$select", "id,subject,from,toRecipients,bodyPreview,receivedDateTime,isRead,hasAttachments"),
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
        let body = serde_json::json!({
            "message": { "toRecipients": recipients },
            "comment": payload.body,
        });
        state
            .http_client
            .post(format!("{GRAPH}/me/messages/{reply_id}/reply"))
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
