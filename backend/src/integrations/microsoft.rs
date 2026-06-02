use serde::{Deserialize, Serialize};

pub const KEY_MICROSOFT_EMAIL: &str = "microsoft_email";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MicrosoftEmailConfig {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: String,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<i64>,
    pub connected_email: Option<String>,
}

/// Returns a valid access token, refreshing it transparently if it's expired or close to expiry.
pub async fn get_valid_token(state: &crate::state::AppState) -> anyhow::Result<String> {
    use crate::db;

    let config = db::settings::get::<MicrosoftEmailConfig>(&state.db, KEY_MICROSOFT_EMAIL)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Microsoft email integration not configured"))?;

    let access_token = config
        .access_token
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("not_connected"))?;

    let now = chrono::Utc::now().timestamp();
    // Treat the token as still valid if it has more than 5 minutes left.
    if config.token_expires_at.unwrap_or(0) > now + 300 {
        return Ok(access_token.to_string());
    }

    let refresh_token = config
        .refresh_token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not_connected"))?;

    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        config.tenant_id
    );

    let form = [
        ("client_id", config.client_id.as_str()),
        ("client_secret", config.client_secret.as_str()),
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

    let updated = MicrosoftEmailConfig {
        access_token: Some(new_access.clone()),
        refresh_token: new_refresh,
        token_expires_at: Some(now + expires_in),
        ..config
    };

    db::settings::set(&state.db, KEY_MICROSOFT_EMAIL, &updated).await?;

    Ok(new_access)
}
