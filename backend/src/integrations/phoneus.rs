//! PhoneUs integration — Sanctum-token API client for the user's PhoneUs
//! accounts/comms platform (Laravel). Connected per Episteme user from
//! Settings → Integrations: base URL + email/password are exchanged for a
//! Sanctum token via POST /api/mobile/auth/login; only the token (never the
//! password) is stored. The chat agent's phoneus tools all go through `request`.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhoneusConfig {
    /// e.g. https://portal.phoneus.example — stored without a trailing slash.
    pub base_url: String,
    /// Account the token belongs to (display only).
    pub email: String,
    /// Sanctum plain-text bearer token.
    pub token: String,
}

/// Resolve the chosen PhoneUs instance's config (by name, default, or sole).
pub async fn config(state: &AppState, user_id: &str, instance: Option<&str>) -> Result<PhoneusConfig> {
    crate::integrations::registry::resolve(&state.db, user_id, "phoneus", instance)
        .await?
        .parse_config()
}

/// Exchange credentials for a Sanctum token (POST /api/mobile/auth/login).
pub async fn login(state: &AppState, base_url: &str, email: &str, password: &str) -> Result<String> {
    let url = format!("{}/api/mobile/auth/login", base_url.trim_end_matches('/'));
    let res = state
        .http_client
        .post(&url)
        .header("Accept", "application/json")
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .await
        .map_err(|e| anyhow!("could not reach PhoneUs at {url}: {e}"))?;

    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = body["message"].as_str().unwrap_or("login failed");
        return Err(anyhow!("PhoneUs login failed ({status}): {msg}"));
    }
    body["token"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("PhoneUs login response missing token"))
}

/// Authenticated PhoneUs API call. `path` is relative to /api/integration (e.g.
/// "/customers"). Laravel validation errors are flattened into the message so
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
    let url = format!("{}/api/integration{}", cfg.base_url.trim_end_matches('/'), path);
    let mut req = state
        .http_client
        .request(method, &url)
        .bearer_auth(&cfg.token)
        .header("Accept", "application/json");
    if let Some(b) = body {
        req = req.json(b);
    }
    let res = req.send().await.map_err(|e| anyhow!("PhoneUs request failed: {e}"))?;

    let status = res.status();
    let parsed: Value = res.json().await.unwrap_or(Value::Null);

    if status.as_u16() == 401 {
        return Err(anyhow!("PhoneUs session expired — reconnect in Settings → Integrations"));
    }
    if !status.is_success() {
        let mut msg = parsed["message"].as_str().unwrap_or("PhoneUs API error").to_string();
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
        return Err(anyhow!("PhoneUs API {status}: {msg}"));
    }
    Ok(parsed)
}

/// Fetch the portal's health endpoint (GET /api/health). This sits *outside* the
/// `/api/integration` namespace `request` uses, so it's a dedicated call. Unlike
/// `request`, a non-2xx response is *not* an error here: a degraded portal may
/// answer 503 with the details in its body, and that failing status is exactly
/// the signal we want the model to inspect. Returns the HTTP `status`, an `ok`
/// flag, and the raw `report` the portal provided (overall `status` plus
/// per-section `items`). Only a genuine transport failure (or an expired
/// session) errors.
pub async fn health(state: &AppState, user_id: &str, instance: Option<&str>) -> Result<Value> {
    let cfg = config(state, user_id, instance).await?;
    let url = format!("{}/api/health", cfg.base_url.trim_end_matches('/'));
    let res = state
        .http_client
        .get(&url)
        .bearer_auth(&cfg.token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| anyhow!("could not reach PhoneUs at {url}: {e}"))?;

    let status = res.status();
    if status.as_u16() == 401 {
        return Err(anyhow!("PhoneUs session expired — reconnect in Settings → Integrations"));
    }
    let report: Value = res.json().await.unwrap_or(Value::Null);
    Ok(serde_json::json!({
        "status": status.as_u16(),
        "ok": status.is_success(),
        "report": report,
    }))
}
