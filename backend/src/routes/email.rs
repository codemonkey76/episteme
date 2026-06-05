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

use axum::Extension;

use crate::agent::{self, AgentEvent};
use crate::db;
use crate::routes::auth::CurrentUser;
use crate::error::{AppError, AppResult};
use crate::integrations::microsoft;
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig, StreamChunk};
use crate::state::AppState;
use tokio::sync::mpsc;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

/// Graph mailbox path segment: `me` for the signed-in user, or `users/{address}`
/// for a shared mailbox the user has delegated access to. The address is
/// validated to block path injection; Graph still enforces the real access
/// rights, so an address the user can't open just yields a 403.
pub fn mailbox_seg(mailbox: Option<&str>) -> AppResult<String> {
    match mailbox.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok("me".to_string()),
        Some(addr) => {
            let valid = addr.contains('@')
                && !addr.contains('/')
                && !addr.contains(char::is_whitespace)
                && addr.len() <= 320;
            if valid {
                Ok(format!("users/{addr}"))
            } else {
                Err(AppError::BadRequest("invalid mailbox address".into()))
            }
        }
    }
}

/// Optional `?mailbox=<address>` selecting a shared mailbox (absent = own).
#[derive(Deserialize)]
pub struct MailboxQuery {
    #[serde(default)]
    mailbox: Option<String>,
}

