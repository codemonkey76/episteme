//! Shared Microsoft Graph plumbing used by the email routes, the calendar,
//! the categorizer worker, and the agent's native email tools. Everything
//! here returns `anyhow::Result` so it composes with both route handlers
//! (`AppError: From<anyhow::Error>`) and the tools contract.

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::integrations::microsoft;
use crate::state::AppState;

pub const GRAPH: &str = "https://graph.microsoft.com/v1.0";

/// Graph mailbox path segment: `me` for the signed-in user, or `users/{address}`
/// for a shared mailbox the user has delegated access to. The address is
/// validated to block path injection; Graph still enforces the real access
/// rights, so an address the user can't open just yields a 403.
pub fn mailbox_seg(mailbox: Option<&str>) -> Result<String> {
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
                Err(anyhow!("invalid mailbox address"))
            }
        }
    }
}

pub async fn graph_get(
    state: &AppState,
    user_id: &str,
    account: Option<&str>,
    url: &str,
    params: &[(&str, &str)],
) -> Result<Value> {
    let token = microsoft::get_valid_token(state, user_id, account).await?;

    let response = state
        .http_client
        .get(url)
        .query(params)
        .bearer_auth(&token)
        .send()
        .await?;

    let status = response.status();
    let body: Value = response.json().await?;

    if !status.is_success() {
        let msg = body["error"]["message"]
            .as_str()
            .unwrap_or("Graph API error")
            .to_string();
        tracing::error!("Graph API {status}: {msg}");
        return Err(anyhow!("Graph API {status}: {msg}"));
    }

    Ok(body)
}

/// POST a JSON body to Graph and return the parsed response.
pub async fn graph_post(
    state: &AppState,
    user_id: &str,
    account: Option<&str>,
    url: &str,
    body: &Value,
) -> Result<Value> {
    let token = microsoft::get_valid_token(state, user_id, account).await?;

    let response = state
        .http_client
        .post(url)
        .bearer_auth(&token)
        .json(body)
        .send()
        .await?;

    let status = response.status();
    let parsed: Value = response.json().await.unwrap_or(Value::Null);

    if !status.is_success() {
        let msg = parsed["error"]["message"]
            .as_str()
            .unwrap_or("Graph API error")
            .to_string();
        tracing::error!("Graph POST {status}: {msg}");
        return Err(anyhow!("Graph POST {status}: {msg}"));
    }

    Ok(parsed)
}

/// PATCH a JSON body to Graph (draft updates).
pub async fn graph_patch(
    state: &AppState,
    user_id: &str,
    account: Option<&str>,
    url: &str,
    body: &Value,
) -> Result<Value> {
    let token = microsoft::get_valid_token(state, user_id, account).await?;

    let response = state
        .http_client
        .patch(url)
        .bearer_auth(&token)
        .json(body)
        .send()
        .await?;

    let status = response.status();
    let parsed: Value = response.json().await.unwrap_or(Value::Null);

    if !status.is_success() {
        let msg = parsed["error"]["message"]
            .as_str()
            .unwrap_or("Graph API error")
            .to_string();
        tracing::error!("Graph PATCH {status}: {msg}");
        return Err(anyhow!("Graph PATCH {status}: {msg}"));
    }

    Ok(parsed)
}

/// DELETE a Graph resource. Treats any 2xx as success.
pub async fn graph_delete(state: &AppState, user_id: &str, account: Option<&str>, url: &str) -> Result<()> {
    let token = microsoft::get_valid_token(state, user_id, account).await?;

    let response = state
        .http_client
        .delete(url)
        .bearer_auth(&token)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        tracing::error!("Graph DELETE {status}");
        return Err(anyhow!("Graph DELETE {status}"));
    }
    Ok(())
}

/// Map plain addresses to Graph recipient objects.
pub fn recipients(addrs: &[String]) -> Vec<Value> {
    addrs
        .iter()
        .map(|addr| serde_json::json!({ "emailAddress": { "address": addr } }))
        .collect()
}

/// Byte-wise ASCII-case-insensitive substring search (HTML tags are ASCII, and
/// byte offsets stay valid in the original string — unlike `to_lowercase()`,
/// which can change length).
pub fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Insert the user's message just inside <body>, above the quoted history that
/// Graph put in the createReply/createForward draft.
pub fn prepend_html(user_html: &str, draft_html: &str) -> String {
    if let Some(start) = find_ci(draft_html, "<body") {
        if let Some(close) = draft_html[start..].find('>') {
            let at = start + close + 1;
            return format!("{}{}{}", &draft_html[..at], user_html, &draft_html[at..]);
        }
    }
    format!("{user_html}{draft_html}")
}

/// Minimal HTML→text: block tags to newlines, drop script/style, strip tags,
/// decode a few entities, tidy whitespace. Good enough to give the model context.
pub fn html_to_text(html: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_seg_own_mailbox() {
        assert_eq!(mailbox_seg(None).unwrap(), "me");
        assert_eq!(mailbox_seg(Some("")).unwrap(), "me");
        assert_eq!(mailbox_seg(Some("  ")).unwrap(), "me");
    }

    #[test]
    fn mailbox_seg_shared_mailbox() {
        assert_eq!(
            mailbox_seg(Some("support@example.com")).unwrap(),
            "users/support@example.com"
        );
    }

    #[test]
    fn mailbox_seg_rejects_injection() {
        assert!(mailbox_seg(Some("not-an-address")).is_err());
        assert!(mailbox_seg(Some("a@b/../c")).is_err());
        assert!(mailbox_seg(Some("a b@c.com")).is_err());
    }

    #[test]
    fn html_to_text_strips_and_decodes() {
        let html = "<html><head><style>p{color:red}</style></head>\
                    <body><p>Hello &amp; welcome</p><script>alert(1)</script>\
                    <div>Line two</div></body></html>";
        let text = html_to_text(html);
        // Dropped <script> blocks leave a single space; only line ends are trimmed.
        assert_eq!(text, "Hello & welcome\n Line two");
        assert!(!text.contains("alert"));
        assert!(!text.contains("color"));
    }

    #[test]
    fn prepend_html_inserts_inside_body() {
        let draft = "<html><BODY style=\"x\"><p>quoted</p></BODY></html>";
        let merged = prepend_html("<p>mine</p>", draft);
        assert_eq!(
            merged,
            "<html><BODY style=\"x\"><p>mine</p><p>quoted</p></BODY></html>"
        );
    }

    #[test]
    fn prepend_html_without_body_concatenates() {
        assert_eq!(prepend_html("<p>a</p>", "<p>b</p>"), "<p>a</p><p>b</p>");
    }
}
