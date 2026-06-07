//! One-call notification fan-out: every channel the user has registered —
//! FCM (mobile app) and Web Push (browsers). Call sites notify here instead
//! of picking a transport; each channel no-ops when unconfigured.

use crate::state::AppState;

pub async fn notify(state: &AppState, user_id: &str, title: &str, body: &str) {
    super::fcm::notify(state, user_id, title, body).await;
    super::webpush::notify(state, user_id, title, body).await;
}
