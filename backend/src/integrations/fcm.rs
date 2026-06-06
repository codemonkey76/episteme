//! Firebase Cloud Messaging (HTTP v1) — push notifications to the mobile app.
//!
//! Configured by dropping a Firebase service-account JSON at
//! `$FCM_SERVICE_ACCOUNT` (default `{DATA_DIR}/firebase-service-account.json`).
//! When the file is absent every send quietly no-ops, so push is strictly
//! opt-in infrastructure.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::AppState;

#[derive(Debug, Clone, Deserialize)]
struct ServiceAccount {
    project_id: String,
    private_key: String,
    client_email: String,
    token_uri: String,
}

fn sa_path() -> String {
    std::env::var("FCM_SERVICE_ACCOUNT").unwrap_or_else(|_| {
        let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string());
        format!("{data_dir}/firebase-service-account.json")
    })
}

/// Loaded once; None = file missing/unreadable → push disabled.
fn service_account() -> Option<&'static ServiceAccount> {
    static SA: OnceLock<Option<ServiceAccount>> = OnceLock::new();
    SA.get_or_init(|| {
        let path = sa_path();
        match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str(&raw) {
                Ok(sa) => Some(sa),
                Err(e) => {
                    tracing::warn!("push disabled: {path} is not a valid service account: {e}");
                    None
                }
            },
            Err(_) => None, // absent = not configured; stay silent
        }
    })
    .as_ref()
}

pub fn configured() -> bool {
    service_account().is_some()
}

/// Cached OAuth bearer token (FCM tokens last an hour; refresh at 55 min).
static TOKEN: Mutex<Option<(String, Instant)>> = Mutex::new(None);

async fn access_token(client: &reqwest::Client) -> Result<String> {
    if let Some((token, at)) = TOKEN.lock().unwrap().clone() {
        if at.elapsed() < Duration::from_secs(55 * 60) {
            return Ok(token);
        }
    }
    let sa = service_account().ok_or_else(|| anyhow!("push not configured"))?;

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let claims = json!({
        "iss": sa.client_email,
        "scope": "https://www.googleapis.com/auth/firebase.messaging",
        "aud": sa.token_uri,
        "iat": now,
        "exp": now + 3600,
    });
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(sa.private_key.as_bytes())
        .map_err(|e| anyhow!("service account private key invalid: {e}"))?;
    let assertion = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &claims,
        &key,
    )?;

    let res: Value = client
        .post(&sa.token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion.as_str()),
        ])
        .send()
        .await?
        .json()
        .await?;
    let token = res["access_token"]
        .as_str()
        .ok_or_else(|| anyhow!("FCM token exchange failed: {res}"))?
        .to_string();

    *TOKEN.lock().unwrap() = Some((token.clone(), Instant::now()));
    Ok(token)
}

/// Outcome of a single-device send, so callers can prune dead tokens.
pub enum SendOutcome {
    Sent,
    /// FCM says this token is gone — delete it.
    Unregistered,
}

async fn send_to_token(
    client: &reqwest::Client,
    device_token: &str,
    title: &str,
    body: &str,
) -> Result<SendOutcome> {
    let sa = service_account().ok_or_else(|| anyhow!("push not configured"))?;
    let bearer = access_token(client).await?;

    let res = client
        .post(format!(
            "https://fcm.googleapis.com/v1/projects/{}/messages:send",
            sa.project_id
        ))
        .bearer_auth(&bearer)
        .json(&json!({
            "message": {
                "token": device_token,
                "notification": { "title": title, "body": body },
            }
        }))
        .send()
        .await?;

    let status = res.status();
    if status.is_success() {
        return Ok(SendOutcome::Sent);
    }
    let detail: Value = res.json().await.unwrap_or(Value::Null);
    let err_status = detail["error"]["details"][0]["errorCode"]
        .as_str()
        .or_else(|| detail["error"]["status"].as_str())
        .unwrap_or_default()
        .to_string();
    if status == reqwest::StatusCode::NOT_FOUND || err_status.contains("UNREGISTERED") {
        return Ok(SendOutcome::Unregistered);
    }
    Err(anyhow!("FCM send failed: {status} {err_status}"))
}

/// Push a notification to all of a user's devices, pruning dead tokens.
/// Best-effort: no-op when unconfigured, logs failures, never errors out.
pub async fn notify(state: &AppState, user_id: &str, title: &str, body: &str) {
    if !configured() {
        return;
    }
    let tokens = match crate::db::push_tokens::list_for_user(&state.db, user_id).await {
        Ok(t) if !t.is_empty() => t,
        _ => return,
    };
    // Notification bodies should be glanceable; clip long agent output.
    let body: String = body.chars().take(180).collect();
    for token in tokens {
        match send_to_token(&state.http_client, &token, title, &body).await {
            Ok(SendOutcome::Sent) => {}
            Ok(SendOutcome::Unregistered) => {
                let _ = crate::db::push_tokens::remove(&state.db, &token).await;
                tracing::info!("pruned unregistered push token");
            }
            Err(e) => tracing::warn!("push notify failed: {e}"),
        }
    }
}
