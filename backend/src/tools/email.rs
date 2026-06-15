//! Email tools — model-facing adapter over the shared Microsoft Graph helpers.
//! Read-only access plus draft creation: the agent can search, list, and read
//! the user's mail and prepare a draft in the Drafts folder, but sending stays
//! with the human (there is deliberately no send tool).

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::integrations::graph::{
    self, graph_get, graph_patch, graph_post, html_to_text, prepend_html, recipients, GRAPH,
};
use crate::state::AppState;

use super::localize;

/// Full-body char budget for email_read (~2k tokens).
const BODY_MAX: usize = 8000;
/// Result caps: tools keep tighter limits than the UI routes for context budget.
const LIST_DEFAULT: u64 = 15;
const LIST_MAX: u64 = 30;

pub fn schemas() -> Vec<Value> {
    let mailbox_prop = json!({
        "type": "string",
        "description": "Optional shared mailbox address; omit for the user's own mailbox."
    });
    vec![
        json!({
            "name": "email_search",
            "description": "Search the user's mailbox. Combines keyword search (sender, subject, body) with meaning-based matching over indexed mail, so conceptual queries (\"that invoice dispute\") work too. Returns message metadata and a short snippet; use email_read with an id for the full body.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search terms." },
                    "mailbox": mailbox_prop,
                    "limit": { "type": "integer", "description": "Max results, default 15, max 30." }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "email_list",
            "description": "List recent messages in a mail folder, newest first. Returns metadata and a short snippet, not full bodies.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "folder": { "type": "string", "description": "Folder name (e.g. \"Inbox\", \"Sent Items\", \"Processed\"). Default Inbox." },
                    "mailbox": mailbox_prop,
                    "limit": { "type": "integer", "description": "Max results, default 15, max 30." }
                }
            }
        }),
        json!({
            "name": "email_read",
            "description": "Read one email in full: sender, recipients, date, plain-text body, and attachment names. Get the message id from email_search or email_list.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "message_id": { "type": "string" },
                    "mailbox": mailbox_prop
                },
                "required": ["message_id"]
            }
        }),
        json!({
            "name": "email_draft",
            "description": "Create a DRAFT email in the Drafts folder. It is NOT sent — the user reviews and sends it themselves. Use kind reply/reply_all/forward with a message_id to respond to a message (quoted history is kept automatically), or kind new with to + subject for a fresh message.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["reply", "reply_all", "forward", "new"] },
                    "body": { "type": "string", "description": "The message text. Plain text is fine; simple HTML is also accepted." },
                    "message_id": { "type": "string", "description": "The message being responded to. Required for reply/reply_all/forward." },
                    "to": { "type": "array", "items": { "type": "string" }, "description": "Recipient addresses. Required for new and forward; for replies Graph fills them in automatically." },
                    "subject": { "type": "string", "description": "Required for new; replies/forwards keep their Re:/Fwd: subject unless overridden." },
                    "mailbox": mailbox_prop
                },
                "required": ["kind", "body"]
            }
        }),
    ]
}

pub fn handles(name: &str) -> bool {
    matches!(name, "email_search" | "email_list" | "email_read" | "email_draft")
}

pub async fn execute(state: &AppState, user_id: &str, name: &str, args: Value) -> Result<Value> {
    match name {
        "email_search" => search(state, user_id, args).await,
        "email_list" => list(state, user_id, args).await,
        "email_read" => read(state, user_id, args).await,
        "email_draft" => draft(state, user_id, args).await,
        _ => Err(anyhow!("unknown email tool '{name}'")),
    }
}

/// Fields shared by search/list results — bodyPreview is Graph's own snippet.
const SUMMARY_SELECT: &str =
    "id,subject,from,bodyPreview,receivedDateTime,isRead,hasAttachments,flag";

fn limit_arg(args: &Value) -> String {
    args["limit"]
        .as_u64()
        .unwrap_or(LIST_DEFAULT)
        .clamp(1, LIST_MAX)
        .to_string()
}

