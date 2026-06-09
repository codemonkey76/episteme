use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

use crate::mcp_host::McpHost;

pub struct AppState {
    pub db: SqlitePool,
    pub mcp_host: Arc<Mutex<McpHost>>,
    /// In-flight OAuth CSRF states → the user who initiated each connect.
    pub oauth_state: Arc<Mutex<HashMap<String, String>>>,
    pub http_client: reqwest::Client,
    /// Broadcast channel for real-time log streaming to SSE clients.
    pub log_tx: broadcast::Sender<String>,
    /// In-flight tool-approval waits: pending_action id → resume channel.
    /// The agent turn blocks on the receiver; the approve/reject endpoints
    /// send the decision — `None` denies, `Some(args)` approves and carries
    /// the (possibly operator-edited) args to run. Entries die with the
    /// process (rows stay "pending").
    pub pending_approvals:
        Arc<Mutex<HashMap<String, oneshot::Sender<Option<serde_json::Value>>>>>,
    /// Live terminal sessions, keyed by the client's session id. Both the
    /// xterm WebSocket and the AI agent attach to the same shared PTY.
    pub terminal_sessions:
        Arc<Mutex<HashMap<String, Arc<crate::terminal::TermSession>>>>,
    /// In-flight terminal-agent command approvals: id → resume channel.
    /// `None` denies, `Some(cmd)` approves and carries the (possibly edited)
    /// command to run in the shared shell.
    pub pending_terminal_cmds: Arc<Mutex<HashMap<String, oneshot::Sender<Option<String>>>>>,
    /// Persistent conversation history for the Terminal AI, keyed by session id.
    /// The system message is excluded; it is prepended fresh on each turn so
    /// prompt changes take effect immediately. Each new user message is appended,
    /// giving the model full context across multiple `ask()` calls.
    pub terminal_agent_history:
        Arc<Mutex<HashMap<String, Vec<crate::model_router::ChatMessage>>>>,
    /// Hand-off queue for background jobs: senders enqueue, the worker spawned
    /// in main runs them. A queue (rather than spawning `jobs::run` directly)
    /// breaks the async-recursion cycle when the agent's own
    /// `start_background_task` tool launches a job.
    pub job_tx: mpsc::UnboundedSender<crate::db::jobs::Job>,
}

impl AppState {
    /// Returns the state plus the job-queue receiver; main hands the receiver
    /// to `jobs::spawn_worker`.
    pub fn new(db: SqlitePool) -> (Self, mpsc::UnboundedReceiver<crate::db::jobs::Job>) {
        let (log_tx, _) = broadcast::channel(2000);
        let (job_tx, job_rx) = mpsc::unbounded_channel();
        let state = Self {
            db,
            mcp_host: Arc::new(Mutex::new(McpHost::new())),
            oauth_state: Arc::new(Mutex::new(HashMap::new())),
            // Timeouts so an unreachable upstream (Graph, helpdesk) fails with
            // an error instead of hanging a request forever.
            http_client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("http client"),
            log_tx,
            pending_approvals: Arc::new(Mutex::new(HashMap::new())),
            terminal_sessions: Arc::new(Mutex::new(HashMap::new())),
            pending_terminal_cmds: Arc::new(Mutex::new(HashMap::new())),
            terminal_agent_history: Arc::new(Mutex::new(HashMap::new())),
            job_tx,
        };
        (state, job_rx)
    }

    /// The user's home timezone — every model-facing time is resolved into
    /// this so the model never does timezone arithmetic. Falls back to the
    /// `TZ` env var, then UTC, when the setting isn't configured.
    pub async fn home_tz(&self, user_id: &str) -> chrono_tz::Tz {
        let stored: Option<String> =
            crate::db::settings::get(&self.db, &format!("timezone:{user_id}"))
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
