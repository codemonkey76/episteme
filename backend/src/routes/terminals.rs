//! Interactive terminal endpoints. A WebSocket bridges the browser's xterm.js
//! to a shared [`TermSession`] PTY (bash/pwsh) in the container; the AI agent
//! drives that same session — proposing commands (each approved by the user),
//! running them in the visible shell, and reading their output to decide the
//! next step. Plus persistent, searchable command history.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Extension, Json,
};
use futures::{stream, SinkExt, StreamExt};
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

use crate::db;
use crate::error::{AppError, AppResult};
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;
use crate::terminal::{Shell, TermSession};

#[derive(Deserialize)]
pub struct WsParams {
    shell: String,
    session: String,
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

/// GET /api/terminals/ws?shell=&session=&cols=&rows= — upgrade to a WebSocket
/// and attach it to a shared shell session (created on first connect). Auth is
/// enforced by `require_auth`; the session cookie rides the same-origin upgrade.
pub async fn ws(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(params): Query<WsParams>,
    upgrade: WebSocketUpgrade,
) -> AppResult<impl IntoResponse> {
    let shell = Shell::parse(&params.shell)
        .ok_or_else(|| AppError::BadRequest(format!("unknown shell '{}'", params.shell)))?;
    let cols = params.cols.clamp(1, 1000);
    let rows = params.rows.clamp(1, 1000);
    let session_id = params.session;
    let user_id = user.id;
    Ok(upgrade
        .on_upgrade(move |socket| handle_socket(state, socket, user_id, session_id, shell, cols, rows)))
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

async fn handle_socket(
    state: Arc<AppState>,
    socket: WebSocket,
    user_id: String,
    session_id: String,
    shell: Shell,
    cols: u16,
    rows: u16,
) {
    // A fresh shell every connect — we never resurrect the old one (so a
    // refresh can't re-run an in-flight or destructive command). Continuity
    // comes from repainting saved scrollback below, not from a live process.
    let session = match TermSession::create(shell, cols, rows) {
        Ok(s) => s,
        Err(e) => {
            let mut s = socket;
            let _ = s.send(Message::Text(format!("\r\nfailed to start shell: {e}\r\n"))).await;
            return;
        }
    };
    state.terminal_sessions.lock().await.insert(session_id.clone(), session.clone());

    // Subscribe before anything streams so the new shell's first prompt isn't
    // missed between spawn and the send loop starting.
    let mut rx = session.subscribe();
    let capture_rx = session.subscribe();
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Repaint the saved scrollback (display only — never written to the shell),
    // then a "reconnected" banner, before the fresh shell's prompt arrives.
    let saved =
        db::terminal_output::restore_tail(&state.db, &user_id, &session_id).await.unwrap_or_default();
    if !saved.is_empty() {
        // Drop query/shell-integration sequences so replaying scrollback can't
        // make xterm talk back to the fresh shell (the `…R` prompt corruption).
        let _ = ws_tx.send(Message::Binary(crate::terminal::strip_replay_hazards(&saved))).await;
        let when = chrono::Utc::now()
            .with_timezone(&state.home_tz(&user_id).await)
            .format("%a %-d %b %Y, %-I:%M %p");
        let banner = format!("\r\n\x1b[90m── reconnected · {when} ──\x1b[0m\r\n");
        let _ = ws_tx.send(Message::Binary(banner.into_bytes())).await;
    }

    // Batch live output into the durable, searchable archive. A Notify lets us
    // flush and stop it cleanly when the socket closes.
    let cap_notify = Arc::new(tokio::sync::Notify::new());
    let capture_task = {
        let db = state.db.clone();
        let tid = session_id.clone();
        let uid = user_id.clone();
        let shell_name = shell.as_str();
        let notify = cap_notify.clone();
        tokio::spawn(capture_loop(db, tid, uid, shell_name, capture_rx, notify))
    };

    // PTY output → WebSocket.
    let mut send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(chunk) => {
                    if ws_tx.send(Message::Binary(chunk)).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
        let _ = ws_tx.close().await;
    });

    loop {
        tokio::select! {
            incoming = ws_rx.next() => {
                match incoming {
                    Some(Ok(Message::Binary(b))) => session.write(&b),
                    Some(Ok(Message::Text(t))) => {
                        if let Ok(ctrl) = serde_json::from_str::<Control>(&t) {
                            if let Some(r) = ctrl.resize {
                                session.resize(r.cols.clamp(1, 1000), r.rows.clamp(1, 1000));
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
            _ = &mut send_task => break,
        }
    }

    session.kill();
    // Only deregister if a newer connection hasn't already replaced us: stable
    // session ids mean a refresh registers a new session under the same key, and
    // this (older) socket's teardown fires just after — without this guard it
    // would delete the fresh session and break the AI sidebar ("400 not
    // connected").
    let still_ours = {
        let mut sessions = state.terminal_sessions.lock().await;
        match sessions.get(&session_id) {
            Some(s) if Arc::ptr_eq(s, &session) => {
                sessions.remove(&session_id);
                true
            }
            _ => false,
        }
    };
    if still_ours {
        state.terminal_agent_history.lock().await.remove(&session_id);
    }
    send_task.abort();
    // Stop the capture task and let it write out whatever it has buffered.
    cap_notify.notify_one();
    let _ = capture_task.await;
}

/// Batch PTY output into the durable archive: accumulate chunks and flush on a
/// size threshold or a timer, then a final flush when `notify` fires (socket
/// closing). Keeps one DB write per ~batch rather than per chunk.
async fn capture_loop(
    db: sqlx::SqlitePool,
    terminal_id: String,
    user_id: String,
    shell: &'static str,
    mut rx: tokio::sync::broadcast::Receiver<Vec<u8>>,
    notify: Arc<tokio::sync::Notify>,
) {
    const FLUSH_BYTES: usize = 16 * 1024;
    let mut buf: Vec<u8> = Vec::new();
    let mut tick = tokio::time::interval(Duration::from_millis(750));
    loop {
        tokio::select! {
            r = rx.recv() => match r {
                Ok(chunk) => {
                    buf.extend_from_slice(&chunk);
                    if buf.len() >= FLUSH_BYTES {
                        flush_output(&db, &terminal_id, &user_id, shell, &mut buf).await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            _ = tick.tick() => flush_output(&db, &terminal_id, &user_id, shell, &mut buf).await,
            _ = notify.notified() => break,
        }
    }
    // Drain whatever is still buffered in the broadcast, then a final write.
    while let Ok(chunk) = rx.try_recv() {
        buf.extend_from_slice(&chunk);
    }
    flush_output(&db, &terminal_id, &user_id, shell, &mut buf).await;
}

/// Persist one batch: raw bytes for replay plus an ANSI-stripped copy for search.
async fn flush_output(
    db: &sqlx::SqlitePool,
    terminal_id: &str,
    user_id: &str,
    shell: &str,
    buf: &mut Vec<u8>,
) {
    if buf.is_empty() {
        return;
    }
    let raw = std::mem::take(buf);
    let stripped = strip_ansi_escapes::strip(&raw);
    let text = String::from_utf8_lossy(&stripped).replace("\r\n", "\n");
    let _ = db::terminal_output::append(db, terminal_id, user_id, shell, &raw, &text).await;
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

#[derive(Deserialize)]
pub struct OutputSearchQuery {
    q: String,
}

/// GET /api/terminals/output/search?q= — full-text-ish search across the user's
/// whole terminal scrollback archive (all sessions, across restarts).
pub async fn search_output(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(q): Query<OutputSearchQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let hits = db::terminal_output::search(&state.db, &user.id, &q.q, 100)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "hits": hits })))
}

// ── AI agent (multi-step, drives the shared shell, approve-each) ─────────────

#[derive(Deserialize)]
pub struct AgentBody {
    session_id: String,
    message: String,
    #[serde(default)]
    provider: Option<String>,
}

/// Events streamed from the agent loop to the sidebar.
enum AgentEvent {
    Token(String),
    /// The agent typed `command` into the live terminal prompt; the user can
    /// edit it and press Enter to run, or Skip (referenced by `id`).
    Proposed { id: String, command: String },
    Output { command: String, exit: Option<i32> },
    Error(String),
    Done,
}

const MAX_STEPS: usize = 25;
/// One window covering the whole typed-command flow: the user's editing time,
/// any interactive sign-in (device-code auth blocks while they authenticate in
/// a browser), and the command's own runtime.
const TYPED_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub async fn agent(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<AgentBody>,
) -> AppResult<impl IntoResponse> {
    let session = state
        .terminal_sessions
        .lock()
        .await
        .get(&body.session_id)
        .cloned()
        .ok_or_else(|| AppError::BadRequest("terminal session not connected".into()))?;
    let provider = resolve_provider(&state, body.provider.as_deref().unwrap_or("")).await?;

    let (tx, rx) = mpsc::channel::<AgentEvent>(64);
    {
        let state = Arc::clone(&state);
        let user_id = user.id.clone();
        let session_id = body.session_id.clone();
        let message = body.message.clone();
        tokio::spawn(async move {
            run_agent(state, user_id, provider, session, session_id, message, tx).await;
        });
    }

    let event_stream = stream::unfold(rx, |mut rx| async move {
        let ev = rx.recv().await?;
        let data = match ev {
            AgentEvent::Token(text) => serde_json::json!({ "type": "token", "text": text }),
            AgentEvent::Proposed { id, command } => {
                serde_json::json!({ "type": "proposed", "id": id, "command": command })
            }
            AgentEvent::Output { command, exit } => {
                serde_json::json!({ "type": "output", "command": command, "exit": exit })
            }
            AgentEvent::Error(m) => serde_json::json!({ "type": "error", "message": m }),
            AgentEvent::Done => serde_json::json!({ "type": "done" }),
        };
        Some((Ok::<Event, Infallible>(Event::default().data(data.to_string())), rx))
    });

    Ok(Sse::new(event_stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(5))))
}

#[derive(Deserialize)]
pub struct DecideBody {
    id: String,
    approved: bool,
    #[serde(default)]
    command: Option<String>,
}

/// POST /api/terminals/agent/decide — skip the command the agent typed into the
/// prompt (the user runs it themselves by pressing Enter; this is the reject
/// path). Any decision firing the channel cancels the pending capture.
pub async fn agent_decide(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(_user)): Extension<CurrentUser>,
    Json(body): Json<DecideBody>,
) -> AppResult<StatusCode> {
    if let Some(tx) = state.pending_terminal_cmds.lock().await.remove(&body.id) {
        let decision = if body.approved { body.command.or(Some(String::new())) } else { None };
        let _ = tx.send(decision);
    }
    Ok(StatusCode::NO_CONTENT)
}

fn run_command_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "run_command",
        "description": "Run a single shell command in the user's terminal and receive its output and exit code. Use exactly one command per call. Prefer non-interactive commands (no pagers, editors, or programs that wait for input). Read the result before deciding the next command.",
        "input_schema": {
            "type": "object",
            "properties": { "command": { "type": "string", "description": "The shell command to run." } },
            "required": ["command"]
        }
    })
}

async fn run_agent(
    state: Arc<AppState>,
    user_id: String,
    provider: ProviderConfig,
    session: Arc<TermSession>,
    session_id: String,
    message: String,
    ev: mpsc::Sender<AgentEvent>,
) {
    let shell_name = match session.shell() {
        Shell::Pwsh => "PowerShell",
        Shell::Bash => "bash",
    };
    let system = crate::prompts::get(&state.db, "terminal_agent")
        .await
        .replace("{shell}", shell_name);

    // Load prior turns for this session (excludes the system message, which is
    // always rebuilt fresh so edits to the prompt take effect immediately).
    let prior: Vec<ChatMessage> = state
        .terminal_agent_history
        .lock()
        .await
        .get(&session_id)
        .cloned()
        .unwrap_or_default();

    let mut history: Vec<ChatMessage> = std::iter::once(ChatMessage {
        role: "system".to_string(),
        content: serde_json::Value::String(system),
    })
    .chain(prior)
    .chain(std::iter::once(ChatMessage {
        role: "user".to_string(),
        content: serde_json::Value::String(message),
    }))
    .collect();

    let tools = vec![run_command_schema()];

    for _ in 0..MAX_STEPS {
        let (text, calls) = match stream_turn(&state, &user_id, &provider, history.clone(), tools.clone(), &ev).await {
            Ok(v) => v,
            Err(e) => {
                let _ = ev.send(AgentEvent::Error(e.to_string())).await;
                let _ = ev.send(AgentEvent::Done).await;
                return;
            }
        };
        if !text.trim().is_empty() {
            history.push(ChatMessage { role: "assistant".to_string(), content: serde_json::Value::String(text) });
        }
        if calls.is_empty() {
            break; // final answer
        }
        history.push(ChatMessage {
            role: "tool_call".to_string(),
            content: serde_json::to_value(&calls).unwrap_or_default(),
        });

        for call in calls {
            if call.fn_name != "run_command" {
                history.push(tool_result(&call.call_id, "error: unknown tool"));
                continue;
            }
            let proposed = call.fn_arguments["command"].as_str().unwrap_or("").to_string();

            // Type the command into the live prompt for the user to edit (or
            // not) and run themselves, then capture whatever they actually run.
            // A Skip signal (via agent_decide) lets them reject it instead.
            let id = uuid::Uuid::new_v4().to_string();
            let (dtx, drx) = oneshot::channel::<Option<String>>();
            state.pending_terminal_cmds.lock().await.insert(id.clone(), dtx);
            let _ = ev.send(AgentEvent::Proposed { id: id.clone(), command: proposed.clone() }).await;

            tokio::select! {
                (output, exit) = session.type_and_capture(&proposed, TYPED_TIMEOUT) => {
                    let _ = ev.send(AgentEvent::Output { command: proposed.clone(), exit }).await;
                    let exit_str = exit.map(|e| e.to_string()).unwrap_or_else(|| "unknown (did not finish)".to_string());
                    history.push(tool_result(
                        &call.call_id,
                        &format!("exit code: {exit_str}\n\noutput:\n{output}"),
                    ));
                }
                _ = drx => {
                    // User skipped: clear the typed-but-unrun line.
                    session.cancel_line();
                    history.push(tool_result(&call.call_id, "user skipped this command"));
                }
            }
            state.pending_terminal_cmds.lock().await.remove(&id);
        }
    }

    // Persist all turns except the system message for the next call.
    let to_save: Vec<ChatMessage> = history.into_iter().skip(1).collect();
    state.terminal_agent_history.lock().await.insert(session_id, to_save);

    let _ = ev.send(AgentEvent::Done).await;
}

fn tool_result(call_id: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: "tool".to_string(),
        content: serde_json::json!({ "call_id": call_id, "name": "run_command", "content": content }),
    }
}