pub async fn graph_get(
    state: &AppState,
    user_id: &str,
    url: &str,
    params: &[(&str, &str)],
) -> AppResult<Value> {
    let token = microsoft::get_valid_token(state, user_id)
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
pub async fn graph_post(state: &AppState, user_id: &str, url: &str, body: &Value) -> AppResult<Value> {
    let token = microsoft::get_valid_token(state, user_id)
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
pub async fn graph_delete(state: &AppState, user_id: &str, url: &str) -> AppResult<()> {
    let token = microsoft::get_valid_token(state, user_id)
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
pub async fn ensure_folder(
    state: &AppState,
    user_id: &str,
    mailbox: Option<&str>,
    name: &str,
) -> AppResult<String> {
    let seg = mailbox_seg(mailbox)?;
    let existing = graph_get(
        state,
        user_id,
        &format!("{GRAPH}/{seg}/mailFolders"),
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
        user_id,
        &format!("{GRAPH}/{seg}/mailFolders"),
        &serde_json::json!({ "displayName": name }),
    )
    .await?;

    created["id"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("created folder missing id")))
}

/// Move a message into the destination folder.
pub async fn move_message(
    state: &AppState,
    user_id: &str,
    mailbox: Option<&str>,
    message_id: &str,
    dest_folder_id: &str,
) -> AppResult<()> {
    let seg = mailbox_seg(mailbox)?;
    graph_post(
        state,
        user_id,
        &format!("{GRAPH}/{seg}/messages/{message_id}/move"),
        &serde_json::json!({ "destinationId": dest_folder_id }),
    )
    .await?;
    Ok(())
}

/// Set the follow-up flag on a message, leaving it in place.
pub async fn flag_message(
    state: &AppState,
    user_id: &str,
    mailbox: Option<&str>,
    message_id: &str,
) -> AppResult<()> {
    set_flag_status(state, user_id, mailbox, message_id, "flagged").await
}

/// Mark a message's follow-up flag as completed (✓ in Outlook).
pub async fn complete_flag(
    state: &AppState,
    user_id: &str,
    mailbox: Option<&str>,
    message_id: &str,
) -> AppResult<()> {
    set_flag_status(state, user_id, mailbox, message_id, "complete").await
}

async fn set_flag_status(
    state: &AppState,
    user_id: &str,
    mailbox: Option<&str>,
    message_id: &str,
    status: &str,
) -> AppResult<()> {
    let seg = mailbox_seg(mailbox)?;
    let token = microsoft::get_valid_token(state, user_id)
        .await
        .map_err(AppError::Internal)?;

    let res = state
        .http_client
        .patch(format!("{GRAPH}/{seg}/messages/{message_id}"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "flag": { "flagStatus": status } }))
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    if !res.status().is_success() {
        let code = res.status();
        return Err(AppError::Internal(anyhow::anyhow!("flag update failed: {code}")));
    }
    Ok(())
}

/// After a reply is sent: mark the original's follow-up flag complete (if
/// flagged) and file it out of the Inbox into "Processed" so the inbox only
/// holds mail still awaiting action. Detached, best-effort — failures log to
/// the Logs window and never affect the send.
async fn file_replied_message(
    state: Arc<AppState>,
    user_id: String,
    mailbox: Option<String>,
    message_id: String,
) {
    // Only file mail that's actually in the Inbox — replying to something in
    // another folder (search results, sorted mail) shouldn't relocate it.
    let _ = process_message(&state, &user_id, mailbox.as_deref(), &message_id, true).await;
}

/// Complete the follow-up flag (when flagged) and move the message to the
/// "Processed" folder. `inbox_only` restricts the move to Inbox mail (the
/// automatic post-reply path); the explicit Done button moves from anywhere
/// except Processed itself.
async fn process_message(
    state: &Arc<AppState>,
    user_id: &str,
    mailbox: Option<&str>,
    message_id: &str,
    inbox_only: bool,
) -> Result<(), ()> {
    let seg = mailbox_seg(mailbox).map_err(|_| ())?;
    let msg = match graph_get(
        state,
        user_id,
        &format!("{GRAPH}/{seg}/messages/{message_id}"),
        &[("$select", "parentFolderId,flag,subject")],
    )
    .await
    {
        Ok(m) => m,
        Err(_) => {
            state
                .log("email", "warn", "filing: couldn't load message")
                .await;
            return Err(());
        }
    };
    let subject = msg["subject"].as_str().unwrap_or("(no subject)").to_string();

    if msg["flag"]["flagStatus"].as_str() == Some("flagged") {
        match complete_flag(state, user_id, mailbox, message_id).await {
            Ok(()) => state.log("email", "info", format!("Flag completed: {subject}")).await,
            Err(_) => state.log("email", "warn", format!("Flag completion failed: {subject}")).await,
        }
    }

    let folder_id = match ensure_folder(state, user_id, mailbox, "Processed").await {
        Ok(id) => id,
        Err(_) => {
            state.log("email", "warn", "filing: Processed folder unavailable").await;
            return Err(());
        }
    };
    let parent = msg["parentFolderId"].as_str().unwrap_or_default();
    if parent == folder_id {
        return Ok(()); // already filed
    }
    if inbox_only {
        let inbox = match graph_get(
            state,
            user_id,
            &format!("{GRAPH}/{seg}/mailFolders/inbox"),
            &[("$select", "id")],
        )
        .await
        {
            Ok(f) => f,
            Err(_) => return Err(()),
        };
        if inbox["id"].as_str() != Some(parent) {
            return Ok(());
        }
    }

    match move_message(state, user_id, mailbox, message_id, &folder_id).await {
        Ok(()) => {
            state.log("email", "info", format!("Filed to Processed: {subject}")).await;
            Ok(())
        }
        Err(_) => {
            state.log("email", "warn", format!("Filing failed: {subject}")).await;
            Err(())
        }
    }
}

// POST /api/email/messages/:id/done — explicit "no response needed": complete
// the flag and file to Processed, regardless of current folder.
pub async fn mark_done(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(message_id): Path<String>,
    Query(q): Query<MailboxQuery>,
) -> AppResult<StatusCode> {
    let user_id = user.id.as_str();
    process_message(&state, user_id, q.mailbox.as_deref(), &message_id, false)
        .await
        .map_err(|()| AppError::Internal(anyhow::anyhow!("filing failed — see Logs")))?;
    Ok(StatusCode::NO_CONTENT)
}

// GET /api/email/folders
pub async fn list_folders(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(q): Query<MailboxQuery>,
) -> AppResult<Json<Value>> {
    let user_id = user.id.as_str();
    let seg = mailbox_seg(q.mailbox.as_deref())?;
    let res = graph_get(
        &state,
        user_id,
        &format!("{GRAPH}/{seg}/mailFolders"),
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
    #[serde(default)]
    mailbox: Option<String>,
}

pub async fn search_messages(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(params): Query<SearchQuery>,
) -> AppResult<Json<Value>> {
    let user_id = user.id.as_str();
    let token = microsoft::get_valid_token(&state, user_id)
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
        let seg = mailbox_seg(params.mailbox.as_deref())?;
        format!(
            "{GRAPH}/{seg}/messages?$search=%22{}%22&$select=id,subject,from,toRecipients,bodyPreview,receivedDateTime,isRead,hasAttachments,flag&$expand=singleValueExtendedProperties($filter=id%20eq%20'Integer%200x1081')&$top={top}",
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
    #[serde(default)]
    mailbox: Option<String>,
}

pub async fn list_messages(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(folder_id): Path<String>,
    Query(q): Query<MessagesQuery>,
) -> AppResult<Json<Value>> {
    let user_id = user.id.as_str();
    let skip = q.skip.unwrap_or(0).to_string();
    let top = q.top.unwrap_or(30).min(50).to_string();
    let seg = mailbox_seg(q.mailbox.as_deref())?;

    let res = graph_get(
        &state,
        user_id,
        &format!("{GRAPH}/{seg}/mailFolders/{folder_id}/messages"),
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
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(message_id): Path<String>,
    Query(q): Query<MailboxQuery>,
) -> AppResult<Json<Value>> {
    let user_id = user.id.as_str();
    let seg = mailbox_seg(q.mailbox.as_deref())?;
    let res = graph_get(
        &state,
        user_id,
        &format!("{GRAPH}/{seg}/messages/{message_id}"),
        &[("$select", "id,subject,from,toRecipients,ccRecipients,body,receivedDateTime,isRead,hasAttachments")],
    )
    .await?;
    Ok(Json(res))
}

// GET /api/email/messages/:id/attachments — metadata only (no bytes).
pub async fn list_attachments(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(message_id): Path<String>,
    Query(q): Query<MailboxQuery>,
) -> AppResult<Json<Value>> {
    let user_id = user.id.as_str();
    let seg = mailbox_seg(q.mailbox.as_deref())?;
    // Only base `attachment` fields: `contentId` lives on the fileAttachment
    // subtype (selecting it 400s) and the default response includes the heavy
    // `contentBytes`. The frontend resolves inline `cid:` references by name
    // (Outlook's `cid:image001.png@…` ↔ name `image001.png`) with a positional
    // fallback over inline attachments for opaque OWA-style content-IDs.
    let res = graph_get(
        &state,
        user_id,
        &format!("{GRAPH}/{seg}/messages/{message_id}/attachments"),
        &[("$select", "id,name,contentType,size,isInline")],
    )
    .await?;
    Ok(Json(res))
}

// GET /api/email/messages/:id/attachments/:att_id/raw — stream the file bytes
// through with their content type so the browser can render or download them.
pub async fn get_attachment_raw(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path((message_id, att_id)): Path<(String, String)>,
    Query(q): Query<MailboxQuery>,
) -> AppResult<Response> {
    let user_id = user.id.as_str();
    let seg = mailbox_seg(q.mailbox.as_deref())?;
    let token = microsoft::get_valid_token(&state, user_id)
        .await
        .map_err(AppError::Internal)?;

    let url = format!("{GRAPH}/{seg}/messages/{message_id}/attachments/{att_id}/$value");
    // Graph throttles bursts of these (HTTP 429) — an email body with many
    // inline images fires them all at once. Honor Retry-After briefly instead
    // of handing the browser a permanently broken image.
    let mut upstream = state
        .http_client
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    for attempt in 0u32..3 {
        if upstream.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
            break;
        }
        let wait = upstream
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1 << attempt)
            .min(5);
        tokio::time::sleep(Duration::from_secs(wait)).await;
        upstream = state
            .http_client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
    }

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
        // Attachment bytes never change for a given id — let the browser cache
        // them so iframe reloads (dark-mode toggle, re-opening) don't refetch
        // every inline image and trip Graph throttling again.
        .header(header::CACHE_CONTROL, "private, max-age=604800, immutable")
        .body(Body::from(bytes))
        .map_err(|e| AppError::Internal(e.into()))
}

// PATCH /api/email/messages/:id/read
pub async fn mark_read(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(message_id): Path<String>,
    Query(q): Query<MailboxQuery>,
) -> AppResult<StatusCode> {
    let user_id = user.id.as_str();
    let seg = mailbox_seg(q.mailbox.as_deref())?;
    let token = microsoft::get_valid_token(&state, user_id)
        .await
        .map_err(AppError::Internal)?;

    let res = state
        .http_client
        .patch(format!("{GRAPH}/{seg}/messages/{message_id}"))
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

// POST /api/email/folders/:id/read-all — mark every unread message in the
// folder as read. Pages unread ids and PATCHes them in Graph $batch chunks
// (20 per batch, the Graph limit); patched messages drop out of the filter,
// so re-running the same query walks the whole folder.
pub async fn mark_all_read(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(folder_id): Path<String>,
    Query(q): Query<MailboxQuery>,
) -> AppResult<Json<Value>> {
    let user_id = user.id.as_str();
    let seg = mailbox_seg(q.mailbox.as_deref())?;
    let mut marked = 0usize;

    // Safety cap: 40 rounds × 100 messages = 4000 per call; the UI reports
    // the count, so a truly huge folder just needs another click.
    for _ in 0..40 {
        let page = graph_get(
            &state,
            user_id,
            &format!("{GRAPH}/{seg}/mailFolders/{folder_id}/messages"),
            &[("$filter", "isRead eq false"), ("$select", "id"), ("$top", "100")],
        )
        .await?;
        let ids: Vec<String> = page["value"]
            .as_array()
            .map(|a| a.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect())
            .unwrap_or_default();
        if ids.is_empty() {
            break;
        }

        for chunk in ids.chunks(20) {
            let requests: Vec<Value> = chunk
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    serde_json::json!({
                        "id": (i + 1).to_string(),
                        "method": "PATCH",
                        "url": format!("/{seg}/messages/{id}"),
                        "headers": { "Content-Type": "application/json" },
                        "body": { "isRead": true },
                    })
                })
                .collect();
            let res = graph_post(
                &state,
                user_id,
                &format!("{GRAPH}/$batch"),
                &serde_json::json!({ "requests": requests }),
            )
            .await?;
            // $batch returns 200 with per-request statuses; count real successes.
            marked += res["responses"]
                .as_array()
                .map(|rs| {
                    rs.iter()
                        .filter(|r| r["status"].as_u64().map(|s| (200..300).contains(&s)).unwrap_or(false))
                        .count()
                })
                .unwrap_or(chunk.len());
        }

        if ids.len() < 100 {
            break;
        }
    }

    if marked > 0 {
        state.log("email", "info", format!("Marked {marked} message(s) as read")).await;
    }
    Ok(Json(serde_json::json!({ "marked": marked })))
}

// POST /api/email/messages/delete — soft-delete: move the given messages to
// Deleted Items (recoverable from Outlook, like deleting in any mail client).
// Batched in Graph $batch chunks of 20, mirroring mark_all_read.
#[derive(Deserialize)]
pub struct DeleteMessagesBody {
    ids: Vec<String>,
    #[serde(default)]
    mailbox: Option<String>,
}

pub async fn delete_messages(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(payload): Json<DeleteMessagesBody>,
) -> AppResult<Json<Value>> {
    let user_id = user.id.as_str();
    let seg = mailbox_seg(payload.mailbox.as_deref())?;
    if payload.ids.is_empty() || payload.ids.len() > 500 {
        return Err(AppError::BadRequest("ids must contain 1–500 message ids".into()));
    }

    let mut deleted = 0usize;
    for chunk in payload.ids.chunks(20) {
        let requests: Vec<Value> = chunk
            .iter()
            .enumerate()
            .map(|(i, id)| {
                serde_json::json!({
                    "id": (i + 1).to_string(),
                    "method": "POST",
                    "url": format!("/{seg}/messages/{id}/move"),
                    "headers": { "Content-Type": "application/json" },
                    "body": { "destinationId": "deleteditems" },
                })
            })
            .collect();
        let res = graph_post(
            &state,
            user_id,
            &format!("{GRAPH}/$batch"),
            &serde_json::json!({ "requests": requests }),
        )
        .await?;
        deleted += res["responses"]
            .as_array()
            .map(|rs| {
                rs.iter()
                    .filter(|r| r["status"].as_u64().map(|s| (200..300).contains(&s)).unwrap_or(false))
                    .count()
            })
            .unwrap_or(chunk.len());
    }

    state.log("email", "info", format!("Deleted {deleted} message(s)")).await;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

// POST /api/email/send
#[derive(Deserialize)]
pub struct SendBody {
    to: Vec<String>,
    #[serde(default)]
    cc: Vec<String>,
    #[serde(default)]
    bcc: Vec<String>,
    subject: Option<String>,
    body: String,
    reply_to_message_id: Option<String>,
    /// "reply" | "replyAll" | "forward" — only "forward" changes the Graph
    /// endpoint; reply/replyAll both send via /reply with explicit recipients.
    #[serde(default)]
    action: Option<String>,
    /// The original AI draft, when this send started from "AI reply" — used to
    /// learn writing-style preferences from the user's edits.
    ai_draft: Option<String>,
    /// Provider that produced the draft (reused for style extraction).
    ai_provider: Option<String>,
    /// Plain text of the message being replied to — context for commitment
    /// detection so short replies ("I'll get this done tonight") resolve.
    reply_context: Option<String>,
    /// Shared mailbox address to send as (absent = the user's own mailbox).
    #[serde(default)]
    mailbox: Option<String>,
}

/// Map plain addresses to Graph recipient objects.
fn recipients(addrs: &[String]) -> Vec<Value> {
    addrs
        .iter()
        .map(|addr| serde_json::json!({ "emailAddress": { "address": addr } }))
        .collect()
}

pub async fn send_email(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(payload): Json<SendBody>,
) -> AppResult<StatusCode> {
    let user_id = user.id.as_str();
    let seg = mailbox_seg(payload.mailbox.as_deref())?;
    let token = microsoft::get_valid_token(&state, user_id)
        .await
        .map_err(AppError::Internal)?;

    // Pulled out before `payload` is partially moved below.
    let mailbox = payload.mailbox.clone();
    let ai_draft = payload.ai_draft.clone().filter(|d| !d.trim().is_empty());
    let ai_provider = payload.ai_provider.clone();
    let sent_body = payload.body.clone();
    // Replies (not forwards) trigger post-send filing of the original.
    let replied_id = payload
        .reply_to_message_id
        .clone()
        .filter(|_| payload.action.as_deref() != Some("forward"));
    let reply_context = payload.reply_context.clone();
    let send_context = format!(
        "{} to {}{}",
        match payload.action.as_deref() {
            Some("forward") => "Forward",
            Some(_) => "Reply",
            None => "Email",
        },
        payload.to.join(", "),
        payload
            .subject
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| format!(": {s}"))
            .unwrap_or_default(),
    );

    let to = recipients(&payload.to);
    let cc = recipients(&payload.cc);
    let bcc = recipients(&payload.bcc);

    if let Some(reply_id) = payload.reply_to_message_id {
        // /forward carries the original message; /reply (used for both reply and
        // reply-all) takes the recipient list the frontend computed. Cc/Bcc go in
        // the `message` parameter for both actions.
        let (url, body) = if payload.action.as_deref() == Some("forward") {
            (
                format!("{GRAPH}/{seg}/messages/{reply_id}/forward"),
                serde_json::json!({
                    "toRecipients": to,
                    "message": { "ccRecipients": cc, "bccRecipients": bcc },
                    "comment": payload.body,
                }),
            )
        } else {
            (
                format!("{GRAPH}/{seg}/messages/{reply_id}/reply"),
                serde_json::json!({
                    "message": { "toRecipients": to, "ccRecipients": cc, "bccRecipients": bcc },
                    "comment": payload.body,
                }),
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
                "toRecipients": to,
                "ccRecipients": cc,
                "bccRecipients": bcc,
            },
        });
        state
            .http_client
            .post(format!("{GRAPH}/{seg}/sendMail"))
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
    }

    // The send succeeded — kick off detached, best-effort follow-ups that must
    // never affect the send result:
    //  - file the replied-to message out of the Inbox (flag → complete)
    //  - style learning, when the send started as an AI draft the user edited
    //  - commitment detection on what was sent ("I'll do X on Saturday…")
    if let Some(id) = replied_id {
        let st = Arc::clone(&state);
        tokio::spawn(file_replied_message(st, user.id.clone(), mailbox.clone(), id));
    }
    if let Some(provider_name) = ai_provider {
        let providers: Vec<ProviderConfig> =
            db::settings::get(&state.db, "providers").await.ok().flatten().unwrap_or_default();
        if let Some(provider) = providers.into_iter().find(|p| p.name == provider_name) {
            if let Some(draft) = ai_draft {
                let st = Arc::clone(&state);
                let prov = provider.clone();
                let sent = sent_body.clone();
                let uid = user.id.clone();
                tokio::spawn(async move {
                    crate::memory::extract_style(&st, &uid, prov, draft, sent).await;
                });
            }
            let st = Arc::clone(&state);
            let uid = user.id.clone();
            tokio::spawn(async move {
                crate::suggestions::detect_commitments(
                    &st,
                    &uid,
                    provider,
                    sent_body,
                    send_context,
                    reply_context,
                )
                .await;
            });
        }
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
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(payload): Json<AiDraftBody>,
) -> AppResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>> {
    let user_id = user.id.as_str();
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

    let mut system = crate::prompts::get(&state.db, "email_draft").await;

    // Apply style lessons learned from the user's past edits to AI drafts.
    let style_memories = db::memories::list(&state.db, user_id, Some("style"), None, 20)
        .await
        .unwrap_or_default();
    if !style_memories.is_empty() {
        system.push_str(
            "\n\nApply these writing-style preferences, learned from how the user edited \
previous drafts:\n",
        );
        for m in &style_memories {
            system.push_str(&format!("- {}\n", m.content));
        }
    }

    let user = format!(
        "Draft a reply to this email.\n\nFrom: {}\nSubject: {}\n\n{}",
        payload.from, payload.subject, payload.body
    );

    let history = vec![
        ChatMessage { role: "system".to_string(), content: Value::String(system) },
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

// POST /api/email/messages/:id/summarize — stream a short summary of the email
// to help the user draft a reply. Inline and ephemeral: no chat session, no DB
// writes, unlike "Ask AI" which seeds a full conversation.
#[derive(Deserialize)]
pub struct SummarizeBody {
    provider: String,
    #[serde(default)]
    mailbox: Option<String>,
}

pub async fn summarize(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(message_id): Path<String>,
    Json(payload): Json<SummarizeBody>,
) -> AppResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>> {
    let user_id = user.id.as_str();
    let providers: Vec<ProviderConfig> = db::settings::get(&state.db, "providers")
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    let provider = providers
        .into_iter()
        .find(|p| p.name == payload.provider)
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("provider '{}' not found", payload.provider)))?;

    let seg = mailbox_seg(payload.mailbox.as_deref())?;
    let msg = graph_get(
        &state,
        user_id,
        &format!("{GRAPH}/{seg}/messages/{message_id}"),
        &[("$select", "subject,from,body")],
    )
    .await?;
    let subject = msg["subject"].as_str().unwrap_or("(no subject)");
    let from_name = msg["from"]["emailAddress"]["name"].as_str().unwrap_or("");
    let from_addr = msg["from"]["emailAddress"]["address"].as_str().unwrap_or("");
    let body_raw = msg["body"]["content"].as_str().unwrap_or("");
    let body_text = if msg["body"]["contentType"]
        .as_str()
        .map(|c| c.eq_ignore_ascii_case("html"))
        .unwrap_or(false)
    {
        html_to_text(body_raw)
    } else {
        body_raw.to_string()
    };

    let system = crate::prompts::get(&state.db, "email_summary").await;
    let user_msg = format!("From: {from_name} <{from_addr}>\nSubject: {subject}\n\n{body_text}");
    let history = vec![
        ChatMessage { role: "system".to_string(), content: Value::String(system) },
        ChatMessage { role: "user".to_string(), content: Value::String(user_msg) },
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
            tracing::error!("summarize stream error: {e}");
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

// ── Ask AI about this email (multimodal chat handoff) ───────────────────────────

#[derive(Deserialize)]
pub struct AdviseBody {
    session_id: String,
    provider: String,
    instruction: Option<String>,
    #[serde(default)]
    mailbox: Option<String>,
}

/// Cap on inline images forwarded to the model, and per-image base64 size.
const MAX_ADVISE_IMAGES: usize = 4;
const MAX_IMAGE_B64: usize = 5_000_000;

// POST /api/email/messages/:id/advise — seed a chat session with the email
// (body text + inline images) and stream the agent's advice.
pub async fn advise(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(message_id): Path<String>,
    Json(payload): Json<AdviseBody>,
) -> AppResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>> {
    let user_id = user.id.as_str();
    let providers: Vec<ProviderConfig> = db::settings::get(&state.db, "providers")
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    let provider = providers
        .into_iter()
        .find(|p| p.name == payload.provider)
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("provider '{}' not found", payload.provider)))?;

    let seg = mailbox_seg(payload.mailbox.as_deref())?;
    // Message metadata + body.
    let msg = graph_get(
        &state,
        user_id,
        &format!("{GRAPH}/{seg}/messages/{message_id}"),
        &[("$select", "subject,from,body")],
    )
    .await?;
    let subject = msg["subject"].as_str().unwrap_or("(no subject)");
    let from_name = msg["from"]["emailAddress"]["name"].as_str().unwrap_or("");
    let from_addr = msg["from"]["emailAddress"]["address"].as_str().unwrap_or("");
    let body_raw = msg["body"]["content"].as_str().unwrap_or("");
    let body_text = if msg["body"]["contentType"].as_str().map(|c| c.eq_ignore_ascii_case("html")).unwrap_or(false) {
        html_to_text(body_raw)
    } else {
        body_raw.to_string()
    };

    // Inline images → base64 (Graph's contentBytes is already base64).
    let attachments = graph_get(
        &state,
        user_id,
        &format!("{GRAPH}/{seg}/messages/{message_id}/attachments"),
        &[("$select", "id,name,contentType,size,isInline")],
    )
    .await?;
    let mut images: Vec<Value> = Vec::new();
    if let Some(arr) = attachments["value"].as_array() {
        for att in arr {
            if images.len() >= MAX_ADVISE_IMAGES {
                break;
            }
            let ctype = att["contentType"].as_str().unwrap_or("");
            if !ctype.starts_with("image/") {
                continue;
            }
            let Some(att_id) = att["id"].as_str() else { continue };
            // No $select: contentBytes lives on the fileAttachment subtype (selecting
            // it 400s); the single-attachment GET returns it by default.
            let full = graph_get(
                &state,
                user_id,
                &format!("{GRAPH}/{seg}/messages/{message_id}/attachments/{att_id}"),
                &[],
            )
            .await?;
            if let Some(b64) = full["contentBytes"].as_str() {
                if b64.len() <= MAX_IMAGE_B64 {
                    images.push(serde_json::json!({ "mime": ctype, "b64": b64 }));
                }
            }
        }
    }

    let instruction = match payload.instruction.filter(|s| !s.trim().is_empty()) {
        Some(i) => i,
        None => crate::prompts::get(&state.db, "email_advise").await,
    };
    let text = format!(
        "{instruction}\n\n--- Email ---\nFrom: {from_name} <{from_addr}>\nSubject: {subject}\n\n{body_text}"
    );

    let content = serde_json::json!({ "type": "multimodal", "text": text, "images": images });
    db::messages::insert(
        &state.db,
        &payload.session_id,
        "user",
        &serde_json::to_string(&content).unwrap_or_default(),
        None,
        None,
    )
    .await
    .map_err(AppError::Internal)?;
    db::sessions::touch(&state.db, &payload.session_id)
        .await
        .map_err(AppError::Internal)?;

    // Stream the agent's reply, mirroring routes::chat::stream.
    let (tx, rx) = mpsc::channel::<AgentEvent>(64);
    tokio::spawn(agent::run_turn(
        Arc::clone(&state),
        user.id.clone(),
        payload.session_id,
        provider,
        tx,
    ));

    let event_stream = stream::unfold(rx, |mut rx| async move {
        let event = match rx.recv().await? {
            AgentEvent::Token(text) => Event::default()
                .data(serde_json::json!({ "type": "token", "text": text }).to_string()),
            AgentEvent::ToolCall { name } => Event::default()
                .data(serde_json::json!({ "type": "tool", "name": name }).to_string()),
            AgentEvent::Done => Event::default().data(serde_json::json!({ "type": "done" }).to_string()),
            AgentEvent::AwaitingApproval { action_id, tool_name, tool_args } => Event::default().data(
                serde_json::json!({
                    "type": "awaiting_approval",
                    "action_id": action_id,
                    "tool_name": tool_name,
                    "tool_args": tool_args,
                })
                .to_string(),
            ),
        };
        Some((Ok::<Event, Infallible>(event), rx))
    });

    Ok(Sse::new(event_stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(5))))
}

