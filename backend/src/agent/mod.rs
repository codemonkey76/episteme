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
    let history: Vec<ChatMessage> = raw_messages
        .into_iter()
        .map(|m| ChatMessage {
            role: m.role,
            content: serde_json::from_str(&m.content).unwrap_or(Value::String(m.content)),
        })
        .collect();

    loop {
        let tools = {
            let mcp = state.mcp_host.lock().await;
            mcp.list_tools().await?
        };

        let tool_schemas: Vec<Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();

        let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel(64);

        {
            let provider2 = provider.clone();
            let hist = history.clone();
            let schemas = tool_schemas.clone();
            tokio::spawn(async move {
                if let Err(e) = ModelRouter::stream(&provider2, hist, schemas, true, chunk_tx).await {
                    tracing::error!("model_router error: {e}");
                }
            });
        }

        let mut tool_calls_from_model = None;

        while let Some(chunk) = chunk_rx.recv().await {
            if chunk.done {
                tool_calls_from_model = chunk.tool_calls;
            } else {
                // Persist assistant token to DB would go here in a production impl.
                tx.send(AgentEvent::Token(chunk.text)).await?;
            }
        }

        match tool_calls_from_model {
            None => {
                // Model returned a final text answer.
                tx.send(AgentEvent::Done).await?;
                return Ok(());
            }
            Some(calls) => {
                for call in calls {
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