/// Run one model turn: stream text tokens to the sidebar, return the full text
/// and any tool calls the model requested.
async fn stream_turn(
    state: &AppState,
    user_id: &str,
    provider: &ProviderConfig,
    history: Vec<ChatMessage>,
    tools: Vec<serde_json::Value>,
    ev: &mpsc::Sender<AgentEvent>,
) -> anyhow::Result<(String, Vec<genai::chat::ToolCall>)> {
    let (tx, mut rx) = mpsc::channel(64);
    let p = provider.clone();
    // think=true: qwen3 reliably narrates its next step ("Let me reconnect…")
    // and stops without calling run_command when reasoning is off, leaving the
    // agent stalled. Reasoning fixes that at the cost of a brief pause before
    // the visible reply (the think trace isn't streamed).
    let handle = tokio::spawn(async move { ModelRouter::stream(&p, history, tools, true, tx).await });

    let mut text = String::new();
    let mut calls = Vec::new();
    while let Some(chunk) = rx.recv().await {
        if !chunk.text.is_empty() {
            text.push_str(&chunk.text);
            let _ = ev.send(AgentEvent::Token(chunk.text)).await;
        }
        if chunk.done {
            if let Some(c) = chunk.tool_calls {
                calls = c;
            }
            if let Some(usage) = chunk.usage {
                db::usage::record(&state.db, user_id, provider, "terminal_agent", Some(usage)).await;
            }
        }
    }
    handle.await??;
    Ok((text, calls))
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