/// Minimal HTML→text: block tags to newlines, drop script/style, strip tags,
/// decode a few entities, tidy whitespace. Good enough to give the model context.
fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    for tag in ["</p>", "<br>", "<br/>", "<br />", "</div>", "</tr>", "</li>", "</h1>", "</h2>", "</h3>"] {
        s = s.replace(tag, "\n");
    }
    // Drop <script>/<style> blocks.
    for (open, close) in [("<script", "</script>"), ("<style", "</style>")] {
        loop {
            let lower = s.to_lowercase();
            match (lower.find(open), lower.find(close)) {
                (Some(a), Some(b)) if b > a => s.replace_range(a..b + close.len(), " "),
                _ => break,
            }
        }
    }
    // Strip remaining tags.
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    // Collapse runs of blank lines / trailing spaces.
    out.lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .split("\n\n\n")
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
}

// ── Auto-categorizer config / manual run ───────────────────────────────────────

// GET /api/email/categorizer
pub async fn get_categorizer(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<crate::categorizer::CategorizerConfig>> {
    let user_id = user.id.as_str();
    let cfg = crate::categorizer::get_config(&state.db, user_id)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(cfg))
}

// PUT /api/email/categorizer
pub async fn put_categorizer(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(cfg): Json<crate::categorizer::CategorizerConfig>,
) -> AppResult<Json<crate::categorizer::CategorizerConfig>> {
    let user_id = user.id.as_str();
    crate::categorizer::set_config(&state.db, user_id, &cfg)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(cfg))
}

// POST /api/email/categorizer/run?mailbox=… — run one mailbox's sort now.
pub async fn run_categorizer(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(q): Query<MailboxQuery>,
) -> AppResult<Json<crate::categorizer::RunSummary>> {
    let user_id = user.id.as_str();
    let mailbox = q.mailbox.unwrap_or_default();
    // Use the provider configured for that mailbox's task (empty → first available).
    let cfg = crate::categorizer::get_config(&state.db, user_id)
        .await
        .map_err(AppError::Internal)?;
    let task = cfg.tasks.iter().find(|t| t.mailbox == mailbox);
    let provider = task.map(|t| t.provider.clone()).unwrap_or_default();
    let instructions = task.map(|t| t.instructions.clone()).unwrap_or_default();
    let summary = crate::categorizer::run_mailbox(&state, user_id, &mailbox, &provider, &instructions)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(summary))
}
