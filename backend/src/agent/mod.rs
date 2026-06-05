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
    user_id: String,
    session_id: String,
    provider: ProviderConfig,
    tx: mpsc::Sender<AgentEvent>,
) -> Result<()> {
    let raw_messages = db::messages::list_for_session(&state.db, &session_id).await?;
    let mut history: Vec<ChatMessage> = raw_messages
        .into_iter()
        .map(|m| {
            let content = serde_json::from_str(&m.content).unwrap_or(Value::String(m.content));
            // Tool-result rows store the bare result; rewrap with the call id
            // so resumed sessions replay the same shape the live loop builds.
            if m.role == "tool" {
                return ChatMessage {
                    role: "tool".to_string(),
                    content: serde_json::json!({
                        "call_id": m.tool_call_id.unwrap_or_default(),
                        "content": content,
                    }),
                };
            }
            ChatMessage { role: m.role, content }
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
    crate::memory::inject(&mut history, &state.db, &user_id).await;
    // Then the tool/date preamble at the very front.
    history.insert(0, crate::tools::system_preamble(&state, &user_id).await);

    // Per-tool approval policies (tool name → "ask"); absent = auto-execute.
    let policies: std::collections::HashMap<String, String> =
        db::settings::get(&state.db, "tool_policies")
            .await
            .unwrap_or_default()
            .unwrap_or_default();

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
        // Text emitted this iteration — replayed into history alongside any
        // tool calls so the next iteration has the model's full prior turn.
        let mut iter_text = String::new();

        while let Some(chunk) = chunk_rx.recv().await {
            if chunk.done {
                tool_calls_from_model = chunk.tool_calls;
            } else {
                assistant_text.push_str(&chunk.text);
                iter_text.push_str(&chunk.text);
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
                let uid = user_id.clone();
                tokio::spawn(async move {
                    crate::memory::extract(&st, &uid, prov, user_text, assistant_text, Some(sess))
                        .await;
                });
                return Ok(());
            }
            Some(calls) => {
                // Keep any text the model emitted alongside the calls.
                if !iter_text.trim().is_empty() {
                    history.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: Value::String(iter_text.clone()),
                    });
                }
                // Record the assistant's tool calls in history (and DB) so the
                // next iteration sees that the model already acted — without
                // this, models re-issue the same call and duplicate the action.
                let calls_value = serde_json::to_value(&calls).unwrap_or_default();
                db::messages::insert(
                    &state.db,
                    &session_id,
                    "tool_call",
                    &serde_json::to_string(&calls_value).unwrap_or_default(),
                    None,
                    None,
                )
                .await?;
                history.push(ChatMessage { role: "tool_call".to_string(), content: calls_value });

                for call in calls {
                    // Per-tool policy: tools marked "ask" in Settings → Tools
                    // (or ask-by-default, e.g. helpdesk writes) pause the turn
                    // here until the user approves or denies.
                    let policy = policies
                        .get(&call.fn_name)
                        .map(String::as_str)
                        .unwrap_or_else(|| crate::tools::default_policy(&call.fn_name));
                    if policy == "ask" {
                        let approved =
                            approval::await_decision(&state, &session_id, &call, &tx).await?;
                        if !approved {
                            let declined = "user declined this tool call";
                            db::messages::insert(
                                &state.db,
                                &session_id,
                                "tool",
                                &serde_json::to_string(declined).unwrap_or_default(),
                                None,
                                Some(&call.call_id),
                            )
                            .await?;
                            history.push(ChatMessage {
                                role: "tool".to_string(),
                                content: serde_json::json!({
                                    "call_id": call.call_id,
                                    "name": call.fn_name,
                                    "content": declined,
                                }),
                            });
                            continue;
                        }
                    }

                    // Tell the UI a tool is running.
                    let _ = tx.send(AgentEvent::ToolCall { name: call.fn_name.clone() }).await;

                    let result = if crate::tools::is_native(&call.fn_name) {
                        crate::tools::execute(&state, &user_id, &call.fn_name, call.fn_arguments.clone())
                            .await
                    } else {
                        // Resolve the peer under the lock, but run the (possibly
                        // slow) tool call without holding it.
                        let peer = {
                            let mcp = state.mcp_host.lock().await;
                            mcp.peer_for(&call.fn_name)
                        };
                        match peer {
                            Ok((peer, tool)) => {
                                crate::mcp_host::call_on_peer(&peer, &tool, call.fn_arguments.clone())
                                    .await
                            }
                            Err(e) => Err(e),
                        }
                    };

                    let result_str = match result {
                        Ok(v) => {
                            state
                                .log("tools", "info", format!("{} {}", call.fn_name, call.fn_arguments))
                                .await;
                            v.to_string()
                        }
                        Err(e) => {
                            state
                                .log("tools", "error", format!("{} failed: {e}", call.fn_name))
                                .await;
                            format!("error: {e}")
                        }
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
                    // Feed the result back as a proper tool message so the model
                    // can use it on the next iteration.
                    history.push(ChatMessage {
                        role: "tool".to_string(),
                        content: serde_json::json!({
                            "call_id": call.call_id,
                            "name": call.fn_name,
                            "content": result_str,
                        }),
                    });
                }
                // Loop again with the tool results appended.
                continue;
            }
        }
    }
}
