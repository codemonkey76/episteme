//! One-call notification fan-out: every channel the user has registered —
//! FCM (mobile app) and Web Push (browsers). Call sites notify here instead
//! of picking a transport; each channel no-ops when unconfigured.
//!
//! Every notification is also recorded in the `notifications` table so the user
//! can review past ones in the Notifications window (push delivery itself stays
//! best-effort). Use [`notify_linked`] to attach an associated item the UI can
//! deep-link to (e.g. the chat session a scheduled agent wrote to).

use crate::db;
use crate::state::AppState;

/// An item a notification points at, so the UI can open it on click.
/// `kind` is currently always `"session"`; `id` is that item's id.
pub struct Link<'a> {
    pub kind: &'a str,
    pub id: &'a str,
}

/// Notify with a category and an optional associated item.
pub async fn notify_linked(
    state: &AppState,
    user_id: &str,
    title: &str,
    body: &str,
    category: &str,
    link: Option<Link<'_>>,
) {
    let (lk, li) = match &link {
        Some(l) => (Some(l.kind), Some(l.id)),
        None => (None, None),
    };
    // Record first so it survives even if push delivery fails; a failure here
    // shouldn't block delivery, so it's logged and swallowed.
    if let Err(e) = db::notifications::insert(&state.db, user_id, category, title, body, lk, li).await {
        tracing::warn!("failed to record notification: {e}");
    }
    super::fcm::notify(state, user_id, title, body).await;
    super::webpush::notify(state, user_id, title, body).await;
}

/// Notify with no associated item, category `info`.
#[allow(dead_code)] // kept as the simple entry point for future call sites
pub async fn notify(state: &AppState, user_id: &str, title: &str, body: &str) {
    notify_linked(state, user_id, title, body, "info", None).await;
}
