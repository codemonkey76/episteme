//! Pause-for-approval: when a tool's policy is "ask", the agent turn parks
//! here on a oneshot channel until the approve/reject endpoint resolves it
//! (or the client disconnects / the wait times out — both treated as denial).

use anyhow::Result;
use genai::chat::ToolCall;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use super::AgentEvent;
use crate::db;
use crate::state::AppState;

/// How long an approval can sit unanswered before it's auto-denied.
const DECISION_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Create a pending action, notify the UI, and block until decided.
/// Returns whether the tool may run.
pub async fn await_decision(
    state: &Arc<AppState>,
    session_id: &str,
    call: &ToolCall,
    tx: &mpsc::Sender<AgentEvent>,
) -> Result<bool> {
    let action = db::pending_actions::insert(
        &state.db,
        session_id,
        &call.fn_name,
        &call.fn_arguments.to_string(),
    )
    .await?;

    let (decide_tx, decide_rx) = tokio::sync::oneshot::channel::<bool>();
    state.pending_approvals.lock().await.insert(action.id.clone(), decide_tx);

    state
        .log("approvals", "info", format!("awaiting approval: {} {}", call.fn_name, call.fn_arguments))
        .await;
    tx.send(AgentEvent::AwaitingApproval {
        action_id: action.id.clone(),
        tool_name: call.fn_name.clone(),
        tool_args: call.fn_arguments.clone(),
    })
    .await?;

    // The approve/reject endpoints resolve the DB row themselves; we only do
    // so for the paths where no endpoint fired (disconnect / timeout).
    let (approved, resolve_db) = tokio::select! {
        decision = decide_rx => (decision.unwrap_or(false), false),
        _ = tx.closed() => (false, true),
        _ = tokio::time::sleep(DECISION_TIMEOUT) => (false, true),
    };

    state.pending_approvals.lock().await.remove(&action.id);
    if resolve_db {
        let _ = db::pending_actions::resolve(&state.db, &action.id, false).await;
    }
    state
        .log(
            "approvals",
            if approved { "info" } else { "warn" },
            format!(
                "{} {}",
                call.fn_name,
                if approved { "approved" } else { "denied" }
            ),
        )
        .await;
    Ok(approved)
}