/// "Name <addr>" (or just the address) from a Graph recipient object.
fn address(v: &Value) -> String {
    let name = v["emailAddress"]["name"].as_str().unwrap_or_default();
    let addr = v["emailAddress"]["address"].as_str().unwrap_or_default();
    if name.is_empty() || name == addr {
        addr.to_string()
    } else {
        format!("{name} <{addr}>")
    }
}

/// Compact, model-friendly summary of one Graph message.
fn summarize(msg: &Value, tz: chrono_tz::Tz) -> Value {
    let received = msg["receivedDateTime"].as_str().unwrap_or_default();
    let (_, received_display) = localize(received, tz);
    json!({
        "id": msg["id"],
        "subject": msg["subject"].as_str().unwrap_or("(no subject)"),
        "from": address(&msg["from"]),
        "snippet": msg["bodyPreview"].as_str().unwrap_or_default(),
        "received": received_display.unwrap_or_else(|| received.to_string()),
        "is_read": msg["isRead"].as_bool().unwrap_or(true),
        "has_attachments": msg["hasAttachments"].as_bool().unwrap_or(false),
        "flagged": msg["flag"]["flagStatus"].as_str() == Some("flagged"),
    })
}

fn summarize_all(body: &Value, tz: chrono_tz::Tz) -> Vec<Value> {
    body["value"]
        .as_array()
        .map(|msgs| msgs.iter().map(|m| summarize(m, tz)).collect())
        .unwrap_or_default()
}

async fn search(state: &AppState, user_id: &str, args: Value) -> Result<Value> {
    let query = args["query"].as_str().unwrap_or("").trim().replace('"', "");
    if query.is_empty() {
        return Err(anyhow!("query is required"));
    }
    let seg = graph::mailbox_seg(args["mailbox"].as_str())?;
    let body = graph_get(
        state,
        user_id,
        None,
        &format!("{GRAPH}/{seg}/messages"),
        &[
            ("$search", &format!("\"{query}\"")),
            ("$select", SUMMARY_SELECT),
            ("$top", &limit_arg(&args)),
        ],
    )
    .await?;
    let tz = state.home_tz(user_id).await;
    let mut results = summarize_all(&body, tz);

    // Meaning-based hits from the local index, merged after Graph's keyword
    // results (skipping ids Graph already found). Best-effort: an embedding
    // failure (Ollama down, nothing indexed yet) just yields keyword-only.
    let limit = args["limit"].as_u64().unwrap_or(LIST_DEFAULT).clamp(1, LIST_MAX) as usize;
    let mailbox = args["mailbox"].as_str().unwrap_or("");
    match crate::email_index::search(state, user_id, mailbox, &query, limit).await {
        Ok(hits) => {
            for hit in hits {
                if results.iter().any(|r| r["id"].as_str() == Some(hit.message_id.as_str())) {
                    continue;
                }
                let (_, received_display) = localize(&hit.received_at, tz);
                results.push(json!({
                    "id": hit.message_id,
                    "subject": hit.subject,
                    "from": hit.sender,
                    "snippet": hit.snippet,
                    "received": received_display.unwrap_or(hit.received_at),
                    "matched_by": "meaning",
                }));
            }
        }
        Err(e) => tracing::debug!("semantic email search unavailable: {e}"),
    }

    Ok(json!({ "messages": results }))
}

/// Folders Graph accepts as well-known names directly in the URL.
const WELL_KNOWN: &[(&str, &str)] = &[
    ("inbox", "inbox"),
    ("sent", "sentitems"),
    ("sentitems", "sentitems"),
    ("drafts", "drafts"),
    ("deleted", "deleteditems"),
    ("deleteditems", "deleteditems"),
    ("trash", "deleteditems"),
    ("junk", "junkemail"),
    ("junkemail", "junkemail"),
    ("spam", "junkemail"),
    ("archive", "archive"),
    ("outbox", "outbox"),
];

