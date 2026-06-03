use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Redirect,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::integrations::microsoft::{MicrosoftEmailConfig, KEY_MICROSOFT_EMAIL};
use crate::state::AppState;

const GRAPH_SCOPES: &str = "openid email profile offline_access \
    https://graph.microsoft.com/Mail.Read \
    https://graph.microsoft.com/Mail.ReadWrite \
    https://graph.microsoft.com/Mail.Send \
    https://graph.microsoft.com/Calendars.ReadWrite \
    https://graph.microsoft.com/User.Read";

#[derive(Serialize)]
pub struct EmailConfigStatus {
    configured: bool,
    connected: bool,
    tenant_id: String,
    client_id: String,
    connected_email: Option<String>,
}

#[derive(Deserialize)]
pub struct SaveConfigBody {
    tenant_id: String,
    client_id: String,
    /// Omit or send empty string to keep the existing secret.
    client_secret: Option<String>,
}

#[derive(Deserialize)]
pub struct OAuthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

fn redirect_uri_from_host(host: &str) -> String {
    let scheme = if host.starts_with("localhost") || host.starts_with("127.") {
        "http"
    } else {
        "https"
    };
    format!("{scheme}://{host}/api/integrations/email/callback")
}

// GET /api/integrations/email/config
pub async fn get_config(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<EmailConfigStatus>> {
    let config: Option<MicrosoftEmailConfig> =
        db::settings::get(&state.db, KEY_MICROSOFT_EMAIL)
            .await
            .map_err(AppError::Internal)?;

    let status = match config {
        None => EmailConfigStatus {
            configured: false,
            connected: false,
            tenant_id: String::new(),
            client_id: String::new(),
            connected_email: None,
        },
        Some(c) => {
            let configured = !c.tenant_id.is_empty()
                && !c.client_id.is_empty()
                && !c.client_secret.is_empty();
            EmailConfigStatus {
                configured,
                connected: c.access_token.is_some(),
                tenant_id: c.tenant_id,
                client_id: c.client_id,
                connected_email: c.connected_email,
            }
        }
    };

    Ok(Json(status))
}

// POST /api/integrations/email/config
pub async fn save_config(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SaveConfigBody>,
) -> AppResult<StatusCode> {
    let existing: Option<MicrosoftEmailConfig> =
        db::settings::get(&state.db, KEY_MICROSOFT_EMAIL)
            .await
            .map_err(AppError::Internal)?;

    let existing_secret = existing.as_ref().map(|c| c.client_secret.as_str()).unwrap_or("");
    let new_secret_raw = body.client_secret.as_deref().unwrap_or("").trim().to_string();
    let new_secret_entered = !new_secret_raw.is_empty();
    let client_secret = if new_secret_entered {
        new_secret_raw
    } else {
        existing_secret.to_string()
    };

    // Clear OAuth tokens whenever credentials change.
    let credentials_changed = existing.as_ref().map_or(true, |c| {
        c.tenant_id != body.tenant_id || c.client_id != body.client_id || new_secret_entered
    });

    let config = MicrosoftEmailConfig {
        tenant_id: body.tenant_id,
        client_id: body.client_id,
        client_secret,
        access_token: if credentials_changed { None } else { existing.as_ref().and_then(|c| c.access_token.clone()) },
        refresh_token: if credentials_changed { None } else { existing.as_ref().and_then(|c| c.refresh_token.clone()) },
        token_expires_at: if credentials_changed { None } else { existing.as_ref().and_then(|c| c.token_expires_at) },
        connected_email: if credentials_changed { None } else { existing.as_ref().and_then(|c| c.connected_email.clone()) },
    };

    db::settings::set(&state.db, KEY_MICROSOFT_EMAIL, &config)
        .await
        .map_err(AppError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

// DELETE /api/integrations/email/config
pub async fn disconnect(
    State(state): State<Arc<AppState>>,
) -> AppResult<StatusCode> {
    if let Some(mut config) =
        db::settings::get::<MicrosoftEmailConfig>(&state.db, KEY_MICROSOFT_EMAIL)
            .await
            .map_err(AppError::Internal)?
    {
        config.access_token = None;
        config.refresh_token = None;
        config.token_expires_at = None;
        config.connected_email = None;
        db::settings::set(&state.db, KEY_MICROSOFT_EMAIL, &config)
            .await
            .map_err(AppError::Internal)?;
    }

    Ok(StatusCode::NO_CONTENT)
}

// GET /api/integrations/email/connect  — redirects browser to Microsoft login
pub async fn connect(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Redirect> {
    let config =
        db::settings::get::<MicrosoftEmailConfig>(&state.db, KEY_MICROSOFT_EMAIL)
            .await
            .map_err(AppError::Internal)?
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!("Email integration not configured"))
            })?;

    if config.tenant_id.is_empty() || config.client_id.is_empty() || config.client_secret.is_empty() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "Tenant ID, Client ID, and Client Secret must all be set before connecting"
        )));
    }

    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:3000");

    let redirect_uri = redirect_uri_from_host(host);
    let csrf_state = uuid::Uuid::new_v4().to_string();
    *state.oauth_state.lock().await = Some(csrf_state.clone());

    let mut auth_url = url::Url::parse(&format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize",
        config.tenant_id
    ))
    .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid tenant ID in URL: {e}")))?;

    auth_url
        .query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", GRAPH_SCOPES)
        .append_pair("state", &csrf_state)
        .append_pair("response_mode", "query");

    Ok(Redirect::temporary(auth_url.as_str()))
}

