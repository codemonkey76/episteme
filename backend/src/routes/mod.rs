use std::sync::Arc;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

use crate::state::AppState;

mod approvals;
mod chat;
mod email;
mod integrations;
mod logs;
mod sessions;
mod settings;

pub fn router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let static_dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string());

    Router::new()
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
        .route("/api/email/messages/:id/read", axum::routing::patch(email::mark_read))
        .route("/api/email/send", post(email::send_email))
        .route("/api/email/ai-draft", post(email::ai_draft))
        .layer(cors)
        .with_state(state)
        .fallback_service(
            ServeDir::new(&static_dir)
                .not_found_service(ServeFile::new(format!("{static_dir}/index.html"))),
        )
}
