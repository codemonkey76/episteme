use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
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

// --- Ollama model discovery ---

#[derive(Deserialize)]
pub struct OllamaModelsQuery {
    base_url: String,
}

pub async fn list_ollama_models(
    State(state): State<Arc<AppState>>,
    Query(params): Query<OllamaModelsQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let url = format!("{}/api/tags", params.base_url.trim_end_matches('/'));
    let response = state
        .http_client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    if !response.status().is_success() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "Ollama returned {}",
            response.status()
        )));
    }

    let body: serde_json::Value = response.json().await.map_err(|e| AppError::Internal(e.into()))?;
    let names: Vec<&str> = body["models"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|m| m["name"].as_str()).collect())
        .unwrap_or_default();

    Ok(Json(serde_json::json!({ "models": names })))
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
) -> AppResult<Json<serde_json::Value>> {
    let mut servers: Vec<McpServerConfig> = db::settings::get(&state.db, KEY_MCP_SERVERS)
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    if let Some(existing) = servers.iter_mut().find(|s| s.name == server.name) {
        *existing = server.clone();
    } else {
        servers.push(server.clone());
    }
    db::settings::set(&state.db, KEY_MCP_SERVERS, &servers)
        .await
        .map_err(AppError::Internal)?;

    // Connect (or re-connect) right away so the caller gets live status back.
    let status = {
        let mut mcp = state.mcp_host.lock().await;
        mcp.reconnect(server).await
    };
    if status.connected {
        state
            .log("mcp", "info", format!("connected to '{}' ({} tools)", status.name, status.tool_count))
            .await;
    } else {
        let err = status.error.clone().unwrap_or_else(|| "unknown error".into());
        state
            .log("mcp", "error", format!("failed to connect to '{}': {err}", status.name))
            .await;
    }
    Ok(Json(serde_json::json!({ "status": status })))
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

    {
        let mut mcp = state.mcp_host.lock().await;
        mcp.disconnect(&name).await;
    }
    state.log("mcp", "info", format!("disconnected '{name}'")).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn mcp_server_status(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let statuses = {
        let mcp = state.mcp_host.lock().await;
        mcp.status()
    };
    Ok(Json(serde_json::json!({ "statuses": statuses })))
}

// --- Tool approval policies ---

const KEY_TOOL_POLICIES: &str = "tool_policies";

/// All tools (native + MCP) with their approval policy, for the Tools page.
pub async fn list_tools(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let policies: std::collections::HashMap<String, String> =
        db::settings::get(&state.db, KEY_TOOL_POLICIES)
            .await
            .map_err(AppError::Internal)?
            .unwrap_or_default();
    let policy_of =
        |name: &str| policies.get(name).cloned().unwrap_or_else(|| "auto".to_string());

    let mut tools: Vec<serde_json::Value> = Vec::new();
    for (group, schemas) in crate::tools::catalog() {
        for s in schemas {
            let name = s["name"].as_str().unwrap_or_default();
            tools.push(serde_json::json!({
                "name": name,
                "description": s["description"].as_str().unwrap_or_default(),
                "group": group,
                "source": "native",
                "policy": policy_of(name),
                "suggest_ask": false,
            }));
        }
    }
    {
        let mcp = state.mcp_host.lock().await;
        for t in mcp.list_tools().await.map_err(AppError::Internal)? {
            tools.push(serde_json::json!({
                "name": t.name,
                "description": t.description,
                "group": t.server,
                "source": "mcp",
                "policy": policy_of(&t.name),
                // Non-read-only MCP tools are worth gating.
                "suggest_ask": t.requires_approval,
            }));
        }
    }
    Ok(Json(serde_json::json!({ "tools": tools })))
}

#[derive(Deserialize)]
pub struct ToolPolicyBody {
    name: String,
    policy: String,
}

pub async fn set_tool_policy(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ToolPolicyBody>,
) -> AppResult<StatusCode> {
    if body.policy != "auto" && body.policy != "ask" {
        return Err(AppError::BadRequest(format!(
            "invalid policy '{}' — expected 'auto' or 'ask'",
            body.policy
        )));
    }
    let mut policies: std::collections::HashMap<String, String> =
        db::settings::get(&state.db, KEY_TOOL_POLICIES)
            .await
            .map_err(AppError::Internal)?
            .unwrap_or_default();
    if body.policy == "auto" {
        // auto is the default — keep the stored map minimal.
        policies.remove(&body.name);
    } else {
        policies.insert(body.name, body.policy);
    }
    db::settings::set(&state.db, KEY_TOOL_POLICIES, &policies)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Timezone ---

pub async fn get_timezone(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<serde_json::Value>> {
    let tz = state.home_tz().await;
    Ok(Json(serde_json::json!({ "timezone": tz.name() })))
}

#[derive(Deserialize)]
pub struct TimezoneBody {
    timezone: String,
}

pub async fn set_timezone(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TimezoneBody>,
) -> AppResult<StatusCode> {
    // Validate before storing — an unparseable zone would silently fall back to UTC.
    if body.timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(AppError::BadRequest(format!(
            "'{}' is not a valid IANA timezone",
            body.timezone
        )));
    }
    db::settings::set(&state.db, "timezone", &body.timezone)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
