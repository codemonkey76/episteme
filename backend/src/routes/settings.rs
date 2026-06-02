use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::mcp_host::McpServerConfig;
use crate::model_router::ProviderConfig;
use crate::state::AppState;

const KEY_PROVIDERS: &str = "providers";
const KEY_MCP_SERVERS: &str = "mcp_servers";

// --- Providers ---

pub async fn list_providers(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let providers: Vec<ProviderConfig> = db::settings::get(&state.db, KEY_PROVIDERS)
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    Ok(Json(serde_json::json!({ "providers": providers })))
}

pub async fn upsert_provider(
    State(state): State<Arc<AppState>>,
    Json(provider): Json<ProviderConfig>,
) -> AppResult<StatusCode> {
    let mut providers: Vec<ProviderConfig> = db::settings::get(&state.db, KEY_PROVIDERS)
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    if let Some(existing) = providers.iter_mut().find(|p| p.name == provider.name) {
        *existing = provider;
    } else {
        providers.push(provider);
    }
    db::settings::set(&state.db, KEY_PROVIDERS, &providers)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_provider(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> AppResult<StatusCode> {
    let mut providers: Vec<ProviderConfig> = db::settings::get(&state.db, KEY_PROVIDERS)
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    providers.retain(|p| p.name != name);
    db::settings::set(&state.db, KEY_PROVIDERS, &providers)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// --- MCP Servers ---

pub async fn list_mcp_servers(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let servers: Vec<McpServerConfig> = db::settings::get(&state.db, KEY_MCP_SERVERS)
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    Ok(Json(serde_json::json!({ "mcp_servers": servers })))
}

pub async fn upsert_mcp_server(
    State(state): State<Arc<AppState>>,
    Json(server): Json<McpServerConfig>,
) -> AppResult<StatusCode> {
    let mut servers: Vec<McpServerConfig> = db::settings::get(&state.db, KEY_MCP_SERVERS)
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    if let Some(existing) = servers.iter_mut().find(|s| s.name == server.name) {
        *existing = server;
    } else {
        servers.push(server);
    }
    db::settings::set(&state.db, KEY_MCP_SERVERS, &servers)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> AppResult<StatusCode> {
    let mut servers: Vec<McpServerConfig> = db::settings::get(&state.db, KEY_MCP_SERVERS)
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    servers.retain(|s| s.name != name);
    db::settings::set(&state.db, KEY_MCP_SERVERS, &servers)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