// GET /api/integrations/email/callback  — Microsoft redirects here after login
pub async fn callback(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<OAuthCallbackQuery>,
) -> Redirect {
    match callback_inner(state, headers, params).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("OAuth callback error: {e:#}");
            Redirect::temporary("/?integration=email&status=error")
        }
    }
}

async fn callback_inner(
    state: Arc<AppState>,
    headers: HeaderMap,
    params: OAuthCallbackQuery,
) -> anyhow::Result<Redirect> {
    if let Some(err) = params.error {
        anyhow::bail!("Microsoft returned error: {err}");
    }

    let code = params
        .code
        .ok_or_else(|| anyhow::anyhow!("no authorization code in callback"))?;

    let expected = state.oauth_state.lock().await.clone();
    if expected.as_deref() != params.state.as_deref() {
        anyhow::bail!("CSRF state mismatch — possible replay attack");
    }
    *state.oauth_state.lock().await = None;

    let config = db::settings::get::<MicrosoftEmailConfig>(&state.db, KEY_MICROSOFT_EMAIL)
        .await?
        .ok_or_else(|| anyhow::anyhow!("config disappeared between connect and callback"))?;

    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:3000");
    let redirect_uri = redirect_uri_from_host(host);

    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        config.tenant_id
    );

    let form = [
        ("client_id", config.client_id.as_str()),
        ("client_secret", config.client_secret.as_str()),
        ("code", code.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("grant_type", "authorization_code"),
    ];

    let token_res: serde_json::Value = state
        .http_client
        .post(&token_url)
        .form(&form)
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = token_res["error"].as_str() {
        let desc = token_res["error_description"].as_str().unwrap_or("");
        anyhow::bail!("token endpoint error: {err} — {desc}");
    }

    let access_token = token_res["access_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no access_token in response: {token_res}"))?
        .to_string();

    let refresh_token = token_res["refresh_token"].as_str().map(|s| s.to_string());
    let expires_in = token_res["expires_in"].as_i64().unwrap_or(3600);
    let token_expires_at = chrono::Utc::now().timestamp() + expires_in;

    let me: serde_json::Value = state
        .http_client
        .get("https://graph.microsoft.com/v1.0/me")
        .bearer_auth(&access_token)
        .send()
        .await?
        .json()
        .await?;

    let connected_email = me["mail"]
        .as_str()
        .or_else(|| me["userPrincipalName"].as_str())
        .map(|s| s.to_string());

    let updated = MicrosoftEmailConfig {
        tenant_id: config.tenant_id,
        client_id: config.client_id,
        client_secret: config.client_secret,
        access_token: Some(access_token),
        refresh_token,
        token_expires_at: Some(token_expires_at),
        connected_email,
    };

    db::settings::set(&state.db, KEY_MICROSOFT_EMAIL, &updated).await?;

    Ok(Redirect::temporary("/?integration=email&status=connected"))
}
