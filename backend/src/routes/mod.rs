use std::sync::Arc;

use axum::{
    http::{header, HeaderValue},
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::state::AppState;

mod approvals;
pub(crate) mod auth;
mod calendar;
mod chat;
mod documents;
pub(crate) mod email;
mod integrations;
mod jobs;
mod logs;
mod memories;
mod prompts;
mod reports;
mod scheduler;
mod transcribe;
mod tasks;
mod notes;
mod writer;
mod notifications;
mod suggestions;
mod users;
mod sessions;
mod settings;
mod terminals;

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
        .route("/api/auth/register", post(auth::register))
        // Public report share links — the token IS the capability, no session.
        .route("/shared/:token", get(reports::shared));

    // Everything else requires a valid session cookie.
    let protected = Router::new()
        .route("/api/auth/change-password", post(auth::change_password))
        .route("/api/auth/2fa", get(auth::totp_status))
        .route("/api/auth/2fa/setup", post(auth::totp_setup))
        .route("/api/auth/2fa/enable", post(auth::totp_enable))
        .route("/api/auth/2fa/disable", post(auth::totp_disable))
        .route("/api/auth/stop-impersonating", post(auth::stop_impersonating))
        .route(
            "/api/sessions/:id/chat",
            // Multimodal messages carry base64 images (4 × 8 MB b64 max).
            post(chat::stream).layer(axum::extract::DefaultBodyLimit::max(40 * 1024 * 1024)),
        )
        .route("/api/sessions/search", get(sessions::search))
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
        .route("/api/approvals/pending", get(approvals::list_all_pending))
        .route("/api/jobs", get(jobs::list))
        .route("/api/jobs/:id/cancel", post(jobs::cancel))
        .route("/api/jobs/:id/retry", post(jobs::retry))
        .route("/api/research", post(reports::start_research))
        .route("/api/reports", get(reports::list))
        .route("/api/reports/:id/html", get(reports::html))
        .route("/api/reports/:id/share", post(reports::share))
        .route("/api/reports/:id/share", delete(reports::unshare))
        .route("/api/reports/:id", delete(reports::delete))
        .route("/api/approvals/:action_id/approve", post(approvals::approve))
        .route("/api/approvals/:action_id/reject", post(approvals::reject))
        // Microsoft OAuth callback — stable URI registered in every Azure app.
        .route("/api/integrations/email/callback", get(integrations::callback))
        .route("/api/integrations", get(integrations::integrations_list))
        .route("/api/integrations", post(integrations::integrations_create))
        .route("/api/integrations/:id", put(integrations::integrations_update))
        .route("/api/integrations/:id", delete(integrations::integrations_delete))
        .route("/api/integrations/:id/default", post(integrations::integrations_set_default))
        // Microsoft 365 per-instance connection + shared mailboxes.
        .route("/api/integrations/:id/connect", get(integrations::connect))
        .route("/api/integrations/:id/connection", delete(integrations::disconnect))
        .route("/api/integrations/:id/shared", get(integrations::list_shared))
        .route("/api/integrations/:id/shared", post(integrations::add_shared))
        .route("/api/integrations/:id/shared/:address", delete(integrations::remove_shared))
        .route("/api/email/folders", get(email::list_folders))
        .route("/api/email/folders/:id/messages", get(email::list_messages))
        .route("/api/email/folders/:id/read-all", post(email::mark_all_read))
        .route("/api/email/search", get(email::search_messages))
        .route("/api/email/messages/:id", get(email::get_message))
        .route("/api/email/messages/:id/attachments", get(email::list_attachments))
        .route("/api/email/messages/:id/attachments/:att_id/raw", get(email::get_attachment_raw))
        .route("/api/email/messages/:id/read", axum::routing::patch(email::mark_read))
        .route("/api/email/messages/:id/done", post(email::mark_done))
        .route("/api/email/messages/delete", post(email::delete_messages))
        .route("/api/email/messages/move", post(email::move_messages))
        // Send carries base64 attachments — allow well past the default 2 MB
        // body cap (Graph's message limit is ~35 MB raw ≈ 47 MB base64).
        .route(
            "/api/email/send",
            post(email::send_email).layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route("/api/email/signatures", get(email::get_signatures))
        .route("/api/email/signatures", put(email::put_signatures))
        .route("/api/email/ai-draft", post(email::ai_draft))
        .route("/api/email/improve", post(email::improve))
        .route("/api/email/messages/:id/summarize", post(email::summarize))
        .route("/api/email/messages/:id/ticket/preview", post(email::ticket_preview))
        .route("/api/email/tickets", post(email::ticket_create))
        .route("/api/email/messages/:id/advise", post(email::advise))
        .route("/api/email/categorizer", get(email::get_categorizer))
        .route("/api/email/categorizer", put(email::put_categorizer))
        .route("/api/email/categorizer/run", post(email::run_categorizer))
        .route("/api/documents", get(documents::list))
        .route(
            "/api/documents",
            post(documents::create).layer(axum::extract::DefaultBodyLimit::max(40 * 1024 * 1024)),
        )
        .route("/api/documents/:id", delete(documents::delete))
        .route("/api/memories", get(memories::list))
        .route("/api/memories", post(memories::create))
        .route("/api/memories/consolidate", post(memories::consolidate))
        .route("/api/memories/deleted", get(memories::list_deleted))
        .route("/api/memories/:id", put(memories::update))
        .route("/api/memories/:id", delete(memories::delete))
        .route("/api/memories/:id/restore", post(memories::restore))
        .route("/api/tasks", get(tasks::list))
        .route("/api/tasks", post(tasks::create))
        .route("/api/tasks/lists", get(tasks::list_lists))
        .route("/api/tasks/lists", post(tasks::create_list))
        .route("/api/tasks/lists/:id", put(tasks::rename_list))
        .route("/api/tasks/lists/:id", delete(tasks::delete_list))
        .route("/api/tasks/:id", put(tasks::update))
        .route("/api/tasks/:id", delete(tasks::delete))
        .route("/api/notes", get(notes::list))
        .route("/api/notes", post(notes::create))
        .route("/api/notes/:id", put(notes::update))
        .route("/api/notes/:id", delete(notes::delete))
        .route("/api/writer/docs", get(writer::list))
        .route("/api/writer/docs", post(writer::create))
        .route("/api/writer/docs/:id", put(writer::update))
        .route("/api/writer/docs/:id", delete(writer::delete))
        .route("/api/writer/improve", post(writer::improve))
        .route("/api/notifications", get(notifications::list))
        .route("/api/notifications", delete(notifications::clear_all))
        .route("/api/notifications/read-all", post(notifications::mark_all_read))
        .route("/api/notifications/:id/read", post(notifications::mark_read))
        .route("/api/notifications/:id", delete(notifications::delete))
        .route("/api/suggestions", get(suggestions::list_pending))
        .route("/api/suggestions/:id/accept", post(suggestions::accept))
        .route("/api/suggestions/:id/dismiss", post(suggestions::dismiss))
        .route("/api/scheduled-agents", get(scheduler::list))
        .route("/api/scheduled-agents", put(scheduler::put))
        .route("/api/scheduled-agents/:id/run", post(scheduler::run))
        .route("/api/push/register", post(scheduler::register_push))
        .route("/api/push/vapid", get(scheduler::vapid_public))
        .route(
            "/api/transcribe",
            post(transcribe::transcribe).layer(axum::extract::DefaultBodyLimit::max(24 * 1024 * 1024)),
        )
        .route("/api/calendar/events", get(calendar::list_events))
        .route("/api/calendar/events", post(calendar::create_event))
        .route("/api/calendar/events/:id", delete(calendar::delete_event))
        .route("/api/terminals/ws", get(terminals::ws))
        .route("/api/terminals/history", get(terminals::list_history))
        .route("/api/terminals/history", post(terminals::record_history))
        .route("/api/terminals/output/search", get(terminals::search_output))
        .route("/api/terminals/agent", post(terminals::agent))
        .route("/api/terminals/agent/decide", post(terminals::agent_decide))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth::require_auth));

    // Admin-only management routes (auth + role check).
    let admin = Router::new()
        // Shared infrastructure config — admin only (the UI also hides these
        // tabs from members, but the API is the real boundary).
        .route("/api/usage/summary", get(settings::usage_summary))
        .route("/api/settings/providers", post(settings::upsert_provider))
        .route("/api/settings/providers/:name", delete(settings::delete_provider))
        .route("/api/settings/model-prices", get(settings::get_model_prices))
        .route("/api/settings/model-prices", put(settings::set_model_prices))
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

    // Content-hashed bundles (/assets/index-<hash>.js etc.) are immutable for a
    // given name, so cache them for a year. Everything else under the SPA
    // fallback — index.html, sw.js, icons — is served `no-cache` (revalidate on
    // every load) so a new deploy is picked up immediately instead of the
    // browser serving a stale app shell until the user hard-refreshes.
    let assets_service = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ))
        .service(ServeDir::new(format!("{static_dir}/assets")));
    let spa_service = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .service(
            ServeDir::new(&static_dir)
                .not_found_service(ServeFile::new(format!("{static_dir}/index.html"))),
        );

    public
        .merge(protected)
        .merge(admin)
        .layer(cors)
        .with_state(state)
        .nest_service("/assets", assets_service)
        .fallback_service(spa_service)
}
