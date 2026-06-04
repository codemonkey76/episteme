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
mod prompts;
mod tasks;
mod notes;
mod suggestions;
mod users;
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
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/invite/:code", get(auth::check_invite))
        .route("/api/auth/register", post(auth::register));

    // Everything else requires a valid session cookie.
    let protected = Router::new()
        .route("/api/auth/change-password", post(auth::change_password))
        .route("/api/auth/stop-impersonating", post(auth::stop_impersonating))
        .route("/api/sessions/:id/chat", post(chat::stream))
        .route("/api/sessions", get(sessions::list))
        .route("/api/sessions", post(sessions::create))
        .route("/api/sessions/:id", get(sessions::get))
        .route("/api/sessions/:id", put(sessions::update))
        .route("/api/sessions/:id", delete(sessions::delete))
        .route("/api/sessions/:id/messages", get(sessions::messages))
        // Provider list is readable by everyone (the chat picker needs it,
        // with api keys stripped); managing them is admin-only below.
        .route("/api/settings/providers", get(settings::list_providers))
        .route("/api/settings/timezone", get(settings::get_timezone))
        .route("/api/settings/timezone", post(settings::set_timezone))
        .route("/api/settings/theme", get(settings::get_theme))
        .route("/api/settings/theme", post(settings::set_theme))
        .route("/api/sessions/:id/approvals", get(approvals::list_pending))
        .route("/api/approvals/:action_id/approve", post(approvals::approve))
        .route("/api/approvals/:action_id/reject", post(approvals::reject))
        .route("/api/integrations/email/config", get(integrations::get_config))
        .route("/api/integrations/email/config", post(integrations::save_config))
        .route("/api/integrations/email/config", delete(integrations::disconnect))
        .route("/api/integrations/email/connect", get(integrations::connect))
        .route("/api/integrations/email/callback", get(integrations::callback))
        .route("/api/email/folders", get(email::list_folders))
        .route("/api/email/folders/:id/messages", get(email::list_messages))
        .route("/api/email/folders/:id/read-all", post(email::mark_all_read))
        .route("/api/email/search", get(email::search_messages))
        .route("/api/email/messages/:id", get(email::get_message))
        .route("/api/email/messages/:id/attachments", get(email::list_attachments))
        .route("/api/email/messages/:id/attachments/:att_id/raw", get(email::get_attachment_raw))
        .route("/api/email/messages/:id/read", axum::routing::patch(email::mark_read))
        .route("/api/email/messages/:id/done", post(email::mark_done))
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
        .route("/api/tasks", get(tasks::list))
        .route("/api/tasks", post(tasks::create))
        .route("/api/tasks/:id", put(tasks::update))
        .route("/api/tasks/:id", delete(tasks::delete))
        .route("/api/notes", get(notes::list))
        .route("/api/notes", post(notes::create))
        .route("/api/notes/:id", put(notes::update))
        .route("/api/notes/:id", delete(notes::delete))
        .route("/api/suggestions", get(suggestions::list_pending))
        .route("/api/suggestions/:id/accept", post(suggestions::accept))
        .route("/api/suggestions/:id/dismiss", post(suggestions::dismiss))
        .route("/api/calendar/events", get(calendar::list_events))
        .route("/api/calendar/events", post(calendar::create_event))
        .route("/api/calendar/events/:id", delete(calendar::delete_event))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth::require_auth));

    // Admin-only management routes (auth + role check).
    let admin = Router::new()
        // Shared infrastructure config — admin only (the UI also hides these
        // tabs from members, but the API is the real boundary).
        .route("/api/settings/providers", post(settings::upsert_provider))
        .route("/api/settings/providers/:name", delete(settings::delete_provider))
        .route("/api/settings/ollama/models", get(settings::list_ollama_models))
        .route("/api/settings/mcp-servers", get(settings::list_mcp_servers))
        .route("/api/settings/mcp-servers", post(settings::upsert_mcp_server))
        .route("/api/settings/mcp-servers/status", get(settings::mcp_server_status))
        .route("/api/settings/mcp-servers/:name", delete(settings::delete_mcp_server))
        .route("/api/settings/tools", get(settings::list_tools))
        .route("/api/settings/tools", post(settings::set_tool_policy))
        // Model prompts are instance-wide (every feature's system messages).
        .route("/api/settings/prompts", get(prompts::list))
        .route("/api/settings/prompts/:key", put(prompts::save))
        .route("/api/settings/prompts/:key", delete(prompts::reset))
        // Logs are instance-wide (every user's activity) — admin only.
        .route("/api/logs", post(logs::create))
        .route("/api/logs", get(logs::list))
        .route("/api/logs", delete(logs::clear))
        .route("/api/logs/stream", get(logs::stream))
        .route("/api/users", get(users::list))
        .route("/api/users/:id/impersonate", post(users::impersonate))
        .route("/api/users/:id/disable", post(users::disable))
        .route("/api/users/:id/enable", post(users::enable))
        .route("/api/users/:id", delete(users::delete))
        .route("/api/admin/invites", post(users::create_invite))
        .route("/api/admin/invites", get(users::list_invites))
        .route("/api/admin/invites/:code", delete(users::revoke_invite))
        .route_layer(middleware::from_fn(auth::require_admin))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth::require_auth));

    public
        .merge(protected)
        .merge(admin)
        .layer(cors)
        .with_state(state)
        .fallback_service(
            ServeDir::new(&static_dir)
                .not_found_service(ServeFile::new(format!("{static_dir}/index.html"))),
        )
}
