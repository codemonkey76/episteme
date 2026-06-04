use serde::{Deserialize, Serialize};

/// Shared Azure app registration (tenant/client/secret) — one per instance,
/// managed by the admin. Members connect their own mailboxes through it.
pub const KEY_MICROSOFT_APP: &str = "microsoft_app";

/// Legacy single-user key (pre multi-user); migrated at startup.
pub const KEY_LEGACY: &str = "microsoft_email";

/// Per-user OAuth tokens + connected mailbox.
pub fn user_key(user_id: &str) -> String {
    format!("microsoft_email:{user_id}")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MicrosoftAppConfig {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MicrosoftUserTokens {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<i64>,
    pub connected_email: Option<String>,
}

pub async fn app_config(state: &crate::state::AppState) -> anyhow::Result<MicrosoftAppConfig> {
    crate::db::settings::get::<MicrosoftAppConfig>(&state.db, KEY_MICROSOFT_APP)
        .await?
        .filter(|c| !c.client_id.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Microsoft integration not configured"))
}

/// One-time startup migration from the single-user era: split the shared app
/// credentials out of the legacy config; tokens were already copied to the
/// admin's per-user key by the SQL migration.
pub async fn migrate_legacy(state: &crate::state::AppState) {
    use crate::db;
    let Ok(Some(legacy)) = db::settings::get::<serde_json::Value>(&state.db, KEY_LEGACY).await
    else {
        return;
    };
    let app = MicrosoftAppConfig {
        tenant_id: legacy["tenant_id"].as_str().unwrap_or_default().to_string(),
        client_id: legacy["client_id"].as_str().unwrap_or_default().to_string(),
        client_secret: legacy["client_secret"].as_str().unwrap_or_default().to_string(),
    };
    if !app.client_id.is_empty() {
        let _ = db::settings::set(&state.db, KEY_MICROSOFT_APP, &app).await;
    }
    let _ = db::settings::delete(&state.db, KEY_LEGACY).await;
    tracing::info!("migrated Microsoft app credentials to shared config");
}

/// Returns a valid access token for the given user, refreshing transparently
/// when it's expired or close to expiry.
pub async fn get_valid_token(
    state: &crate::state::AppState,
    user_id: &str,
) -> anyhow::Result<String> {
    use crate::db;

    let key = user_key(user_id);
    let tokens = db::settings::get::<MicrosoftUserTokens>(&state.db, &key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("not_connected"))?;

    let access_token = tokens
        .access_token
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("not_connected"))?;

    let now = chrono::Utc::now().timestamp();
    // Treat the token as still valid if it has more than 5 minutes left.
    if tokens.token_expires_at.unwrap_or(0) > now + 300 {
        return Ok(access_token.to_string());
    }

    let refresh_token = tokens
        .refresh_token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not_connected"))?;

    let app = app_config(state).await?;
    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        app.tenant_id
    );

    let form = [
        ("client_id", app.client_id.as_str()),
        ("client_secret", app.client_secret.as_str()),
        ("refresh_token", refresh_token.as_str()),
        ("grant_type", "refresh_token"),
    ];

    let res: serde_json::Value = state
        .http_client
        .post(&token_url)
        .form(&form)
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = res["error"].as_str() {
        let desc = res["error_description"].as_str().unwrap_or("");
        anyhow::bail!("token refresh failed: {err} — {desc}");
    }

    let new_access = res["access_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no access_token in refresh response"))?
        .to_string();

    let new_refresh = res["refresh_token"]
        .as_str()
        .map(|s| s.to_string())
        .or(Some(refresh_token));

    let expires_in = res["expires_in"].as_i64().unwrap_or(3600);

    let updated = MicrosoftUserTokens {
        access_token: Some(new_access.clone()),
        refresh_token: new_refresh,
        token_expires_at: Some(now + expires_in),
        connected_email: tokens.connected_email,
    };

    db::settings::set(&state.db, &key, &updated).await?;

    Ok(new_access)
}
