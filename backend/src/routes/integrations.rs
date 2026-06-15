use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Redirect,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use axum::Extension;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::integrations::microsoft::{self, MicrosoftConfig, MicrosoftTokens};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

const GRAPH_SCOPES: &str = "openid email profile offline_access \
    https://graph.microsoft.com/Mail.Read \
    https://graph.microsoft.com/Mail.ReadWrite \
    https://graph.microsoft.com/Mail.Send \
    https://graph.microsoft.com/Mail.Read.Shared \
    https://graph.microsoft.com/Mail.ReadWrite.Shared \
    https://graph.microsoft.com/Mail.Send.Shared \
    https://graph.microsoft.com/Calendars.ReadWrite \
    https://graph.microsoft.com/User.Read";

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
    // Stable across every account/Azure app — each app registers this exact URI.
    format!("{scheme}://{host}/api/integrations/email/callback")
}

// ── Microsoft 365: per-instance OAuth connect / callback / disconnect ───────────

// GET /api/integrations/:id/connect — start Microsoft OAuth for one M365 instance.
// The registered redirect URI is constant; the CSRF state carries the user +
// which instance is being connected so the callback can route the tokens back.
pub async fn connect(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Redirect> {
    let (_integ, cfg) = microsoft::load(&state, &user.id, Some(&id))
        .await
        .map_err(AppError::Internal)?;
    if cfg.client_id.is_empty() || cfg.tenant_id.is_empty() {
        return Err(AppError::BadRequest("Save tenant and client id first.".into()));
    }

    let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("localhost:3000");
    let redirect_uri = redirect_uri_from_host(host);
    let csrf_state = uuid::Uuid::new_v4().to_string();
    state
        .oauth_state
        .lock()
        .await
        .insert(csrf_state.clone(), format!("{}\t{}", user.id, id));

    let mut auth_url = url::Url::parse(&format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize",
        cfg.tenant_id
    ))
    .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid tenant ID in URL: {e}")))?;
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", &cfg.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", GRAPH_SCOPES)
        .append_pair("state", &csrf_state)
        .append_pair("response_mode", "query");

    Ok(Redirect::temporary(auth_url.as_str()))
}

// GET /api/integrations/email/callback — Microsoft redirects here after login.
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
    let code = params.code.ok_or_else(|| anyhow::anyhow!("no authorization code in callback"))?;

    let binding = {
        let mut states = state.oauth_state.lock().await;
        params
            .state
            .as_deref()
            .and_then(|s| states.remove(s))
            .ok_or_else(|| anyhow::anyhow!("CSRF state mismatch — possible replay attack"))?
    };
    let (user_id, integration_id) = binding
        .split_once('\t')
        .ok_or_else(|| anyhow::anyhow!("malformed oauth state"))?;

    let (integ, mut cfg) = microsoft::load(&state, user_id, Some(integration_id)).await?;

    let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("localhost:3000");
    let redirect_uri = redirect_uri_from_host(host);
    let token_url =
        format!("https://login.microsoftonline.com/{}/oauth2/v2.0/token", cfg.tenant_id);
    let form = [
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
        ("code", code.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("grant_type", "authorization_code"),
    ];
    let token_res: serde_json::Value =
        state.http_client.post(&token_url).form(&form).send().await?.json().await?;
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
    let connected_email =
        me["mail"].as_str().or_else(|| me["userPrincipalName"].as_str()).map(|s| s.to_string());

    // Volatile tokens kept apart from the editable config; connected mailbox
    // recorded on the instance for the table's status + account label.
    microsoft::save_tokens(
        &state,
        &integ.id,
        &MicrosoftTokens {
            access_token: Some(access_token),
            refresh_token,
            token_expires_at: Some(token_expires_at),
        },
    )
    .await?;
    cfg.connected_email = connected_email;
    microsoft::save_config(&state, &integ, &cfg).await?;

    Ok(Redirect::temporary("/?integration=email&status=connected"))
}

// DELETE /api/integrations/:id/connection — disconnect (clear tokens), keep creds.
pub async fn disconnect(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let (integ, mut cfg) =
        microsoft::load(&state, &user.id, Some(&id)).await.map_err(AppError::Internal)?;
    db::settings::delete(&state.db, &microsoft::tokens_key(&id))
        .await
        .map_err(AppError::Internal)?;
    cfg.connected_email = None;
    microsoft::save_config(&state, &integ, &cfg).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Shared mailboxes (per Microsoft 365 instance) ──────────────────────────────
// Graph can't enumerate delegated shared mailboxes, so users add them by address.
// We verify access by opening the target's inbox with this instance's token, then
// remember it on the instance config.

#[derive(Deserialize)]
pub struct AddSharedBody {
    address: String,
    #[serde(default)]
    name: Option<String>,
}

// GET /api/integrations/:id/shared
pub async fn list_shared(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let (_integ, cfg) =
        microsoft::load(&state, &user.id, Some(&id)).await.map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "mailboxes": cfg.shared_mailboxes })))
}

