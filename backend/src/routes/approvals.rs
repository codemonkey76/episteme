use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;

use axum::Extension;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

pub async fn list_pending(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(session_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    db::sessions::get(&state.db, &user.id, &session_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    let actions = db::pending_actions::list_pending(&state.db, &session_id)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "pending_actions": actions })))
}

/// Optional approve body: the operator's edited tool args from the approval
/// card. Absent (no/empty body) means run the model's original draft verbatim.
#[derive(serde::Deserialize)]
pub struct ApproveReq {
    edited_args: Option<serde_json::Value>,
}

pub async fn approve(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(action_id): Path<String>,
    // Body extractor must come last. Optional so a plain approve (no edits)
    // can POST with no body at all.
    body: Option<Json<ApproveReq>>,
) -> AppResult<StatusCode> {
    let edited_args = body.and_then(|Json(b)| b.edited_args);
    decide(&state, &user.id, &action_id, true, edited_args).await
}

pub async fn reject(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(action_id): Path<String>,
) -> AppResult<StatusCode> {
    decide(&state, &user.id, &action_id, false, None).await
}

async fn decide(
    state: &Arc<AppState>,
    user_id: &str,
    action_id: &str,
    approved: bool,
    edited_args: Option<serde_json::Value>,
) -> AppResult<StatusCode> {
    // Only the owner of the session may decide its tool calls.
    if !db::pending_actions::owned_by(&state.db, action_id, user_id)
        .await
        .map_err(AppError::Internal)?
    {
        return Err(AppError::NotFound);
    }
    let action = db::pending_actions::get(&state.db, action_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    // Idempotence: a second decide on the same action (double-click, two
    // devices) must not re-execute the tool.
    if action.status != "pending" {
        return Ok(StatusCode::NO_CONTENT);
    }

    // The args to run with: the operator's edits if they tweaked the card,
    // else the model's stored draft. `None` overall means denied. Persist
    // edits before resolving so the row (and the parked path below) reflect
    // what actually ran.
    let effective_args: Option<serde_json::Value> = if approved {
        let args = match &edited_args {
            Some(edited) => {
                db::pending_actions::update_args(&state.db, action_id, &edited.to_string())
                    .await
                    .map_err(AppError::Internal)?;
                edited.clone()
            }
            None => serde_json::from_str(&action.tool_args).unwrap_or(serde_json::Value::Null),
        };
        Some(args)
    } else {
        None
    };

    db::pending_actions::resolve(&state.db, action_id, approved)
        .await
        .map_err(AppError::Internal)?;
    // Wake the paused agent turn, if it's still in flight (live chat).
    if let Some(tx) = state.pending_approvals.lock().await.remove(action_id) {
        let _ = tx.send(effective_args);
        return Ok(StatusCode::NO_CONTENT);
    }

    // No live waiter. A PARKED action (call_id set — from an unattended run)
    // executes here and now, then resumes its suspended job once the session
    // has no more pending decisions. Live-chat rows that went dead
    // (timeout/disconnect/restart) have no call_id and stay DB-only updates.
    let Some(call_id) = action.call_id.clone() else {
        return Ok(StatusCode::NO_CONTENT);
    };

    let result_str = if let Some(args) = effective_args {
        match crate::agent::execute_tool(state, user_id, &action.tool_name, args).await {
            Ok(v) => {
                state
                    .log("tools", "info", format!("{} (approved from queue)", action.tool_name))
                    .await;
                v.to_string()
            }
            Err(e) => {
                state
                    .log("tools", "error", format!("{} failed: {e}", action.tool_name))
                    .await;
                format!("error: {e}")
            }
        }
    } else {
        "user declined this tool call".to_string()
    };
    db::messages::insert(
        &state.db,
        &action.session_id,
        "tool",
        &serde_json::to_string(&result_str).unwrap_or_default(),
        None,
        Some(&call_id),
    )
    .await
    .map_err(AppError::Internal)?;

    // Resume the job only when every parked call in the session is decided —
    // the replay invariant (each tool_call has its result rows) holds then.
    let remaining = db::pending_actions::count_pending(&state.db, &action.session_id)
        .await
        .map_err(AppError::Internal)?;
    if remaining == 0 {
        if let Some(job) = db::jobs::suspended_for_session(&state.db, &action.session_id)
            .await
            .map_err(AppError::Internal)?
        {
            // Atomic gate: racing decisions both reach here; one wins.
            if db::jobs::try_resume(&state.db, &job.id).await.map_err(AppError::Internal)? {
                state.log("jobs", "info", format!("'{}' resuming", job.name)).await;
                // Re-fetch so the queued job carries its post-resume status.
                if let Some(fresh) =
                    db::jobs::get(&state.db, &job.id).await.map_err(AppError::Internal)?
                {
                    let _ = state.job_tx.send(fresh);
                }
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/approvals/pending — every pending action across the user's
/// sessions (the global approval queue in the Jobs window).
pub async fn list_all_pending(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<serde_json::Value>> {
    let actions = db::pending_actions::list_pending_for_user(&state.db, &user.id)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "pending_actions": actions })))
}
