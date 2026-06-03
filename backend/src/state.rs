use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

use crate::mcp_host::McpHost;
use crate::model_router::ModelRouter;

pub struct AppState {
    pub db: SqlitePool,
    pub model_router: ModelRouter,
    pub mcp_host: Arc<Mutex<McpHost>>,
    /// Temporary CSRF state token for in-flight OAuth flows.
    pub oauth_state: Arc<Mutex<Option<String>>>,
    pub http_client: reqwest::Client,
    /// Broadcast channel for real-time log streaming to SSE clients.
    pub log_tx: broadcast::Sender<String>,
}

impl AppState {
    pub fn new(db: SqlitePool) -> Self {
        let (log_tx, _) = broadcast::channel(2000);
        Self {
            db,
            model_router: ModelRouter::new(),
            mcp_host: Arc::new(Mutex::new(McpHost::new())),
            oauth_state: Arc::new(Mutex::new(None)),
            http_client: reqwest::Client::new(),
            log_tx,
        }
    }

    /// The user's home timezone — every model-facing time is resolved into
    /// this so the model never does timezone arithmetic. Falls back to the
    /// `TZ` env var, then UTC, when the setting isn't configured.
    pub async fn home_tz(&self) -> chrono_tz::Tz {
        let stored: Option<String> = crate::db::settings::get(&self.db, "timezone")
            .await
            .ok()
            .flatten();
        stored
            .or_else(|| std::env::var("TZ").ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(chrono_tz::Tz::UTC)
    }

    /// Record a backend event in the logs table and broadcast it to live
    /// SSE subscribers (the Logs window). Best-effort — never fails the caller.
    pub async fn log(&self, category: &str, level: &str, message: impl Into<String>) {
        let entry = crate::db::logs::LogEntry {
            id: uuid::Uuid::new_v4().to_string(),
            ts: chrono::Utc::now().timestamp_millis(),
            category: category.to_string(),
            level: level.to_string(),
            message: message.into(),
        };
        if let Err(e) = crate::db::logs::insert(&self.db, &entry).await {
            tracing::warn!("failed to persist log entry: {e}");
        }
        let _ = self.log_tx.send(serde_json::to_string(&entry).unwrap_or_default());
    }
}
