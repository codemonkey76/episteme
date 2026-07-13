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
use crate::integrations::graph::{
    self, graph_delete, graph_get, graph_patch, graph_post, html_to_text, prepend_html,
    recipients, GRAPH,
};
use crate::integrations::microsoft;
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig, StreamChunk};
use crate::state::AppState;
use tokio::sync::mpsc;

/// Route-layer wrapper preserving the 400 (not 500) on a malformed address.
pub fn mailbox_seg(mailbox: Option<&str>) -> AppResult<String> {
    graph::mailbox_seg(mailbox).map_err(|e| AppError::BadRequest(e.to_string()))
}

/// `?account=<id>` selects which connected Microsoft 365 instance (absent =
/// default), and `?mailbox=<address>` selects a shared mailbox under it (absent
/// = that account's own mailbox).
#[derive(Deserialize)]
pub struct MailboxQuery {
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    mailbox: Option<String>,
}

/// Return the id of the mail folder named `name`, creating it under the mailbox
/// root if it doesn't already exist. Matching is case-insensitive on displayName.
pub async fn ensure_folder(
    state: &AppState,
    user_id: &str,
    account: Option<&str>,
    mailbox: Option<&str>,
    name: &str,
) -> AppResult<String> {
    let seg = mailbox_seg(mailbox)?;
    let existing = graph_get(
        state,
        user_id,
        account,
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
        account,
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
    account: Option<&str>,
    mailbox: Option<&str>,
    message_id: &str,
    dest_folder_id: &str,
) -> AppResult<()> {
    let seg = mailbox_seg(mailbox)?;
    graph_post(
        state,
        user_id,
        account,
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
    account: Option<&str>,
    mailbox: Option<&str>,
    message_id: &str,
) -> AppResult<()> {
    set_flag_status(state, user_id, account, mailbox, message_id, "flagged").await
}

/// Mark a message's follow-up flag as completed (✓ in Outlook).
pub async fn complete_flag(
    state: &AppState,
    user_id: &str,
    account: Option<&str>,
    mailbox: Option<&str>,
    message_id: &str,
) -> AppResult<()> {
    set_flag_status(state, user_id, account, mailbox, message_id, "complete").await
}

async fn set_flag_status(
    state: &AppState,
    user_id: &str,
    account: Option<&str>,
    mailbox: Option<&str>,
    message_id: &str,
    status: &str,
) -> AppResult<()> {
    let seg = mailbox_seg(mailbox)?;
    let token = microsoft::get_valid_token(state, user_id, account)
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
    account: Option<String>,
    mailbox: Option<String>,
    message_id: String,
) {
    // Only file mail that's actually in the Inbox — replying to something in
    // another folder (search results, sorted mail) shouldn't relocate it.
    let _ = process_message(&state, &user_id, account.as_deref(), mailbox.as_deref(), &message_id, true).await;
}

/// Complete the follow-up flag (when flagged) and move the message to the
/// "Processed" folder. `inbox_only` restricts the move to Inbox mail (the
/// automatic post-reply path); the explicit Done button moves from anywhere
/// except Processed itself.
async fn process_message(
    state: &Arc<AppState>,
    user_id: &str,
    account: Option<&str>,
    mailbox: Option<&str>,
    message_id: &str,
    inbox_only: bool,
) -> Result<(), ()> {
    let seg = mailbox_seg(mailbox).map_err(|_| ())?;
    let msg = match graph_get(
        state,
        user_id,
        account,
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
        match complete_flag(state, user_id, account, mailbox, message_id).await {
            Ok(()) => state.log("email", "info", format!("Flag completed: {subject}")).await,
            Err(_) => state.log("email", "warn", format!("Flag completion failed: {subject}")).await,
        }
    }

    let folder_id = match ensure_folder(state, user_id, account, mailbox, "Processed").await {
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
            account,
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

    match move_message(state, user_id, account, mailbox, message_id, &folder_id).await {
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
    process_message(&state, user_id, q.account.as_deref(), q.mailbox.as_deref(), &message_id, false)
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
        q.account.as_deref(),
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
    account: Option<String>,
    #[serde(default)]
    mailbox: Option<String>,
}

pub async fn search_messages(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(params): Query<SearchQuery>,
) -> AppResult<Json<Value>> {
    let user_id = user.id.as_str();
    let token = microsoft::get_valid_token(&state, user_id, params.account.as_deref())
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
            "{GRAPH}/{seg}/messages?$search=%22{}%22&$select=id,subject,from,toRecipients,bodyPreview,receivedDateTime,isRead,hasAttachments,isDraft,flag&$expand=singleValueExtendedProperties($filter=id%20eq%20'Integer%200x1081')&$top={top}",
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
    account: Option<String>,
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
        q.account.as_deref(),
        &format!("{GRAPH}/{seg}/mailFolders/{folder_id}/messages"),
        &[
            ("$select", "id,subject,from,toRecipients,bodyPreview,receivedDateTime,isRead,hasAttachments,isDraft,flag"),
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
        q.account.as_deref(),
        &format!("{GRAPH}/{seg}/messages/{message_id}"),
        &[("$select", "id,subject,from,toRecipients,ccRecipients,bccRecipients,body,receivedDateTime,isRead,hasAttachments,isDraft")],
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
        q.account.as_deref(),
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
    let token = microsoft::get_valid_token(&state, user_id, q.account.as_deref())
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
    let token = microsoft::get_valid_token(&state, user_id, q.account.as_deref())
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
            q.account.as_deref(),
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
                q.account.as_deref(),
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
    account: Option<String>,
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
            payload.account.as_deref(),
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

// POST /api/email/messages/move — move the given messages into `destination`
// (a folder id, or a well-known name like "archive"). Same $batch chunking as
// delete; powers drag-and-drop onto a folder.
#[derive(Deserialize)]
pub struct MoveMessagesBody {
    ids: Vec<String>,
    destination: String,
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    mailbox: Option<String>,
}

pub async fn move_messages(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(payload): Json<MoveMessagesBody>,
) -> AppResult<Json<Value>> {
    let user_id = user.id.as_str();
    let seg = mailbox_seg(payload.mailbox.as_deref())?;
    if payload.ids.is_empty() || payload.ids.len() > 500 {
        return Err(AppError::BadRequest("ids must contain 1–500 message ids".into()));
    }
    let dest = payload.destination.trim();
    if dest.is_empty() {
        return Err(AppError::BadRequest("destination folder is required".into()));
    }

    let mut moved = 0usize;
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
                    "body": { "destinationId": dest },
                })
            })
            .collect();
        let res = graph_post(
            &state,
            user_id,
            payload.account.as_deref(),
            &format!("{GRAPH}/$batch"),
            &serde_json::json!({ "requests": requests }),
        )
        .await?;
        moved += res["responses"]
            .as_array()
            .map(|rs| {
                rs.iter()
                    .filter(|r| r["status"].as_u64().map(|s| (200..300).contains(&s)).unwrap_or(false))
                    .count()
            })
            .unwrap_or(chunk.len());
    }

    state.log("email", "info", format!("Moved {moved} message(s)")).await;
    Ok(Json(serde_json::json!({ "moved": moved })))
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
    /// HTML body of the user's message (the composer is rich text). For
    /// replies/forwards the quoted history is NOT included here — Graph's
    /// createReply/createForward drafts carry it, and we prepend this above it.
    body: String,
    reply_to_message_id: Option<String>,
    /// An existing draft being edited: its content/recipients are overwritten
    /// and the same message is sent, preserving any attachments already on it.
    #[serde(default)]
    draft_id: Option<String>,
    /// "reply" | "replyAll" | "forward" — selects the Graph draft endpoint
    /// (createReply / createReplyAll / createForward).
    #[serde(default)]
    action: Option<String>,
    /// Attachments to upload onto the draft before sending. Inline ones carry a
    /// contentId referenced from the body as `cid:<contentId>`.
    #[serde(default)]
    attachments: Vec<AttachmentUpload>,
    /// The original AI draft, when this send started from "AI reply" — used to
    /// learn writing-style preferences from the user's edits.
    ai_draft: Option<String>,
    /// Provider that produced the draft (reused for style extraction).
    ai_provider: Option<String>,
    /// Plain text of the message being replied to — context for commitment
    /// detection so short replies ("I'll get this done tonight") resolve.
    reply_context: Option<String>,
    /// Which connected Microsoft 365 account to send from (absent = default).
    #[serde(default)]
    account: Option<String>,
    /// Shared mailbox address to send as (absent = the user's own mailbox).
    #[serde(default)]
    mailbox: Option<String>,
}

#[derive(Deserialize)]
pub struct AttachmentUpload {
    name: String,
    content_type: String,
    /// Raw file bytes, base64-encoded (no `data:` prefix).
    content_bytes: String,
    #[serde(default)]
    is_inline: bool,
    /// Content-ID for inline images (the body references `cid:<contentId>`).
    #[serde(default)]
    content_id: Option<String>,
}

/// Graph caps direct fileAttachment POSTs at ~3 MB; anything bigger goes
/// through an upload session.
const DIRECT_ATTACHMENT_MAX: usize = 3 * 1024 * 1024;
/// Upload-session chunk size — Graph requires a multiple of 320 KiB.
const UPLOAD_CHUNK: usize = 10 * 320 * 1024;

/// Attach one file to a draft: small files inline in a single POST, large ones
/// via createUploadSession + chunked PUTs to the pre-authenticated uploadUrl.
async fn add_attachment(
    state: &AppState,
    user_id: &str,
    account: Option<&str>,
    seg: &str,
    draft_id: &str,
    att: &AttachmentUpload,
) -> AppResult<()> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(att.content_bytes.trim())
        .map_err(|e| AppError::BadRequest(format!("attachment '{}' is not valid base64: {e}", att.name)))?;

    if bytes.len() < DIRECT_ATTACHMENT_MAX {
        let mut body = serde_json::json!({
            "@odata.type": "#microsoft.graph.fileAttachment",
            "name": att.name,
            "contentType": att.content_type,
            "contentBytes": att.content_bytes,
            "isInline": att.is_inline,
        });
        if let Some(cid) = &att.content_id {
            body["contentId"] = Value::String(cid.clone());
        }
        graph_post(state, user_id, account, &format!("{GRAPH}/{seg}/messages/{draft_id}/attachments"), &body)
            .await?;
        return Ok(());
    }

    let mut item = serde_json::json!({
        "attachmentType": "file",
        "name": att.name,
        "contentType": att.content_type,
        "size": bytes.len(),
        "isInline": att.is_inline,
    });
    if let Some(cid) = &att.content_id {
        item["contentId"] = Value::String(cid.clone());
    }
    let session = graph_post(
        state,
        user_id,
        account,
        &format!("{GRAPH}/{seg}/messages/{draft_id}/attachments/createUploadSession"),
        &serde_json::json!({ "AttachmentItem": item }),
    )
    .await?;
    let upload_url = session["uploadUrl"]
        .as_str()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("upload session missing uploadUrl")))?;

    let total = bytes.len();
    let mut start = 0usize;
    while start < total {
        let end = (start + UPLOAD_CHUNK).min(total);
        // The uploadUrl is pre-authenticated — no bearer token. Retry transient
        // failures (429 / 5xx / transport) a couple of times per chunk.
        let mut attempt = 0u32;
        loop {
            let res = state
                .http_client
                .put(upload_url)
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", start, end - 1, total))
                .body(bytes[start..end].to_vec())
                .send()
                .await;
            match res {
                Ok(r) if r.status().is_success() => break,
                Ok(r) if attempt < 2
                    && (r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                        || r.status().is_server_error()) =>
                {
                    tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                }
                Ok(r) => {
                    let status = r.status();
                    let detail = r.text().await.unwrap_or_default();
                    return Err(AppError::Internal(anyhow::anyhow!(
                        "attachment '{}' chunk upload failed: {status} {detail}",
                        att.name
                    )));
                }
                Err(e) if attempt < 2 => {
                    tracing::warn!("attachment chunk transport error (retrying): {e}");
                    tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                }
                Err(e) => return Err(AppError::Internal(e.into())),
            }
            attempt += 1;
        }
        start = end;
    }
    Ok(())
}