/// Resolve a folder argument to a URL segment: a well-known name when it
/// matches, otherwise the id of the folder whose displayName matches
/// case-insensitively (custom folders like "Processed").
async fn resolve_folder(
    state: &AppState,
    user_id: &str,
    seg: &str,
    folder: &str,
) -> Result<String> {
    let normalized: String = folder
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if let Some((_, well_known)) = WELL_KNOWN.iter().find(|(k, _)| *k == normalized) {
        return Ok(well_known.to_string());
    }
    let folders = graph_get(
        state,
        user_id,
        None,
        &format!("{GRAPH}/{seg}/mailFolders"),
        &[("$top", "100"), ("$select", "id,displayName")],
    )
    .await?;
    folders["value"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|f| {
            f["displayName"]
                .as_str()
                .map(|d| d.eq_ignore_ascii_case(folder.trim()))
                .unwrap_or(false)
        })
        .and_then(|f| f["id"].as_str().map(String::from))
        .ok_or_else(|| anyhow!("no folder named '{folder}'"))
}

async fn list(state: &AppState, user_id: &str, args: Value) -> Result<Value> {
    let seg = graph::mailbox_seg(args["mailbox"].as_str())?;
    let folder_arg = args["folder"].as_str().unwrap_or("inbox");
    let folder = resolve_folder(state, user_id, &seg, folder_arg).await?;
    let body = graph_get(
        state,
        user_id,
        None,
        &format!("{GRAPH}/{seg}/mailFolders/{folder}/messages"),
        &[
            ("$select", SUMMARY_SELECT),
            ("$orderby", "receivedDateTime desc"),
            ("$top", &limit_arg(&args)),
        ],
    )
    .await?;
    let tz = state.home_tz(user_id).await;
    Ok(json!({ "folder": folder_arg, "messages": summarize_all(&body, tz) }))
}

/// Char-boundary-safe truncation with a marker.
fn truncate_body(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}\n…[truncated]")
}