// POST /api/integrations/:id/shared — verify access, then add.
pub async fn add_shared(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<AddSharedBody>,
) -> AppResult<Json<serde_json::Value>> {
    let address = body.address.trim().to_lowercase();
    if address.is_empty()
        || !address.contains('@')
        || address.contains('/')
        || address.contains(char::is_whitespace)
    {
        return Err(AppError::BadRequest("Enter a valid mailbox email address.".into()));
    }
    let (integ, mut cfg) =
        microsoft::load(&state, &user.id, Some(&id)).await.map_err(AppError::Internal)?;

    let token = microsoft::get_valid_token(&state, &user.id, Some(&id))
        .await
        .map_err(|_| AppError::BadRequest("Connect this account first.".into()))?;
    let probe = state
        .http_client
        .get(format!("https://graph.microsoft.com/v1.0/users/{address}/mailFolders/inbox"))
        .query(&[("$select", "id")])
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    if !probe.status().is_success() {
        return Err(AppError::BadRequest(
            "Couldn't open that mailbox — check the address and that you've been granted access."
                .into(),
        ));
    }

    if !cfg.shared_mailboxes.iter().any(|m| m.address.eq_ignore_ascii_case(&address)) {
        cfg.shared_mailboxes.push(microsoft::SharedMailbox {
            address: address.clone(),
            name: body.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
        });
        microsoft::save_config(&state, &integ, &cfg).await.map_err(AppError::Internal)?;
    }
    Ok(Json(serde_json::json!({ "mailboxes": cfg.shared_mailboxes })))
}

