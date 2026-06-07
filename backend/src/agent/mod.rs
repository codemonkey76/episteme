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

/// How a turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    Completed,
    /// One or more gated tool calls were parked as pending approvals and the
    /// turn ended early (unattended runs only). The run resumes — via
    /// `jobs::run` — once every parked call is decided.
    Suspended { pending: usize },
}

/// Run one agent turn for the given session, streaming `AgentEvent`s through `tx`.
pub async fn run_turn(
    state: Arc<AppState>,
    user_id: String,
    session_id: String,
    provider: ProviderConfig,
    tx: mpsc::Sender<AgentEvent>,
    // Unattended runs (scheduled agents, background jobs) have nobody at the
    // keyboard: "ask"-gated tools are PARKED as pending approvals — never
    // auto-approved — and the turn suspends.
    unattended: bool,
) -> Result<TurnOutcome> {
    // Compaction state: once a session outgrows the context budget, messages
    // at or before the cursor are represented by the stored summary instead of
    // being replayed verbatim. The full transcript stays in `messages`.
    let (session_summary, summary_until) =
        db::sessions::compaction(&state.db, &session_id).await?;
    let raw_messages =
        db::messages::list_for_session_after(&state.db, &session_id, summary_until.as_deref())
            .await?;
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
            // Multimodal: only the text matters for memory/relevance — the
            // base64 image payload would drown it.
            other if other.get("type").and_then(Value::as_str) == Some("multimodal") => {
                other.get("text").and_then(Value::as_str).unwrap_or_default().to_string()
            }
            other => other.to_string(),
        })
        .unwrap_or_default();

    // Cheap context dieting: clip fat tool results outside the recent window
    // (model-facing only — the stored transcript keeps the full results).
    crate::compaction::truncate_old_tool_results(&mut history);

    // Compacted-away turns ride in as a single system summary at the front.
    if let Some(summary) = &session_summary {
        history.insert(0, crate::compaction::summary_message(summary));
    }
    // Prepend stored memories so the model has cross-session context —
    // relevance-selected against the latest user message once the store is big.
    crate::memory::inject(&mut history, &state, &user_id, &user_text).await;
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
    // Token counts summed across all model round-trips in this turn.
    let mut turn_usage = crate::model_router::TokenUsage::default();

    // Cap tool round-trips so a misbehaving model can't loop forever.
    let mut iterations = 0;

    loop {
        iterations += 1;
        if iterations > 6 {
            // All text so far was persisted per-iteration (alongside its tool
            // calls), so there's nothing left to save at the cap.
            tracing::warn!("agent turn hit iteration cap");
            tx.send(AgentEvent::Done).await?;
            db::usage::record(&state.db, &user_id, &provider, "chat", Some(turn_usage)).await;
            return Ok(TurnOutcome::Completed);
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
                if let Some(u) = chunk.usage {
                    turn_usage.prompt += u.prompt;
                    turn_usage.completion += u.completion;
                }
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
                // Persist this iteration's reply so it survives a page refresh
                // (earlier iterations' text was already persisted alongside
                // their tool calls). Content is JSON-encoded like other messages.
                if !iter_text.trim().is_empty() {
                    let _ = db::messages::insert(
                        &state.db,
                        &session_id,
                        "assistant",
                        &serde_json::to_string(&iter_text).unwrap_or_default(),
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
                // Likewise detached: summarize the session's older turns once
                // it outgrows the context budget, so the next turn replays a
                // compact history instead of the whole transcript.
                tokio::spawn(crate::compaction::maybe_compact(
                    Arc::clone(&state),
                    user_id.clone(),
                    session_id.clone(),
                    provider.clone(),
                ));
                db::usage::record(&state.db, &user_id, &provider, "chat", Some(turn_usage)).await;
                return Ok(TurnOutcome::Completed);
            }
            Some(calls) => {
                // Keep any text the model emitted alongside the calls — and
                // persist it, so a suspended turn replays it on resume (the
                // in-memory copy alone would be lost with the process).
                if !iter_text.trim().is_empty() {
                    let _ = db::messages::insert(
                        &state.db,
                        &session_id,
                        "assistant",
                        &serde_json::to_string(&iter_text).unwrap_or_default(),
                        None,
                        None,
                    )
                    .await;
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

                // Gated calls parked this batch (unattended runs only).
                let mut parked = 0usize;

                for call in calls {
                    // Per-tool policy: tools marked "ask" in Settings → Tools
                    // (or ask-by-default, e.g. helpdesk writes) pause the turn
                    // here until the user approves or denies.
                    let policy = policies
                        .get(&call.fn_name)
                        .map(String::as_str)
                        .unwrap_or_else(|| crate::tools::default_policy(&call.fn_name));
                    if policy == "ask" {
                        if unattended {
                            // Park: persist the call as a pending approval (with
                            // its call_id, so the decision handler can write the
                            // tool-result row later) and move on. The turn
                            // suspends after the batch; deciding the last parked
                            // action resumes the job.
                            let action = db::pending_actions::insert_parked(
                                &state.db,
                                &session_id,
                                &call.fn_name,
                                &call.fn_arguments.to_string(),
                                &call.call_id,
                            )
                            .await?;
                            state
                                .log(
                                    "agent",
                                    "info",
                                    format!("parked for approval: {} ({})", call.fn_name, action.id),
                                )
                                .await;
                            parked += 1;
                            continue;
                        }
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

                    let result = execute_tool(&state, &user_id, &call.fn_name, call.fn_arguments.clone())
                        .await;

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

                // One or more calls await approval: suspend. The replay
                // invariant holds — auto calls wrote their result rows above;
                // parked calls get theirs when decided, and the job only
                // resumes once no pending rows remain for this session.
                if parked > 0 {
                    tx.send(AgentEvent::Done).await?;
                    db::usage::record(&state.db, &user_id, &provider, "chat", Some(turn_usage))
                        .await;
                    return Ok(TurnOutcome::Suspended { pending: parked });
                }

                // Loop again with the tool results appended.
                continue;
            }
        }
    }
}

/// Execute one tool call — native registry or MCP — returning the raw result.
/// Shared by the agent loop and the approval-resume path, so a parked call
/// executes exactly the way a live one would have.
pub async fn execute_tool(
    state: &Arc<AppState>,
    user_id: &str,
    name: &str,
    args: Value,
) -> Result<Value> {
    if crate::tools::is_native(name) {
        return crate::tools::execute(state, user_id, name, args).await;
    }
    // Resolve the peer under the lock, but run the (possibly slow) tool call
    // without holding it.
    let peer = {
        let mcp = state.mcp_host.lock().await;
        mcp.peer_for(name)
    };
    match peer {
        Ok((peer, tool)) => crate::mcp_host::call_on_peer(&peer, &tool, args).await,
        Err(e) => Err(e),
    }
}
