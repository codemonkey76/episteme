//! Pause-for-approval: when a tool's policy is "ask", the agent turn parks
//! here on a oneshot channel until the approve/reject endpoint resolves it
//! (or the client disconnects / the wait times out — both treated as denial).

use anyhow::Result;
use genai::chat::ToolCall;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use super::AgentEvent;
use crate::db;
use crate::state::AppState;

/// How long an approval can sit unanswered before it's auto-denied.
const DECISION_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Create a pending action, notify the UI, and block until decided.
/// Returns `None` if denied, or `Some(args)` to run with — the operator may
/// have edited the args in the approval card, so these can differ from the
/// model's original `call.fn_arguments`.
pub async fn await_decision(
    state: &Arc<AppState>,
    session_id: &str,
    call: &ToolCall,
    tx: &mpsc::Sender<AgentEvent>,
) -> Result<Option<Value>> {
    let action = db::pending_actions::insert(
        &state.db,
        session_id,
        &call.fn_name,
        &call.fn_arguments.to_string(),
    )
    .await?;

    let (decide_tx, decide_rx) = tokio::sync::oneshot::channel::<Option<Value>>();
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
    let (decision, resolve_db): (Option<Value>, bool) = tokio::select! {
        decision = decide_rx => (decision.unwrap_or(None), false),
        _ = tx.closed() => (None, true),
        _ = tokio::time::sleep(DECISION_TIMEOUT) => (None, true),
    };

    state.pending_approvals.lock().await.remove(&action.id);
    if resolve_db {
        let _ = db::pending_actions::resolve(&state.db, &action.id, false).await;
    }
    state
        .log(
            "approvals",
            if decision.is_some() { "info" } else { "warn" },
            format!(
                "{} {}",
                call.fn_name,
                if decision.is_some() { "approved" } else { "denied" }
            ),
        )
        .await;
    Ok(decision)
}
