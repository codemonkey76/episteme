//! Helpdesk integration — Sanctum-token API client for the user's custom
//! helpdesk platform (Laravel). Connected per Episteme user from Settings →
//! Integrations: base URL + agent email/password are exchanged for a ~1-year
//! bearer token via POST /api/login; only the token (never the password) is
//! stored. The chat agent's helpdesk tools all go through `request`.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskConfig {
    /// e.g. https://helpdesk.example.com — stored without a trailing slash.
    pub base_url: String,
    /// Agent account the token belongs to (display only).
    pub email: String,
    /// Sanctum plain-text bearer token.
    pub token: String,
}

/// Resolve the chosen helpdesk instance's config (by name, default, or sole).
pub async fn config(state: &AppState, user_id: &str, instance: Option<&str>) -> Result<HelpdeskConfig> {
    crate::integrations::registry::resolve(&state.db, user_id, "helpdesk", instance)
        .await?
        .parse_config()
}

/// Exchange agent credentials for a Sanctum token (POST /api/login).
pub async fn login(state: &AppState, base_url: &str, email: &str, password: &str) -> Result<String> {
    let url = format!("{}/api/login", base_url.trim_end_matches('/'));
    let res = state
        .http_client
        .post(&url)
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "email": email,
            "password": password,
            "device_name": "episteme",
        }))
        .send()
        .await
        .map_err(|e| anyhow!("could not reach helpdesk at {url}: {e}"))?;

    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = body["message"].as_str().unwrap_or("login failed");
        return Err(anyhow!("helpdesk login failed ({status}): {msg}"));
    }
    body["token"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("helpdesk login response missing token"))
}

/// One file to upload as a multipart attachment.
pub struct UploadFile {
    pub field: String, // e.g. "attachments[0]"
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// Authenticated multipart POST (text `fields` + file parts) — for endpoints
/// that take uploads, e.g. creating a ticket with image attachments. Errors are
/// flattened like `request`.
pub async fn request_multipart(
    state: &AppState,
    user_id: &str,
    instance: Option<&str>,
    path: &str,
    fields: Vec<(String, String)>,
    files: Vec<UploadFile>,
) -> Result<Value> {
    let cfg = config(state, user_id, instance).await?;
    let url = format!("{}/api{}", cfg.base_url.trim_end_matches('/'), path);

    let mut form = reqwest::multipart::Form::new();
    for (k, v) in fields {
        form = form.text(k, v);
    }
    for f in files {
        let part = reqwest::multipart::Part::bytes(f.bytes)
            .file_name(f.filename)
            .mime_str(&f.content_type)
            .map_err(|e| anyhow!("bad attachment mime: {e}"))?;
        form = form.part(f.field, part);
    }

    let res = state
        .http_client
        .post(&url)
        .bearer_auth(&cfg.token)
        .header("Accept", "application/json")
        .multipart(form)
        .send()
        .await
        .map_err(|e| anyhow!("helpdesk request failed: {e}"))?;

    let status = res.status();
    let parsed: Value = res.json().await.unwrap_or(Value::Null);
    if status.as_u16() == 401 {
        return Err(anyhow!("helpdesk session expired — reconnect in Settings → Integrations"));
    }
    if !status.is_success() {
        let mut msg = parsed["message"].as_str().unwrap_or("helpdesk API error").to_string();
        if let Some(errors) = parsed["errors"].as_object() {
            let details: Vec<&str> = errors
                .values()
                .filter_map(|v| v.as_array())
                .flatten()
                .filter_map(|v| v.as_str())
                .collect();
            if !details.is_empty() {
                msg = format!("{msg} — {}", details.join("; "));
            }
        }
        return Err(anyhow!("helpdesk API {status}: {msg}"));
    }
    Ok(parsed)
}

/// Authenticated helpdesk API call. `path` is relative to /api (e.g.
/// "/tickets"). Laravel validation errors are flattened into the message so
/// the model can correct its arguments.
pub async fn request(
    state: &AppState,
    user_id: &str,
    instance: Option<&str>,
    method: reqwest::Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let cfg = config(state, user_id, instance).await?;
    let url = format!("{}/api{}", cfg.base_url.trim_end_matches('/'), path);
    let mut req = state
        .http_client
        .request(method, &url)
        .bearer_auth(&cfg.token)
        .header("Accept", "application/json");
    if let Some(b) = body {
        req = req.json(b);
    }
    let res = req.send().await.map_err(|e| anyhow!("helpdesk request failed: {e}"))?;

    let status = res.status();
    let parsed: Value = res.json().await.unwrap_or(Value::Null);

    if status.as_u16() == 401 {
        return Err(anyhow!("helpdesk session expired — reconnect in Settings → Integrations"));
    }
    if !status.is_success() {
        let mut msg = parsed["message"].as_str().unwrap_or("helpdesk API error").to_string();
        if let Some(errors) = parsed["errors"].as_object() {
            let details: Vec<&str> = errors
                .values()
                .filter_map(|v| v.as_array())
                .flatten()
                .filter_map(|v| v.as_str())
                .collect();
            if !details.is_empty() {
                msg = format!("{msg} — {}", details.join("; "));
            }
        }
        return Err(anyhow!("helpdesk API {status}: {msg}"));
    }
    Ok(parsed)
}