pub async fn send_email(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(payload): Json<SendBody>,
) -> AppResult<StatusCode> {
    let user_id = user.id.as_str();
    let seg = mailbox_seg(payload.mailbox.as_deref())?;
    let account = payload.account.clone();

    // Pulled out before `payload` is partially moved below.
    let mailbox = payload.mailbox.clone();
    let ai_draft = payload.ai_draft.clone().filter(|d| !d.trim().is_empty());
    let ai_provider = payload.ai_provider.clone();
    // Post-send analysis (style learning, commitments) works on plain text.
    let sent_text = html_to_text(&payload.body);
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

    // Draft-based send: create a draft (createReply/createReplyAll/createForward
    // keep the quoted history and threading), patch in the user's HTML and
    // recipients, upload attachments, then send. One flow for every case so
    // HTML bodies, inline images and large attachments all work uniformly.
    let draft_id: String = if let Some(existing) =
        payload.draft_id.as_deref().filter(|s| !s.trim().is_empty())
    {
        // Editing an existing draft: overwrite its content and recipients in
        // place and send the SAME message, so attachments already on it survive.
        let mut patch = serde_json::json!({
            "body": { "contentType": "html", "content": payload.body },
            "toRecipients": to,
            "ccRecipients": cc,
            "bccRecipients": bcc,
        });
        if let Some(s) = payload.subject.as_deref() {
            patch["subject"] = Value::String(s.to_string());
        }
        graph_patch(&state, user_id, account.as_deref(), &format!("{GRAPH}/{seg}/messages/{existing}"), &patch).await?;
        existing.to_string()
    } else if let Some(reply_id) = &payload.reply_to_message_id {
        let verb = match payload.action.as_deref() {
            Some("forward") => "createForward",
            Some("replyAll") => "createReplyAll",
            _ => "createReply",
        };
        let draft = graph_post(
            &state,
            user_id,
            account.as_deref(),
            &format!("{GRAPH}/{seg}/messages/{reply_id}/{verb}"),
            &serde_json::json!({}),
        )
        .await?;
        // The user's message goes above the quoted history Graph put in the draft.
        let quoted = draft["body"]["content"].as_str().unwrap_or_default();
        let merged = prepend_html(&payload.body, quoted);
        let mut patch = serde_json::json!({
            "body": { "contentType": "html", "content": merged },
            "toRecipients": to,
            "ccRecipients": cc,
            "bccRecipients": bcc,
        });
        // Keep the draft's own Re:/Fwd: subject unless the user edited it.
        if let Some(s) = payload.subject.as_deref().filter(|s| !s.trim().is_empty()) {
            patch["subject"] = Value::String(s.to_string());
        }
        let draft_id = draft["id"]
            .as_str()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("draft missing id")))?;
        graph_patch(&state, user_id, account.as_deref(), &format!("{GRAPH}/{seg}/messages/{draft_id}"), &patch).await?;
        draft_id.to_string()
    } else {
        let draft = graph_post(
            &state,
            user_id,
            account.as_deref(),
            &format!("{GRAPH}/{seg}/messages"),
            &serde_json::json!({
                "subject": payload.subject.clone().unwrap_or_default(),
                "body": { "contentType": "html", "content": payload.body },
                "toRecipients": to,
                "ccRecipients": cc,
                "bccRecipients": bcc,
            }),
        )
        .await?;
        draft["id"]
            .as_str()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("draft missing id")))?
            .to_string()
    };

    // Attachments, then send. On failure, delete the draft (best-effort) so
    // half-built messages don't pile up in Drafts.
    let result: AppResult<()> = async {
        for att in &payload.attachments {
            add_attachment(&state, user_id, account.as_deref(), &seg, &draft_id, att).await?;
        }
        graph_post(
            &state,
            user_id,
            account.as_deref(),
            &format!("{GRAPH}/{seg}/messages/{draft_id}/send"),
            &serde_json::json!({}),
        )
        .await?;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        let _ = graph_delete(&state, user_id, account.as_deref(), &format!("{GRAPH}/{seg}/messages/{draft_id}")).await;
        return Err(e);
    }

    // The send succeeded — kick off detached, best-effort follow-ups that must
    // never affect the send result:
    //  - file the replied-to message out of the Inbox (flag → complete)
    //  - style learning, when the send started as an AI draft the user edited
    //  - commitment detection on what was sent ("I'll do X on Saturday…")
    if let Some(id) = replied_id {
        let st = Arc::clone(&state);
        tokio::spawn(file_replied_message(st, user.id.clone(), account.clone(), mailbox.clone(), id));
    }
    if let Some(provider_name) = ai_provider {
        let providers: Vec<ProviderConfig> =
            db::settings::get(&state.db, "providers").await.ok().flatten().unwrap_or_default();
        if let Some(provider) = providers.into_iter().find(|p| p.name == provider_name) {
            if let Some(draft) = ai_draft {
                let st = Arc::clone(&state);
                let prov = provider.clone();
                let sent = sent_text.clone();
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
                    sent_text,
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
    /// Optional short steer from the user on how to reply (e.g. "decline
    /// politely", "tell them it's still ongoing"). Empty = default behaviour.
    #[serde(default)]
    instruction: Option<String>,
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

    // A short user steer, when given, directs the tone/content of the reply.
    let task = match payload.instruction.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(steer) => format!(
            "Draft a reply to this email. The user wants the reply to: {steer}. \
Honour that instruction while keeping the reply appropriate to the email."
        ),
        None => "Draft a reply to this email.".to_string(),
    };
    let user = format!(
        "{task}\n\nFrom: {}\nSubject: {}\n\n{}",
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
    let pool = state.db.clone();
    let uid = user_id.to_string();
    let prov_meta = provider.clone();
    tokio::spawn(async move {
        let (tx, mut rx) = mpsc::channel::<StreamChunk>(64);
        let model_task =
            tokio::spawn(async move { ModelRouter::stream(&provider, history, Vec::new(), false, tx).await });

        let mut used = None;
        while let Some(chunk) = rx.recv().await {
            if chunk.usage.is_some() {
                used = chunk.usage;
            }
            let data = if chunk.done {
                serde_json::json!({ "type": "done" }).to_string()
            } else {
                serde_json::json!({ "type": "token", "text": chunk.text }).to_string()
            };
            if ev_tx.send(data).await.is_err() {
                return; // client disconnected
            }
        }
        db::usage::record(&pool, &uid, &prov_meta, "email-ai", used).await;

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

// POST /api/email/improve — rewrite the user's own draft reply, streaming the
// improved text back so they can accept it or revert to what they had.
#[derive(Deserialize)]
pub struct ImproveBody {
    provider: String,
    /// The user's current draft text (plain text, signature stripped client-side).
    draft: String,
    /// Context of the email being replied to (empty for a fresh compose).
    #[serde(default)]
    from: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    body: String,
}

pub async fn improve(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(payload): Json<ImproveBody>,
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

    let mut system = crate::prompts::get(&state.db, "email_improve").await;

    // Apply the same style lessons learned from the user's past edits to drafts.
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

    // Give the model the original email as context (when replying), then the
    // draft to improve.
    let context = if payload.body.trim().is_empty() {
        String::new()
    } else {
        format!(
            "This reply is to the following email — keep the improved draft on-topic:\n\n\
From: {}\nSubject: {}\n\n{}\n\n---\n\n",
            payload.from, payload.subject, payload.body
        )
    };
    let user = format!("{context}Improve this draft reply:\n\n{}", payload.draft);

    let history = vec![
        ChatMessage { role: "system".to_string(), content: Value::String(system) },
        ChatMessage { role: "user".to_string(), content: Value::String(user) },
    ];

    let (ev_tx, ev_rx) = mpsc::channel::<String>(64);
    let pool = state.db.clone();
    let uid = user_id.to_string();
    let prov_meta = provider.clone();
    tokio::spawn(async move {
        let (tx, mut rx) = mpsc::channel::<StreamChunk>(64);
        let model_task =
            tokio::spawn(async move { ModelRouter::stream(&provider, history, Vec::new(), false, tx).await });

        let mut used = None;
        while let Some(chunk) = rx.recv().await {
            if chunk.usage.is_some() {
                used = chunk.usage;
            }
            let data = if chunk.done {
                serde_json::json!({ "type": "done" }).to_string()
            } else {
                serde_json::json!({ "type": "token", "text": chunk.text }).to_string()
            };
            if ev_tx.send(data).await.is_err() {
                return; // client disconnected
            }
        }
        db::usage::record(&pool, &uid, &prov_meta, "email-ai", used).await;

        if let Ok(Err(e)) = model_task.await {
            tracing::error!("improve stream error: {e}");
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
    account: Option<String>,
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
        payload.account.as_deref(),
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
    let pool = state.db.clone();
    let uid = user.id.clone();
    let prov_meta = provider.clone();
    tokio::spawn(async move {
        let (tx, mut rx) = mpsc::channel::<StreamChunk>(64);
        let model_task =
            tokio::spawn(async move { ModelRouter::stream(&provider, history, Vec::new(), false, tx).await });
        let mut used = None;
        while let Some(chunk) = rx.recv().await {
            if chunk.usage.is_some() {
                used = chunk.usage;
            }
            let data = if chunk.done {
                serde_json::json!({ "type": "done" }).to_string()
            } else {
                serde_json::json!({ "type": "token", "text": chunk.text }).to_string()
            };
            if ev_tx.send(data).await.is_err() {
                return; // client disconnected
            }
        }
        db::usage::record(&pool, &uid, &prov_meta, "email-ai", used).await;
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

// POST /api/email/messages/:id/ticket/preview — match the sender to a helpdesk
// contact and have the AI extract subject/description/priority, but DON'T
// create the ticket: the result is returned for the user to review and edit
// first (ticket_create does the actual creation).
#[derive(Deserialize)]
pub struct TicketFromEmailBody {
    provider: String,
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    mailbox: Option<String>,
}

pub async fn ticket_preview(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(message_id): Path<String>,
    Json(payload): Json<TicketFromEmailBody>,
) -> AppResult<Json<Value>> {
    let user_id = user.id.as_str();
    tracing::info!("ticket_from_email: start (mailbox={:?})", payload.mailbox);

    // The email being converted.
    let seg = mailbox_seg(payload.mailbox.as_deref())?;
    let msg = graph_get(
        &state,
        user_id,
        payload.account.as_deref(),
        &format!("{GRAPH}/{seg}/messages/{message_id}"),
        &[("$select", "subject,from,body")],
    )
    .await?;
    tracing::info!("ticket_from_email: email loaded, fetching helpdesk clients");
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

    // Match the sender to a helpdesk contact → client_id + user_id. The
    // contact_email filter does the lookup server-side — the bare /clients
    // list is capped at 20, so scanning it would miss contacts on clients
    // past the first page.
    let clients = crate::integrations::helpdesk::request(
        &state,
        user_id,
        None,
        reqwest::Method::GET,
        &format!("/clients?contact_email={}", urlencoding::encode(from_addr)),
        None,
    )
    .await
    .map_err(AppError::Internal)?;
    let mut matched: Option<(i64, i64, String)> = None;
    if let Some(arr) = clients["data"].as_array() {
        'outer: for c in arr {
            let Some(users) = c["users"].as_array() else { continue };
            for u in users {
                if u["email"].as_str().map(|e| e.eq_ignore_ascii_case(from_addr)).unwrap_or(false) {
                    if let (Some(cid), Some(uid)) = (c["id"].as_i64(), u["id"].as_i64()) {
                        matched = Some((cid, uid, c["name"].as_str().unwrap_or("?").to_string()));
                        break 'outer;
                    }
                }
            }
        }
    }
    let Some((client_id, requester_id, client_name)) = matched else {
        return Err(AppError::BadRequest(format!(
            "no helpdesk contact with the address {from_addr} — add them to a client first"
        )));
    };
    tracing::info!("ticket_from_email: matched {from_addr} → client {client_name}");

    // AI: extract the request as ticket subject/description/priority.
    let providers: Vec<ProviderConfig> = db::settings::get(&state.db, "providers")
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    let provider = providers
        .into_iter()
        .find(|p| p.name == payload.provider)
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("provider '{}' not found", payload.provider)))?;
    let system = crate::prompts::get(&state.db, "email_ticket").await;
    let user_msg = format!("From: {from_name} <{from_addr}>\nSubject: {subject}\n\n{body_text}");
    tracing::info!("ticket_from_email: extracting via provider '{}'", provider.name);
    // The model call is the one hop without its own transport timeout — cap it
    // so a wedged provider yields an error instead of an eternal "Creating…".
    let (raw, used) = tokio::time::timeout(
        Duration::from_secs(120),
        ModelRouter::complete_with_usage(
            &provider,
            vec![
                ChatMessage { role: "system".to_string(), content: Value::String(system) },
                ChatMessage { role: "user".to_string(), content: Value::String(user_msg) },
            ],
        ),
    )
    .await
    .map_err(|_| AppError::Internal(anyhow::anyhow!("AI provider timed out after 120s")))?
    .map_err(AppError::Internal)?;
    db::usage::record(&state.db, user_id, &provider, "email-ai", used).await;
    tracing::info!("ticket_from_email: model returned {} chars", raw.len());
    // Tolerate code fences / prose around the JSON object.
    let start = raw.find('{').ok_or_else(|| AppError::Internal(anyhow::anyhow!("no JSON in model output")))?;
    let end = raw.rfind('}').ok_or_else(|| AppError::Internal(anyhow::anyhow!("no JSON in model output")))?;
    let extracted: Value = serde_json::from_str(&raw[start..=end])
        .map_err(|e| AppError::Internal(anyhow::anyhow!("model output unparseable: {e}")))?;
    let t_subject = extracted["subject"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or(subject);
    let t_description = extracted["description"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&body_text);
    let t_priority = match extracted["priority"].as_str() {
        Some(p @ ("low" | "medium" | "high" | "critical")) => p,
        _ => "medium",
    };

    // How many image attachments will ride along — shown in the review panel.
    let attachment_count = image_attachment_count(&state, user_id, payload.account.as_deref(), &seg, &message_id).await;

    // Return the draft for review — creation happens in ticket_create once the
    // user has (optionally) edited it.
    Ok(Json(serde_json::json!({
        "client_id": client_id,
        "requester_id": requester_id,
        "client": client_name,
        "subject": t_subject,
        "description": t_description,
        "priority": t_priority,
        "attachment_count": attachment_count,
    })))
}

/// Count image attachments on a message (cheap — metadata only).
async fn image_attachment_count(state: &AppState, user_id: &str, account: Option<&str>, seg: &str, message_id: &str) -> usize {
    let res = graph_get(
        state,
        user_id,
        account,
        &format!("{GRAPH}/{seg}/messages/{message_id}/attachments"),
        &[("$select", "contentType")],
    )
    .await
    .unwrap_or(Value::Null);
    res["value"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|a| a["contentType"].as_str().map(|c| c.starts_with("image/")).unwrap_or(false))
                .count()
        })
        .unwrap_or(0)
}

/// Fetch the message's image attachments (file attachments with an `image/*`
/// content type) as (filename, content_type, bytes) — one Graph call, bytes
/// included as base64. Capped to keep the helpdesk upload sane.
async fn fetch_image_attachments(
    state: &AppState,
    user_id: &str,
    account: Option<&str>,
    seg: &str,
    message_id: &str,
) -> Vec<(String, String, Vec<u8>)> {
    use base64::Engine;
    const MAX_FILES: usize = 20;
    const MAX_BYTES: usize = 20 * 1024 * 1024;
    let res = match graph_get(
        state,
        user_id,
        account,
        &format!("{GRAPH}/{seg}/messages/{message_id}/attachments"),
        &[],
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("ticket attachments: list failed: {e}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for a in res["value"].as_array().into_iter().flatten() {
        if out.len() >= MAX_FILES {
            break;
        }
        let ct = a["contentType"].as_str().unwrap_or("");
        let is_file = a["@odata.type"].as_str() == Some("#microsoft.graph.fileAttachment");
        if !is_file || !ct.starts_with("image/") {
            continue;
        }
        let Some(b64) = a["contentBytes"].as_str() else { continue };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else { continue };
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            continue;
        }
        let name = a["name"].as_str().filter(|s| !s.is_empty()).unwrap_or("image").to_string();
        out.push((name, ct.to_string(), bytes));
    }
    out
}

// POST /api/email/tickets — create a helpdesk ticket from the reviewed draft.
// The client/requester ids come from the preview; the helpdesk API validates
// they belong to the connected instance, so a tampered id can't reach another.
#[derive(Deserialize)]
pub struct CreateTicketBody {
    client_id: i64,
    requester_id: i64,
    subject: String,
    description: String,
    priority: String,
    /// Carried through from the preview for the success message/log only.
    #[serde(default)]
    client: Option<String>,
    /// The source email — its image attachments are forwarded onto the ticket.
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    mailbox: Option<String>,
}

pub async fn ticket_create(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<CreateTicketBody>,
) -> AppResult<Json<Value>> {
    let user_id = user.id.as_str();
    let subject = body.subject.trim();
    if subject.is_empty() {
        return Err(AppError::BadRequest("ticket subject is required".into()));
    }
    let priority = match body.priority.as_str() {
        p @ ("low" | "medium" | "high" | "critical") => p,
        _ => "medium",
    };

    // Forward the email's image attachments onto the ticket (best-effort: a
    // fetch failure still creates the ticket, just without the images).
    let images = if let Some(mid) = body.message_id.as_deref().filter(|s| !s.is_empty()) {
        let seg = mailbox_seg(body.mailbox.as_deref())?;
        fetch_image_attachments(&state, user_id, body.account.as_deref(), &seg, mid).await
    } else {
        Vec::new()
    };

    let fields = vec![
        ("client_id".to_string(), body.client_id.to_string()),
        ("user_id".to_string(), body.requester_id.to_string()),
        ("subject".to_string(), subject.to_string()),
        ("description".to_string(), body.description.clone()),
        ("priority".to_string(), priority.to_string()),
        ("source".to_string(), "api".to_string()),
    ];
    let files: Vec<crate::integrations::helpdesk::UploadFile> = images
        .into_iter()
        .enumerate()
        .map(|(i, (name, ct, bytes))| crate::integrations::helpdesk::UploadFile {
            field: format!("attachments[{i}]"),
            filename: name,
            content_type: ct,
            bytes,
        })
        .collect();
    let attached = files.len();

    let created =
        crate::integrations::helpdesk::request_multipart(&state, user_id, None, "/tickets", fields, files)
            .await
            .map_err(AppError::Internal)?;

    let reference = created["data"]["reference"].as_str().unwrap_or("?").to_string();
    let client_name = body.client.as_deref().unwrap_or("?");
    state
        .log(
            "helpdesk",
            "info",
            format!("Ticket {reference} created from email: {subject} ({client_name}, {attached} image(s))"),
        )
        .await;
    Ok(Json(serde_json::json!({
        "reference": reference,
        "id": created["data"]["id"],
        "subject": subject,
        "priority": priority,
        "client": client_name,
        "attached": attached,
    })))
}

// ── Ask AI about this email (multimodal chat handoff) ───────────────────────────

#[derive(Deserialize)]
pub struct AdviseBody {
    session_id: String,
    provider: String,
    instruction: Option<String>,
    #[serde(default)]
    account: Option<String>,
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
    let account = payload.account.as_deref();
    // Message metadata + body.
    let msg = graph_get(
        &state,
        user_id,
        account,
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
        account,
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
                account,
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
        false,
    ));

    let event_stream = stream::unfold(rx, |mut rx| async move {
        let event = match rx.recv().await? {
            AgentEvent::Token(text) => Event::default()
                .data(serde_json::json!({ "type": "token", "text": text }).to_string()),
            AgentEvent::Thinking => Event::default()
                .data(serde_json::json!({ "type": "thinking" }).to_string()),
            AgentEvent::ToolCall { name, detail } => Event::default()
                .data(serde_json::json!({ "type": "tool", "name": name, "detail": detail }).to_string()),
            AgentEvent::Title(title) => Event::default()
                .data(serde_json::json!({ "type": "title", "title": title }).to_string()),
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

// ── Email signatures ─────────────────────────────────────────────────────────
// Per-user HTML signatures, keyed by mailbox address ("" = the user's own
// mailbox). The composer inserts them client-side so they're visible and
// editable before sending.

fn signatures_key(user_id: &str) -> String {
    format!("email_signatures:{user_id}")
}

// GET /api/email/signatures
pub async fn get_signatures(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<Value>> {
    let map: Value = db::settings::get(&state.db, &signatures_key(&user.id))
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_else(|| serde_json::json!({}));
    Ok(Json(map))
}

// PUT /api/email/signatures — stores the whole mailbox→HTML map.
pub async fn put_signatures(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(map): Json<std::collections::HashMap<String, String>>,
) -> AppResult<StatusCode> {
    db::settings::set(&state.db, &signatures_key(&user.id), &map)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Auto-categorizer config / manual run ───────────────────────────────────────

// GET /api/email/categorizer?account=… — that account's AI-sort config.
pub async fn get_categorizer(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(q): Query<MailboxQuery>,
) -> AppResult<Json<crate::categorizer::CategorizerConfig>> {
    let (_integ, cfg) = microsoft::load(&state, &user.id, q.account.as_deref())
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(cfg.categorizer))
}

// PUT /api/email/categorizer?account=… — store that account's AI-sort config.
pub async fn put_categorizer(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(q): Query<MailboxQuery>,
    Json(categorizer): Json<crate::categorizer::CategorizerConfig>,
) -> AppResult<Json<crate::categorizer::CategorizerConfig>> {
    let (integ, mut cfg) = microsoft::load(&state, &user.id, q.account.as_deref())
        .await
        .map_err(AppError::Internal)?;
    cfg.categorizer = categorizer;
    microsoft::save_config(&state, &integ, &cfg)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(cfg.categorizer))
}

// POST /api/email/categorizer/run?account=…&mailbox=… — run one mailbox's sort now.
pub async fn run_categorizer(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(q): Query<MailboxQuery>,
) -> AppResult<Json<crate::categorizer::RunSummary>> {
    let user_id = user.id.as_str();
    let mailbox = q.mailbox.unwrap_or_default();
    let (integ, cfg) = microsoft::load(&state, user_id, q.account.as_deref())
        .await
        .map_err(AppError::Internal)?;
    // Use the provider configured for that mailbox's task (empty → first available).
    let task = cfg.categorizer.tasks.iter().find(|t| t.mailbox == mailbox);
    let provider = task.map(|t| t.provider.clone()).unwrap_or_default();
    let instructions = task.map(|t| t.instructions.clone()).unwrap_or_default();
    let summary = crate::categorizer::run_mailbox(
        &state,
        user_id,
        &integ.id,
        &mailbox,
        &provider,
        &instructions,
        cfg.categorizer.batch_limit,
    )
    .await
    .map_err(AppError::Internal)?;
    Ok(Json(summary))
}
