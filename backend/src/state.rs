use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::mcp_host::McpHost;
use crate::model_router::ModelRouter;

pub struct AppState {
    pub db: SqlitePool,
    pub model_router: ModelRouter,
    pub mcp_host: Arc<Mutex<McpHost>>,
}

impl AppState {
    pub fn new(db: SqlitePool) -> Self {
        Self {
            db,
            model_router: ModelRouter::new(),
            mcp_host: Arc::new(Mutex::new(McpHost::new())),
        }
    }
}
