use serde::{Deserialize, Serialize};

/// Legacy shared Azure app key (pre per-user config) — formerly one app
/// registration for the whole instance. Migrated at startup to each connected
/// user's own key, then removed.
pub const KEY_MICROSOFT_APP: &str = "microsoft_app";

/// Legacy single-user key (pre multi-user); migrated at startup.
pub const KEY_LEGACY: &str = "microsoft_email";

/// Per-user Azure app registration (tenant/client/secret). Each user brings
/// their own registration so mailboxes connect independently.
pub fn app_key(user_id: &str) -> String {
    format!("microsoft_app:{user_id}")
}

/// Per-user OAuth tokens + connected mailbox.
pub fn user_key(user_id: &str) -> String {
    format!("microsoft_email:{user_id}")
}

/// Per-user list of shared mailboxes the user has added (and we verified they
/// can access). Selecting one targets `/users/{address}/…` instead of `/me/…`.
pub fn shared_key(user_id: &str) -> String {
    format!("microsoft_shared:{user_id}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedMailbox {
    pub address: String,
    /// Optional friendly label; falls back to the address in the UI.
    #[serde(default)]
    pub name: Option<String>,
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

// ── Instance-based model ───────────────────────────────────────────────────────
// Each connected O365 account is an `integrations` row (kind "microsoft"). The
// row's `config` JSON holds app creds + connected mailbox + shared mailboxes +
// AI-sort config; the volatile OAuth tokens live in a separate settings key
// (`microsoft_tokens:{integration_id}`) so token refreshes never clobber a
// concurrent config edit (e.g. the categorizer worker saving state).

use crate::db::integrations::Integration;
use crate::state::AppState;

/// Per-instance OAuth token store key.
pub fn tokens_key(integration_id: &str) -> String {
    format!("microsoft_tokens:{integration_id}")
}

/// The `config` JSON of a `microsoft` integration row.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MicrosoftConfig {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: String,
    /// The mailbox this instance is connected as (set on OAuth callback).
    pub connected_email: Option<String>,
    pub shared_mailboxes: Vec<SharedMailbox>,
    /// AI auto-sort config for this account (own + shared mailboxes).
    pub categorizer: crate::categorizer::CategorizerConfig,
}

/// Volatile OAuth tokens, stored apart from the editable config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MicrosoftTokens {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<i64>,
}

/// Resolve the microsoft instance for this user. `account` selects by id/name;
/// absent falls back to the sole/default instance (see `registry::resolve`).
pub async fn resolve(
    state: &AppState,
    user_id: &str,
    account: Option<&str>,
) -> anyhow::Result<Integration> {
    crate::integrations::registry::resolve(&state.db, user_id, "microsoft", account).await
}

/// Resolve the instance and parse its config.
pub async fn load(
    state: &AppState,
    user_id: &str,
    account: Option<&str>,
) -> anyhow::Result<(Integration, MicrosoftConfig)> {
    let integ = resolve(state, user_id, account).await?;
    let cfg = integ.parse_config::<MicrosoftConfig>().unwrap_or_default();
    Ok((integ, cfg))
}

/// Persist an instance's config JSON (preserving its name).
pub async fn save_config(
    state: &AppState,
    integ: &Integration,
    cfg: &MicrosoftConfig,
) -> anyhow::Result<()> {
    let json = serde_json::to_string(cfg)?;
    crate::db::integrations::update(&state.db, &integ.user_id, &integ.id, &integ.name, Some(&json))
        .await
}

