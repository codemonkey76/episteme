//! Interactive terminal endpoints: a WebSocket that bridges the browser's
//! xterm.js to a real PTY (bash/pwsh) in the container, plus persistent
//! command-history storage/search and an AI "suggest a command" helper.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::Arc;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;
use crate::terminal::{self, Shell};

#[derive(Deserialize)]
pub struct WsParams {
    shell: String,
    #[serde(default = "default_cols")]
    cols: u16,
    #[serde(default = "default_rows")]
    rows: u16,
}
fn default_cols() -> u16 {
    80
}
fn default_rows() -> u16 {
    24
}

/// GET /api/terminals/ws?shell=bash|pwsh — upgrade to a WebSocket and attach it
/// to a freshly-spawned shell. Auth is enforced by the `require_auth` layer on
/// the protected router (the session cookie rides the same-origin upgrade).
pub async fn ws(
    State(_state): State<Arc<AppState>>,
    Extension(CurrentUser(_user)): Extension<CurrentUser>,
    Query(params): Query<WsParams>,
    upgrade: WebSocketUpgrade,
) -> AppResult<impl IntoResponse> {
    let shell = Shell::parse(&params.shell)
        .ok_or_else(|| AppError::BadRequest(format!("unknown shell '{}'", params.shell)))?;
    let cols = params.cols.clamp(1, 1000);
    let rows = params.rows.clamp(1, 1000);
    Ok(upgrade.on_upgrade(move |socket| handle_socket(socket, shell, cols, rows)))
}

#[derive(Deserialize)]
struct Control {
    resize: Option<Resize>,
}
#[derive(Deserialize)]
struct Resize {
    cols: u16,
    rows: u16,
}

/// Pump bytes between the WebSocket and the PTY until either side closes.
/// Protocol: client→server Binary frames are raw stdin; Text frames are JSON
/// control (`{"resize":{cols,rows}}`). Server→client frames are raw stdout.
async fn handle_socket(socket: WebSocket, shell: Shell, cols: u16, rows: u16) {
    let pty = match terminal::spawn(shell, cols, rows) {
        Ok(p) => p,
        Err(e) => {
            let mut s = socket;
            let _ = s
                .send(Message::Text(format!("\r\nfailed to start shell: {e}\r\n")))
                .await;
            return;
        }
    };

    let reader = match pty.master.try_clone_reader() {
        Ok(r) => r,
        Err(_) => return,
    };
    let writer = match pty.master.take_writer() {
        Ok(w) => w,
        Err(_) => return,
    };
    let master = pty.master;
    let mut child = pty.child;

    // PTY stdout → channel (blocking read on a dedicated thread).
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 8192];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if out_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Channel → PTY stdin (blocking write on a dedicated thread).
    let (in_tx, in_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut writer = writer;
        while let Ok(data) = in_rx.recv() {
            if std::io::Write::write_all(&mut writer, &data).is_err() {
                break;
            }
            let _ = std::io::Write::flush(&mut writer);
        }
    });

    let (mut ws_tx, mut ws_rx) = socket.split();
    let mut send_task = tokio::spawn(async move {
        while let Some(chunk) = out_rx.recv().await {
            if ws_tx.send(Message::Binary(chunk)).await.is_err() {
                break;
            }
        }
        let _ = ws_tx.close().await;
    });

    loop {
        tokio::select! {
            incoming = ws_rx.next() => {
                match incoming {
                    Some(Ok(Message::Binary(b))) => {
                        if in_tx.send(b).is_err() { break; }
                    }
                    Some(Ok(Message::Text(t))) => {
                        if let Ok(ctrl) = serde_json::from_str::<Control>(&t) {
                            if let Some(r) = ctrl.resize {
                                let _ = master.resize(portable_pty::PtySize {
                                    rows: r.rows.clamp(1, 1000),
                                    cols: r.cols.clamp(1, 1000),
                                    pixel_width: 0,
                                    pixel_height: 0,
                                });
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
            // PTY closed (shell exited): stop.
            _ = &mut send_task => break,
        }
    }

    let _ = child.kill();
    send_task.abort();
}

// ── Persistent command history ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct HistoryQuery {
    shell: Option<String>,
    search: Option<String>,
}

pub async fn list_history(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(q): Query<HistoryQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let shell = q.shell.as_deref().filter(|s| !s.is_empty());
    let search = q.search.as_deref().filter(|s| !s.is_empty());
    let entries = db::terminal_history::list(&state.db, &user.id, shell, search, 100)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "history": entries })))
}

#[derive(Deserialize)]
pub struct RecordBody {
    shell: String,
    command: String,
}

pub async fn record_history(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<RecordBody>,
) -> AppResult<StatusCode> {
    db::terminal_history::insert(&state.db, &user.id, &body.shell, &body.command)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── AI command suggestion ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SuggestBody {
    shell: String,
    request: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    provider: Option<String>,
}

pub async fn suggest(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<SuggestBody>,
) -> AppResult<Json<serde_json::Value>> {
    if body.request.trim().is_empty() {
        return Err(AppError::BadRequest("request is required".into()));
    }
    let provider = resolve_provider(&state, body.provider.as_deref().unwrap_or("")).await?;

    let shell_name = match Shell::parse(&body.shell) {
        Some(Shell::Pwsh) => "PowerShell (pwsh on Linux)",
        _ => "bash on Linux (Debian)",
    };
    let system = format!(
        "You translate a request into a single shell command for {shell_name}. \
Output ONLY the command on one line — no explanation, no markdown, no code fences, \
no leading prompt characters. If the request is unclear, output the closest single \
safe command."
    );
    let mut user_msg = body.request.clone();
    if let Some(ctx) = body.context.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        let tail: String = ctx.chars().rev().take(2000).collect::<String>().chars().rev().collect();
        user_msg = format!("Recent terminal output (context):\n{tail}\n\nRequest: {}", body.request);
    }

    let history = vec![
        ChatMessage { role: "system".to_string(), content: serde_json::Value::String(system) },
        ChatMessage { role: "user".to_string(), content: serde_json::Value::String(user_msg) },
    ];
    let (raw, used) = ModelRouter::complete_with_usage(&provider, history)
        .await
        .map_err(AppError::Internal)?;
    db::usage::record(&state.db, &user.id, &provider, "terminal_suggest", used).await;

    Ok(Json(serde_json::json!({ "command": clean_command(&raw) })))
}

/// Resolve the named provider, or the first configured one when unset.
async fn resolve_provider(state: &AppState, name: &str) -> AppResult<ProviderConfig> {
    let providers: Vec<ProviderConfig> = db::settings::get(&state.db, "providers")
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    let chosen = if name.is_empty() {
        providers.into_iter().next()
    } else {
        providers.into_iter().find(|p| p.name == name)
    };
    chosen.ok_or_else(|| AppError::BadRequest("no AI provider configured".into()))
}

/// Strip code fences, surrounding whitespace, and a single leading prompt
/// char so we get a bare command line to paste.
fn clean_command(raw: &str) -> String {
    let mut s = raw.trim();
    if s.starts_with("```") {
        s = s.trim_start_matches("```");
        // Drop a language tag on the first line (e.g. ```bash).
        if let Some(nl) = s.find('\n') {
            let first = &s[..nl];
            if !first.contains(' ') {
                s = &s[nl + 1..];
            }
        }
        s = s.trim_end_matches("```").trim();
    }
    // Take the first non-empty line.
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    line.trim_start_matches(['$', '#', '>']).trim().to_string()
}
