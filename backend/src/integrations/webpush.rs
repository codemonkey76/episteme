//! Native Web Push (VAPID) — browser notifications, no Firebase involved.
//!
//! VAPID keys are generated once on first use and stored in settings; the
//! frontend fetches the public key, subscribes via the browser's PushManager,
//! and registers the subscription JSON as a `platform = "web"` push token.
//! Pushes go straight to the browser vendor's push service, which fits the
//! self-hosted posture better than the Firebase JS SDK would.

use anyhow::{anyhow, Result};
use base64::Engine;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use web_push::{
    ContentEncoding, HyperWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
    WebPushError, WebPushMessageBuilder,
};

use crate::state::AppState;

const SETTINGS_KEY: &str = "vapid_keys";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VapidKeys {
    /// Uncompressed P-256 public point, base64url — the browser's
    /// `applicationServerKey`.
    public: String,
    /// Raw 32-byte private scalar, base64url (the web-push key format).
    private: String,
}

/// Load the instance's VAPID keypair, generating and persisting one on first
/// use — web push needs zero manual setup.
async fn keys(pool: &SqlitePool) -> Result<VapidKeys> {
    if let Some(k) = crate::db::settings::get::<VapidKeys>(pool, SETTINGS_KEY).await? {
        return Ok(k);
    }
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    // Rejection-sample the scalar: from_slice fails on the (astronomically
    // rare) bytes outside the curve order, so just draw again.
    let secret = loop {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| anyhow!("rng failure: {e}"))?;
        if let Ok(s) = p256::SecretKey::from_slice(&bytes) {
            break s;
        }
    };
    let generated = VapidKeys {
        public: b64.encode(secret.public_key().to_encoded_point(false).as_bytes()),
        private: b64.encode(secret.to_bytes()),
    };
    crate::db::settings::set(pool, SETTINGS_KEY, &generated).await?;
    tracing::info!("generated VAPID keypair for web push");
    Ok(generated)
}

/// The public key the frontend passes to `pushManager.subscribe`.
pub async fn public_key(pool: &SqlitePool) -> Result<String> {
    keys(pool).await.map(|k| k.public)
}

/// Whether this send failed because the subscription is gone.
fn is_dead(e: &WebPushError) -> bool {
    matches!(e, WebPushError::EndpointNotValid(_) | WebPushError::EndpointNotFound(_))
}

async fn send(keys: &VapidKeys, subscription_json: &str, payload: &str) -> Result<(), WebPushError> {
    let sub: SubscriptionInfo = serde_json::from_str(subscription_json)
        .map_err(|_| WebPushError::InvalidCryptoKeys)?;
    let signature = VapidSignatureBuilder::from_base64(&keys.private, &sub)?.build()?;
    let mut builder = WebPushMessageBuilder::new(&sub);
    builder.set_vapid_signature(signature);
    builder.set_payload(ContentEncoding::Aes128Gcm, payload.as_bytes());
    HyperWebPushClient::new().send(builder.build()?).await
}

/// Push a notification to all of a user's browser subscriptions, pruning dead
/// ones. Best-effort like FCM: logs failures, never errors out.
pub async fn notify(state: &AppState, user_id: &str, title: &str, body: &str) {
    let subs = match crate::db::push_tokens::list_web(&state.db, user_id).await {
        Ok(s) if !s.is_empty() => s,
        _ => return,
    };
    let keys = match keys(&state.db).await {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!("web push disabled: {e}");
            return;
        }
    };
    // Same glanceable-body clip as FCM.
    let body: String = body.chars().take(180).collect();
    let payload = serde_json::json!({ "title": title, "body": body }).to_string();
    for sub in subs {
        match send(&keys, &sub, &payload).await {
            Ok(()) => {}
            Err(e) if is_dead(&e) => {
                let _ = crate::db::push_tokens::remove(&state.db, &sub).await;
                tracing::info!("pruned dead web push subscription");
            }
            Err(e) => tracing::warn!("web push failed: {e}"),
        }
    }
}
