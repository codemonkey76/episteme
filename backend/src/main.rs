use anyhow::Result;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod calendar;
mod categorizer;
mod db;
mod error;
mod integrations;
mod memory;
mod model_router;
mod tools;
mod mcp_host;
mod agent;
mod routes;
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

    let state = Arc::new(AppState::new(pool));
    categorizer::spawn_worker(state.clone());
    let app = routes::router(state);

    let addr = std::env::var("BIND").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}
