use anyhow::Result;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod calendar;
mod categorizer;
mod compaction;
mod db;
mod documents;
mod error;
mod integrations;
mod jobs;
mod memory;
mod prompts;
mod research;
mod suggestions;
mod model_router;
mod tools;
mod mcp_host;
mod agent;
mod routes;
mod scheduler;
mod state;

use state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            "episteme=debug,tower_http=debug".into()
        }))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string());
    std::fs::create_dir_all(&data_dir)?;

    let db_url = format!("sqlite://{data_dir}/episteme.db?mode=rwc");
    let pool = db::init(&db_url).await?;

    let (state, job_rx) = AppState::new(pool);
    let state = Arc::new(state);
    integrations::microsoft::migrate_legacy(&state).await;
    integrations::microsoft::migrate_shared_to_per_user(&state).await;
    categorizer::spawn_worker(state.clone());
    scheduler::spawn_worker(state.clone());
    jobs::spawn_worker(state.clone(), job_rx);
    spawn_mcp_connect(state.clone());
    let app = routes::router(state);

    let addr = std::env::var("BIND").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

/// Connect to configured MCP servers in the background so a slow or hung
/// server can never stall app startup.
fn spawn_mcp_connect(state: Arc<AppState>) {
    tokio::spawn(async move {
        let configs: Vec<mcp_host::McpServerConfig> =
            match db::settings::get(&state.db, "mcp_servers").await {
                Ok(Some(servers)) => servers,
                Ok(None) => return,
                Err(e) => {
                    tracing::error!("failed to load mcp_servers setting: {e}");
                    return;
                }
            };
        if configs.is_empty() {
            return;
        }

        let statuses = {
            let mut mcp = state.mcp_host.lock().await;
            mcp.connect_all(configs).await
        };
        for s in statuses {
            if s.connected {
                state
                    .log("mcp", "info", format!("connected to '{}' ({} tools)", s.name, s.tool_count))
                    .await;
            } else {
                let err = s.error.unwrap_or_else(|| "unknown error".into());
                state
                    .log("mcp", "error", format!("failed to connect to '{}': {err}", s.name))
                    .await;
            }
        }
    });
}
