use std::sync::Arc;

use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

use crate::state::AppState;

mod approvals;
pub(crate) mod auth;
mod calendar;
mod chat;
pub(crate) mod email;
mod integrations;
mod logs;
mod memories;
mod sessions;
mod settings;

pub fn router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let static_dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string());

    // Public auth endpoints — reachable without a session so the user can set
    // up an account, log in, and check status.
    let public = Router::new()
        .route("/api/auth/status", get(auth::status))
        .route("/api/auth/setup", post(auth::setup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout));

    // Everything else requires a valid session cookie.
    let protected = Router::new()
        .route("/api/auth/change-password", post(auth::change_password))
        .route("/api/sessions/:id/chat", post(chat::stream))
        .route("/api/sessions", get(sessions::list))
        .route("/api/sessions", post(sessions::create))
        .route("/api/sessions/:id", get(sessions::get))
        .route("/api/sessions/:id", put(sessions::update))
        .route("/api/sessions/:id", delete(sessions::delete))
        .route("/api/sessions/:id/messages", get(sessions::messages))
        .route("/api/settings/providers", get(settings::list_providers))
        .route("/api/settings/providers", post(settings::upsert_provider))
        .route("/api/settings/providers/:name", delete(settings::delete_provider))
        .route("/api/settings/ollama/models", get(settings::list_ollama_models))
        .route("/api/settings/mcp-servers", get(settings::list_mcp_servers))
        .route("/api/settings/mcp-servers", post(settings::upsert_mcp_server))
        .route("/api/settings/mcp-servers/status", get(settings::mcp_server_status))
        .route("/api/settings/timezone", get(settings::get_timezone))
        .route("/api/settings/timezone", post(settings::set_timezone))
        .route("/api/settings/mcp-servers/:name", delete(settings::delete_mcp_server))
        .route("/api/sessions/:id/approvals", get(approvals::list_pending))
        .route("/api/approvals/:action_id/approve", post(approvals::approve))
        .route("/api/approvals/:action_id/reject", post(approvals::reject))
        .route("/api/integrations/email/config", get(integrations::get_config))
        .route("/api/integrations/email/config", post(integrations::save_config))
        .route("/api/integrations/email/config", delete(integrations::disconnect))
        .route("/api/integrations/email/connect", get(integrations::connect))
        .route("/api/integrations/email/callback", get(integrations::callback))
        .route("/api/logs", post(logs::create))
        .route("/api/logs", get(logs::list))
        .route("/api/logs", delete(logs::clear))
        .route("/api/logs/stream", get(logs::stream))
        .route("/api/email/folders", get(email::list_folders))
        .route("/api/email/folders/:id/messages", get(email::list_messages))
        .route("/api/email/search", get(email::search_messages))
        .route("/api/email/messages/:id", get(email::get_message))
        .route("/api/email/messages/:id/attachments", get(email::list_attachments))
        .route("/api/email/messages/:id/attachments/:att_id/raw", get(email::get_attachment_raw))
        .route("/api/email/messages/:id/read", axum::routing::patch(email::mark_read))
        .route("/api/email/send", post(email::send_email))
        .route("/api/email/ai-draft", post(email::ai_draft))
        .route("/api/email/messages/:id/advise", post(email::advise))
        .route("/api/email/categorizer", get(email::get_categorizer))
        .route("/api/email/categorizer", put(email::put_categorizer))
        .route("/api/email/categorizer/run", post(email::run_categorizer))
        .route("/api/memories", get(memories::list))
        .route("/api/memories", post(memories::create))
        .route("/api/memories/:id", put(memories::update))
        .route("/api/memories/:id", delete(memories::delete))
        .route("/api/calendar/events", get(calendar::list_events))
        .route("/api/calendar/events", post(calendar::create_event))
        .route("/api/calendar/events/:id", delete(calendar::delete_event))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth::require_auth));

    public
        .merge(protected)
        .layer(cors)
        .with_state(state)
        .fallback_service(
            ServeDir::new(&static_dir)
                .not_found_service(ServeFile::new(format!("{static_dir}/index.html"))),
        )
}
