use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::db;
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig};
use crate::state::AppState;

pub mod approval;

#[derive(Debug)]
pub enum AgentEvent {
    Token(String),
    /// A native tool is about to run — surfaced in the chat UI.
    ToolCall { name: String },
    Done,
    AwaitingApproval { action_id: String, tool_name: String, tool_args: Value },
}

/// Run one agent turn for the given session, streaming `AgentEvent`s through `tx`.
pub async fn run_turn(
    state: Arc<AppState>,
    session_id: String,
    provider: ProviderConfig,
    tx: mpsc::Sender<AgentEvent>,
) -> Result<()> {
    let raw_messages = db::messages::list_for_session(&state.db, &session_id).await?;
    let mut history: Vec<ChatMessage> = raw_messages
        .into_iter()
        .map(|m| ChatMessage {
            role: m.role,
            content: serde_json::from_str(&m.content).unwrap_or(Value::String(m.content)),
        })
        .collect();

    // Latest user message — captured for post-turn memory extraction.
    let user_text = history
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| match &m.content {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();

    // Prepend stored memories so the model has cross-session context.
    crate::memory::inject(&mut history, &state.db).await;
    // Then the tool/date preamble at the very front.
    history.insert(0, crate::tools::system_preamble());

    // Accumulates the model's visible reply across the turn for extraction.
    let mut assistant_text = String::new();

    // Cap tool round-trips so a misbehaving model can't loop forever.
    let mut iterations = 0;

    loop {
        iterations += 1;
        if iterations > 6 {
            tracing::warn!("agent turn hit iteration cap");
            tx.send(AgentEvent::Done).await?;
            if !assistant_text.trim().is_empty() {
                let _ = db::messages::insert(
                    &state.db,
                    &session_id,
                    "assistant",
                    &serde_json::to_string(&assistant_text).unwrap_or_default(),
                    None,
                    None,
                )
                .await;
            }
            return Ok(());
        }

        let tools = {
            let mcp = state.mcp_host.lock().await;
            mcp.list_tools().await?
        };

        let mut tool_schemas: Vec<Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();
        // Native (built-in) tools — calendar management, etc.
        tool_schemas.extend(crate::tools::schemas());

        let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel(64);

        {
            let provider2 = provider.clone();
            let hist = history.clone();
            let schemas = tool_schemas.clone();
            tokio::spawn(async move {
                // think=false: don't force reasoning models (e.g. qwen3) into a
                // long internal think trace before answering — it isn't streamed
                // back, so chat just appears to hang. Plain replies stream immediately.
                if let Err(e) = ModelRouter::stream(&provider2, hist, schemas, false, chunk_tx).await {
                    tracing::error!("model_router error: {e}");
                }
            });
        }

        let mut tool_calls_from_model = None;

        while let Some(chunk) = chunk_rx.recv().await {
            if chunk.done {
                tool_calls_from_model = chunk.tool_calls;
            } else {
                assistant_text.push_str(&chunk.text);
                tx.send(AgentEvent::Token(chunk.text)).await?;
            }
        }

        match tool_calls_from_model {
            None => {
                // Model returned a final text answer.
                tx.send(AgentEvent::Done).await?;
                // Persist the reply so it survives a page refresh (history is
                // rebuilt from the DB). Content is JSON-encoded like other messages.
                if !assistant_text.trim().is_empty() {
                    let _ = db::messages::insert(
                        &state.db,
                        &session_id,
                        "assistant",
                        &serde_json::to_string(&assistant_text).unwrap_or_default(),
                        None,
                        None,
                    )
                    .await;
                }
                // Best-effort, detached: learn durable memories from this exchange.
                // Never blocks or fails the chat turn.
                let st = Arc::clone(&state);
                let prov = provider.clone();
                let sess = session_id.clone();
                tokio::spawn(async move {
                    crate::memory::extract(&st, prov, user_text, assistant_text, Some(sess)).await;
                });
                return Ok(());
            }
            Some(calls) => {
                for call in calls {
                    // Native tools run inline (no approval), and their result is
                    // appended to the in-memory history so the model can use it on
                    // the next iteration to compose its final answer.
                    if crate::tools::is_native(&call.fn_name) {
                        // Tell the UI a tool is running.
                        let _ = tx.send(AgentEvent::ToolCall { name: call.fn_name.clone() }).await;
                        let result =
                            crate::tools::execute(&state, &call.fn_name, call.fn_arguments.clone()).await;
                        let result_str = match result {
                            Ok(v) => v.to_string(),
                            Err(e) => format!("error: {e}"),
                        };
                        db::messages::insert(
                            &state.db,
                            &session_id,
                            "tool",
                            &serde_json::to_string(&result_str).unwrap_or_default(),
                            None,
                            Some(&call.call_id),
                        )
                        .await?;
                        history.push(ChatMessage {
                            role: "user".to_string(),
                            content: Value::String(format!(
                                "[tool result] {}: {}",
                                call.fn_name, result_str
                            )),
                        });
                        continue;
                    }

                    let tool_def = tools.iter().find(|t| t.name == call.fn_name);
                    let needs_approval =
                        tool_def.map(|t| t.requires_approval).unwrap_or(true);

                    if needs_approval {
                        let action = db::pending_actions::insert(
                            &state.db,
                            &session_id,
                            &call.fn_name,
                            &call.fn_arguments.to_string(),
                        )
                        .await?;

                        tx.send(AgentEvent::AwaitingApproval {
                            action_id: action.id,
                            tool_name: call.fn_name,
                            tool_args: call.fn_arguments,
                        })
                        .await?;

                        // Pause the turn; it resumes when the approval endpoint is hit.
                        return Ok(());
                    }

                    let result = {
                        let mcp = state.mcp_host.lock().await;
                        mcp.execute(&call.fn_name, call.fn_arguments.clone()).await
                    };

                    let result_str = match result {
                        Ok(v) => v.to_string(),
                        Err(e) => format!("error: {e}"),
                    };

                    db::messages::insert(
                        &state.db,
                        &session_id,
                        "tool",
                        &serde_json::to_string(&result_str).unwrap_or_default(),
                        None,
                        Some(&call.call_id),
                    )
                    .await?;
                }
                // Loop again with the tool results appended.
                continue;
            }
        }
    }
}