async fn read(state: &AppState, user_id: &str, args: Value) -> Result<Value> {
    let message_id = args["message_id"]
        .as_str()
        .ok_or_else(|| anyhow!("message_id is required"))?;
    let seg = graph::mailbox_seg(args["mailbox"].as_str())?;
    let msg = graph_get(
        state,
        user_id,
        None,
        &format!("{GRAPH}/{seg}/messages/{message_id}"),
        &[(
            "$select",
            "id,subject,from,toRecipients,ccRecipients,body,receivedDateTime,hasAttachments",
        )],
    )
    .await?;

    let body_raw = msg["body"]["content"].as_str().unwrap_or_default();
    let body_text = if msg["body"]["contentType"].as_str() == Some("html") {
        html_to_text(body_raw)
    } else {
        body_raw.to_string()
    };

    let to_list = |key: &str| -> Vec<String> {
        msg[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(address)
            .collect()
    };

    let attachments = if msg["hasAttachments"].as_bool().unwrap_or(false) {
        let atts = graph_get(
            state,
            user_id,
            None,
            &format!("{GRAPH}/{seg}/messages/{message_id}/attachments"),
            &[("$select", "name,contentType,size")],
        )
        .await?;
        atts["value"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| {
                json!({
                    "name": a["name"],
                    "content_type": a["contentType"],
                    "size": a["size"],
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    let received = msg["receivedDateTime"].as_str().unwrap_or_default();
    let tz = state.home_tz(user_id).await;
    let (_, received_display) = localize(received, tz);

    Ok(json!({
        "id": msg["id"],
        "subject": msg["subject"].as_str().unwrap_or("(no subject)"),
        "from": address(&msg["from"]),
        "to": to_list("toRecipients"),
        "cc": to_list("ccRecipients"),
        "received": received_display.unwrap_or_else(|| received.to_string()),
        "body": truncate_body(&body_text, BODY_MAX),
        "attachments": attachments,
    }))
}

/// Minimal plain-text → HTML: escape and convert newlines, unless the body
/// already looks like HTML.
fn body_html(body: &str) -> String {
    if body.contains('<') && body.contains('>') {
        return body.to_string();
    }
    let escaped = body
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<p>{}</p>", escaped.replace("\n\n", "</p><p>").replace('\n', "<br>"))
}

async fn draft(state: &AppState, user_id: &str, args: Value) -> Result<Value> {
    let kind = args["kind"].as_str().unwrap_or_default();
    let body = args["body"]
        .as_str()
        .filter(|b| !b.trim().is_empty())
        .ok_or_else(|| anyhow!("body is required"))?;
    let seg = graph::mailbox_seg(args["mailbox"].as_str())?;
    let to: Vec<String> = args["to"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    let subject = args["subject"].as_str().map(str::trim).filter(|s| !s.is_empty());
    let html = body_html(body);

    let draft = match kind {
        "reply" | "reply_all" | "forward" => {
            let message_id = args["message_id"]
                .as_str()
                .ok_or_else(|| anyhow!("message_id is required for {kind}"))?;
            if kind == "forward" && to.is_empty() {
                return Err(anyhow!("forward requires at least one `to` recipient"));
            }
            let verb = match kind {
                "forward" => "createForward",
                "reply_all" => "createReplyAll",
                _ => "createReply",
            };
            let draft = graph_post(
                state,
                user_id,
                None,
                &format!("{GRAPH}/{seg}/messages/{message_id}/{verb}"),
                &json!({}),
            )
            .await?;
            // The new text goes above the quoted history Graph put in the draft.
            let quoted = draft["body"]["content"].as_str().unwrap_or_default();
            let mut patch = json!({
                "body": { "contentType": "html", "content": prepend_html(&html, quoted) },
            });
            if !to.is_empty() {
                patch["toRecipients"] = Value::Array(recipients(&to));
            }
            if let Some(s) = subject {
                patch["subject"] = Value::String(s.to_string());
            }
            let draft_id = draft["id"]
                .as_str()
                .ok_or_else(|| anyhow!("draft missing id"))?;
            graph_patch(state, user_id, None, &format!("{GRAPH}/{seg}/messages/{draft_id}"), &patch)
                .await?
        }
        "new" => {
            if to.is_empty() {
                return Err(anyhow!("new requires at least one `to` recipient"));
            }
            let subject = subject.ok_or_else(|| anyhow!("subject is required for new"))?;
            graph_post(
                state,
                user_id,
                None,
                &format!("{GRAPH}/{seg}/messages"),
                &json!({
                    "subject": subject,
                    "body": { "contentType": "html", "content": html },
                    "toRecipients": recipients(&to),
                }),
            )
            .await?
        }
        other => return Err(anyhow!("unknown draft kind '{other}'")),
    };

    Ok(json!({
        "drafted": true,
        "draft_id": draft["id"],
        "subject": draft["subject"],
        "web_link": draft["webLink"],
        "note": "Draft saved to the Drafts folder — the user must review and send it.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_expose_the_four_tools() {
        let names: Vec<String> = schemas()
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["email_search", "email_list", "email_read", "email_draft"]);
        for name in &names {
            assert!(handles(name));
        }
        assert!(!handles("email_send"));
    }

    #[test]
    fn summarize_shapes_a_graph_message() {
        let msg = json!({
            "id": "AAMk1",
            "subject": "Invoice #42",
            "from": { "emailAddress": { "name": "Jo Bloggs", "address": "jo@example.com" } },
            "bodyPreview": "Please find attached…",
            "receivedDateTime": "2026-06-04T02:45:00Z",
            "isRead": false,
            "hasAttachments": true,
            "flag": { "flagStatus": "flagged" }
        });
        let out = summarize(&msg, chrono_tz::Tz::Australia__Brisbane);
        assert_eq!(out["from"], "Jo Bloggs <jo@example.com>");
        assert_eq!(out["received"], "Thu 4 Jun 2026, 12:45 PM");
        assert_eq!(out["is_read"], false);
        assert_eq!(out["flagged"], true);
        assert_eq!(out["snippet"], "Please find attached…");
    }

    #[test]
    fn truncate_body_cuts_on_char_boundary() {
        let s = "é".repeat(10);
        assert_eq!(truncate_body(&s, 10), s);
        let cut = truncate_body(&s, 4);
        assert_eq!(cut, format!("{}\n…[truncated]", "é".repeat(4)));
    }

    #[test]
    fn body_html_escapes_plain_text() {
        assert_eq!(
            body_html("a < b\n\nnext & line\nsame"),
            "<p>a &lt; b</p><p>next &amp; line<br>same</p>"
        );
        // Already-HTML bodies pass through.
        assert_eq!(body_html("<p>hi</p>"), "<p>hi</p>");
    }
}