// DELETE /api/integrations/:id/shared/:address
pub async fn remove_shared(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path((id, address)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    let (integ, mut cfg) =
        microsoft::load(&state, &user.id, Some(&id)).await.map_err(AppError::Internal)?;
    cfg.shared_mailboxes.retain(|m| !m.address.eq_ignore_ascii_case(address.trim()));
    microsoft::save_config(&state, &integ, &cfg).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Integrations registry (helpdesk / phoneus / github / microsoft) ────────────

use crate::db::integrations as int_db;
use crate::integrations::{github, helpdesk, phoneus};

/// Masked view of an instance — never returns secrets/tokens.
#[derive(Serialize)]
pub struct IntegrationView {
    id: String,
    kind: String,
    name: String,
    is_default: bool,
    /// Account label: helpdesk/phoneus email, GitHub login, or M365 mailbox.
    account: String,
    base_url: String,
    default_owner: String,
    /// Microsoft 365 app identifiers (non-secret), so the edit form can prefill.
    tenant_id: String,
    client_id: String,
    /// Microsoft 365 only: whether the mailbox is currently connected (OAuth).
    connected: bool,
}

fn view(i: &int_db::Integration) -> IntegrationView {
    let cfg: serde_json::Value = serde_json::from_str(&i.config).unwrap_or(serde_json::Value::Null);
    let account = cfg["email"]
        .as_str()
        .or_else(|| cfg["login"].as_str())
        .or_else(|| cfg["connected_email"].as_str())
        .unwrap_or("")
        .to_string();
    IntegrationView {
        id: i.id.clone(),
        kind: i.kind.clone(),
        name: i.name.clone(),
        is_default: i.is_default,
        account,
        base_url: cfg["base_url"].as_str().unwrap_or("").to_string(),
        default_owner: cfg["default_owner"].as_str().unwrap_or("").to_string(),
        tenant_id: cfg["tenant_id"].as_str().unwrap_or("").to_string(),
        client_id: cfg["client_id"].as_str().unwrap_or("").to_string(),
        connected: false,
    }
}

// GET /api/integrations — all the user's integration instances (masked).
pub async fn integrations_list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<serde_json::Value>> {
    let _ = int_db::import_legacy(&state.db, &user.id).await;
    let rows = int_db::list(&state.db, &user.id).await.map_err(AppError::Internal)?;
    let mut views: Vec<IntegrationView> = rows.iter().map(view).collect();
    // Microsoft rows report live connection status from the tokens store.
    for v in views.iter_mut() {
        if v.kind == "microsoft" {
            v.connected = db::settings::get::<MicrosoftTokens>(&state.db, &microsoft::tokens_key(&v.id))
                .await
                .ok()
                .flatten()
                .and_then(|t| t.access_token)
                .is_some();
        }
    }
    Ok(Json(serde_json::json!({ "integrations": views })))
}

#[derive(Deserialize)]
pub struct IntegrationBody {
    kind: String,
    name: String,
    #[serde(default)]
    is_default: bool,
    // Type-specific connect fields.
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    default_owner: Option<String>,
    // Microsoft 365 (Azure app registration).
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
}

/// Run the per-kind credential exchange and return the config JSON to store.
/// On edit (`existing` set) an absent password/token/secret keeps the current one.
async fn build_config(
    state: &AppState,
    kind: &str,
    body: &IntegrationBody,
    existing: Option<&int_db::Integration>,
) -> AppResult<serde_json::Value> {
    match kind {
        "helpdesk" | "phoneus" => {
            let base_url =
                body.base_url.clone().unwrap_or_default().trim().trim_end_matches('/').to_string();
            let email = body.email.clone().unwrap_or_default().trim().to_string();
            if base_url.is_empty() || !base_url.starts_with("http") {
                return Err(AppError::BadRequest("base_url must be a http(s) URL".into()));
            }
            let pw = body.password.clone().unwrap_or_default();
            if pw.is_empty() {
                if let Some(ex) = existing {
                    let mut cfg: serde_json::Value =
                        serde_json::from_str(&ex.config).unwrap_or_else(|_| serde_json::json!({}));
                    cfg["base_url"] = serde_json::json!(base_url);
                    cfg["email"] = serde_json::json!(email);
                    return Ok(cfg);
                }
                return Err(AppError::BadRequest("password is required".into()));
            }
            let token = if kind == "helpdesk" {
                helpdesk::login(state, &base_url, &email, &pw).await
            } else {
                phoneus::login(state, &base_url, &email, &pw).await
            }
            .map_err(AppError::Internal)?;
            Ok(serde_json::json!({ "base_url": base_url, "email": email, "token": token }))
        }
        "github" => {
            let default_owner = body.default_owner.clone().unwrap_or_default().trim().to_string();
            let token = body.token.clone().unwrap_or_default();
            if token.is_empty() {
                if let Some(ex) = existing {
                    let mut cfg: serde_json::Value =
                        serde_json::from_str(&ex.config).unwrap_or_else(|_| serde_json::json!({}));
                    cfg["default_owner"] = serde_json::json!(default_owner);
                    return Ok(cfg);
                }
                return Err(AppError::BadRequest("token is required".into()));
            }
            let login = github::verify(state, &token).await.map_err(AppError::Internal)?;
            Ok(serde_json::json!({ "token": token, "login": login, "default_owner": default_owner }))
        }
        "microsoft" => {
            // Start from existing config on edit so connection (tokens are
            // separate), shared mailboxes and AI-sort config survive a creds edit.
            let mut cfg: MicrosoftConfig =
                existing.and_then(|e| e.parse_config::<MicrosoftConfig>().ok()).unwrap_or_default();
            cfg.tenant_id = body.tenant_id.clone().unwrap_or_default().trim().to_string();
            cfg.client_id = body.client_id.clone().unwrap_or_default().trim().to_string();
            let secret = body.client_secret.clone().unwrap_or_default().trim().to_string();
            if !secret.is_empty() {
                cfg.client_secret = secret;
            } else if existing.is_none() {
                return Err(AppError::BadRequest("client secret is required".into()));
            }
            if cfg.tenant_id.is_empty() || cfg.client_id.is_empty() {
                return Err(AppError::BadRequest("tenant id and client id are required".into()));
            }
            serde_json::to_value(&cfg).map_err(|e| AppError::Internal(e.into()))
        }
        other => Err(AppError::BadRequest(format!("unknown integration kind '{other}'"))),
    }
}

// POST /api/integrations — add an instance (runs the connect/credential exchange).
pub async fn integrations_create(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<IntegrationBody>,
) -> AppResult<Json<IntegrationView>> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    let cfg = build_config(&state, &body.kind, &body, None).await?;
    // First instance of a kind becomes default automatically.
    let first =
        int_db::count_by_kind(&state.db, &user.id, &body.kind).await.map_err(AppError::Internal)? == 0;
    let row = int_db::insert(
        &state.db,
        &user.id,
        &body.kind,
        &name,
        body.is_default || first,
        &cfg.to_string(),
    )
    .await
    .map_err(AppError::Internal)?;
    Ok(Json(view(&row)))
}

// PUT /api/integrations/:id — rename and/or re-authenticate.
pub async fn integrations_update(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<IntegrationBody>,
) -> AppResult<Json<IntegrationView>> {
    let existing = int_db::get(&state.db, &user.id, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    let name = if body.name.trim().is_empty() {
        existing.name.clone()
    } else {
        body.name.trim().to_string()
    };
    let cfg = build_config(&state, &existing.kind, &body, Some(&existing)).await?;
    int_db::update(&state.db, &user.id, &id, &name, Some(&cfg.to_string()))
        .await
        .map_err(AppError::Internal)?;
    if body.is_default {
        int_db::set_default(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    }
    let row = int_db::get(&state.db, &user.id, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    Ok(Json(view(&row)))
}

// DELETE /api/integrations/:id
pub async fn integrations_delete(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    // Clean up any Microsoft tokens for this instance (best-effort).
    let _ = db::settings::delete(&state.db, &microsoft::tokens_key(&id)).await;
    int_db::delete(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// POST /api/integrations/:id/default — make this the default for its kind.
pub async fn integrations_set_default(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    int_db::set_default(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