pub async fn save_tokens(
    state: &AppState,
    integration_id: &str,
    tokens: &MicrosoftTokens,
) -> anyhow::Result<()> {
    crate::db::settings::set(&state.db, &tokens_key(integration_id), tokens).await
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

/// Migrate the former shared Azure app (single `microsoft_app` key, used by
/// every member) to per-user config: copy it into the app key of each user who
/// is currently connected, so their mailbox keeps refreshing, then drop the
/// shared key. Users who never connected start fresh with their own app.
/// Idempotent — once the shared key is gone this returns immediately.
pub async fn migrate_shared_to_per_user(state: &crate::state::AppState) {
    use crate::db;
    let Ok(Some(shared)) =
        db::settings::get::<MicrosoftAppConfig>(&state.db, KEY_MICROSOFT_APP).await
    else {
        return;
    };
    if !shared.client_id.is_empty() {
        let users = db::auth::list_users(&state.db).await.unwrap_or_default();
        for u in users {
            let connected = matches!(
                db::settings::get::<MicrosoftUserTokens>(&state.db, &user_key(&u.id)).await,
                Ok(Some(t)) if t.access_token.is_some()
            );
            let has_app = matches!(
                db::settings::get::<MicrosoftAppConfig>(&state.db, &app_key(&u.id)).await,
                Ok(Some(c)) if !c.client_id.is_empty()
            );
            if connected && !has_app {
                let _ = db::settings::set(&state.db, &app_key(&u.id), &shared).await;
            }
        }
        tracing::info!("migrated shared Microsoft app credentials to per-user config");
    }
    let _ = db::settings::delete(&state.db, KEY_MICROSOFT_APP).await;
}

/// Returns a valid access token for the given user's selected microsoft
/// instance, refreshing transparently when it's expired or close to expiry.
/// `account` selects the instance (None = default/sole).
pub async fn get_valid_token(
    state: &AppState,
    user_id: &str,
    account: Option<&str>,
) -> anyhow::Result<String> {
    let (integ, cfg) = load(state, user_id, account).await?;
    get_valid_token_for(state, &integ.id, &cfg).await
}

/// Token resolution for an already-resolved instance.
pub async fn get_valid_token_for(
    state: &AppState,
    integration_id: &str,
    cfg: &MicrosoftConfig,
) -> anyhow::Result<String> {
    use crate::db;

    let key = tokens_key(integration_id);
    let tokens = db::settings::get::<MicrosoftTokens>(&state.db, &key)
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

    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        cfg.tenant_id
    );

    let form = [
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
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

    let updated = MicrosoftTokens {
        access_token: Some(new_access.clone()),
        refresh_token: new_refresh,
        token_expires_at: Some(now + expires_in),
    };

    db::settings::set(&state.db, &key, &updated).await?;

    Ok(new_access)
}

/// One-time migration folding the legacy single-connection M365 (per-user
/// settings keys) into a default `microsoft` integration instance, so existing
/// users keep working with no reconnect. Runs after `migrate_legacy` /
/// `migrate_shared_to_per_user` have populated the per-user app/token keys.
/// Idempotent: skips any user who already has a `microsoft` instance.
pub async fn migrate_email_to_integration(state: &AppState) {
    use crate::db;

    let users = db::auth::list_users(&state.db).await.unwrap_or_default();
    for u in users {
        if db::integrations::count_by_kind(&state.db, &u.id, "microsoft").await.unwrap_or(0) > 0 {
            continue;
        }
        // Only migrate users who actually configured an Azure app.
        let app = match db::settings::get::<MicrosoftAppConfig>(&state.db, &app_key(&u.id)).await {
            Ok(Some(a)) if !a.client_id.is_empty() => a,
            _ => continue,
        };
        let legacy_tokens = db::settings::get::<MicrosoftUserTokens>(&state.db, &user_key(&u.id))
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let shared: Vec<SharedMailbox> = db::settings::get(&state.db, &shared_key(&u.id))
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let categorizer = crate::categorizer::get_config(&state.db, &u.id)
            .await
            .unwrap_or_default();

        let cfg = MicrosoftConfig {
            tenant_id: app.tenant_id,
            client_id: app.client_id,
            client_secret: app.client_secret,
            connected_email: legacy_tokens.connected_email.clone(),
            shared_mailboxes: shared.clone(),
            categorizer,
        };
        let json = match serde_json::to_string(&cfg) {
            Ok(j) => j,
            Err(_) => continue,
        };
        let integ = match db::integrations::insert(
            &state.db,
            &u.id,
            "microsoft",
            "Microsoft 365",
            true,
            &json,
        )
        .await
        {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!("M365 migration failed for {}: {e}", u.id);
                continue;
            }
        };

        // Carry over the live tokens so the mailbox keeps working.
        if legacy_tokens.access_token.is_some() {
            let mt = MicrosoftTokens {
                access_token: legacy_tokens.access_token,
                refresh_token: legacy_tokens.refresh_token,
                token_expires_at: legacy_tokens.token_expires_at,
            };
            let _ = save_tokens(state, &integ.id, &mt).await;
        }

        // Carry over the categorizer's processed-id state (own + shared) so the
        // first post-migration run doesn't re-flag already-handled mail.
        crate::categorizer::migrate_state_owner(
            &state.db,
            &u.id,
            &integ.id,
            shared.iter().map(|m| m.address.as_str()),
        )
        .await;

        // Drop the legacy keys now they're folded in.
        let _ = db::settings::delete(&state.db, &app_key(&u.id)).await;
        let _ = db::settings::delete(&state.db, &user_key(&u.id)).await;
        let _ = db::settings::delete(&state.db, &shared_key(&u.id)).await;
        tracing::info!("migrated Microsoft 365 connection for {} into an integration", u.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::state::AppState;

    async fn test_state() -> AppState {
        let pool = db::init("sqlite::memory:").await.expect("migrations should run");
        sqlx::query("INSERT INTO auth_users (id, username, password_hash, role, created_at) VALUES ('u1','t','x','admin','2026-01-01')")
            .execute(&pool).await.unwrap();
        AppState::new(pool).0
    }

    // A microsoft instance's config round-trips through the integrations row, and
    // resolve()/load() find it by id and as the default.
    #[tokio::test]
    async fn instance_config_round_trip() {
        let state = test_state().await;
        let cfg = MicrosoftConfig {
            tenant_id: "tid".into(),
            client_id: "cid".into(),
            client_secret: "sec".into(),
            connected_email: Some("me@corp.com".into()),
            shared_mailboxes: vec![SharedMailbox { address: "support@corp.com".into(), name: None }],
            ..Default::default()
        };
        let row = db::integrations::insert(
            &state.db, "u1", "microsoft", "Work", true, &serde_json::to_string(&cfg).unwrap(),
        ).await.unwrap();

        // Resolve by id and as the sole/default instance.
        let (by_default, loaded) = load(&state, "u1", None).await.unwrap();
        assert_eq!(by_default.id, row.id);
        assert_eq!(loaded.connected_email.as_deref(), Some("me@corp.com"));
        assert_eq!(loaded.shared_mailboxes.len(), 1);
        let (by_id, _) = load(&state, "u1", Some(&row.id)).await.unwrap();
        assert_eq!(by_id.id, row.id);

        // save_config persists edits.
        let (integ, mut cfg2) = load(&state, "u1", None).await.unwrap();
        cfg2.shared_mailboxes.push(SharedMailbox { address: "team@corp.com".into(), name: Some("Team".into()) });
        save_config(&state, &integ, &cfg2).await.unwrap();
        let (_i, reread) = load(&state, "u1", None).await.unwrap();
        assert_eq!(reread.shared_mailboxes.len(), 2);
    }

    // The legacy single-connection keys fold into a default instance with tokens,
    // shared mailboxes and AI-sort carried over, and the legacy keys removed.
    #[tokio::test]
    async fn migration_folds_legacy_keys() {
        let state = test_state().await;
        db::settings::set(&state.db, &app_key("u1"), &MicrosoftAppConfig {
            tenant_id: "tid".into(), client_id: "cid".into(), client_secret: "sec".into(),
        }).await.unwrap();
        db::settings::set(&state.db, &user_key("u1"), &MicrosoftUserTokens {
            access_token: Some("at".into()),
            refresh_token: Some("rt".into()),
            token_expires_at: Some(99),
            connected_email: Some("me@corp.com".into()),
        }).await.unwrap();
        db::settings::set(&state.db, &shared_key("u1"), &vec![
            SharedMailbox { address: "support@corp.com".into(), name: None },
        ]).await.unwrap();

        migrate_email_to_integration(&state).await;

        // One default microsoft instance with the folded config.
        let rows = db::integrations::list_by_kind(&state.db, "u1", "microsoft").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_default);
        let cfg: MicrosoftConfig = rows[0].parse_config().unwrap();
        assert_eq!(cfg.client_id, "cid");
        assert_eq!(cfg.connected_email.as_deref(), Some("me@corp.com"));
        assert_eq!(cfg.shared_mailboxes.len(), 1);

        // Tokens carried into the per-instance store.
        let tokens: MicrosoftTokens =
            db::settings::get(&state.db, &tokens_key(&rows[0].id)).await.unwrap().unwrap();
        assert_eq!(tokens.access_token.as_deref(), Some("at"));

        // Legacy keys removed; migration is idempotent.
        assert!(db::settings::get::<MicrosoftAppConfig>(&state.db, &app_key("u1")).await.unwrap().is_none());
        migrate_email_to_integration(&state).await;
        assert_eq!(db::integrations::count_by_kind(&state.db, "u1", "microsoft").await.unwrap(), 1);
    }
}
