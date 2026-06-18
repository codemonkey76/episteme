//! Tracked agent runs: chat-triggered background tasks and scheduled-agent
//! fires. A job wraps one (resumable) unattended `agent::run_turn` over its
//! own session. Gated tools park as pending approvals → the job suspends
//! (`needs_approval`) and pushes a notification; deciding the last parked
//! action flips it back to running (`db::jobs::try_resume`) and re-runs the
//! turn, which replays the session transcript including the decided results.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use tokio::sync::mpsc;

use crate::agent::{self, AgentEvent, TurnOutcome};
use crate::db::{self, jobs::Job};
use crate::integrations::push;
use crate::model_router::ProviderConfig;
use crate::state::AppState;

/// Drain the job queue: every enqueued job runs as its own task. Spawned once
/// from main; the queue (rather than direct spawns) is what lets the agent's
/// own tools enqueue jobs without creating an async-recursion cycle.
pub fn spawn_worker(state: Arc<AppState>, mut rx: mpsc::UnboundedReceiver<Job>) {
    tokio::spawn(async move {
        while let Some(job) = rx.recv().await {
            tokio::spawn(run(Arc::clone(&state), job));
        }
    });
}

/// Recover jobs left `running` by a prior process (their worker task died with
/// it). Research runs are self-contained — topic and depth live on the job and
/// session — so they're re-enqueued to run again rather than lost. Background
/// and scheduled runs may have already fired tools with side effects, so
/// blindly re-running them is unsafe: those are failed. Call once at startup,
/// after the queue worker is spawned.
pub async fn recover_orphans(state: &Arc<AppState>) {
    let orphans = match db::jobs::list_orphaned_running(&state.db).await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("orphan-job recovery query failed: {e}");
            return;
        }
    };
    let (mut restarted, mut failed) = (0u32, 0u32);
    for job in orphans {
        if job.kind == "research" {
            // Already `running`; just hand it back to the worker. A fresh
            // pass rebuilds the memo and writes a new report.
            if state.job_tx.send(job).is_ok() {
                restarted += 1;
            }
        } else {
            let _ = db::jobs::fail_orphaned(&state.db, &job.id).await;
            failed += 1;
        }
    }
    if restarted > 0 || failed > 0 {
        state
            .log(
                "jobs",
                "info",
                format!("startup recovery: resumed {restarted} research run(s), failed {failed} other orphan(s)"),
            )
            .await;
    }
}

/// Create the job row (status `running`) for an existing session.
pub async fn start(
    state: &AppState,
    user_id: &str,
    session_id: &str,
    provider_name: &str,
    kind: &str,
    name: &str,
    meta: Option<&str>,
) -> Result<Job> {
    db::jobs::insert(&state.db, user_id, session_id, kind, name, provider_name, meta).await
}

/// Run (or continue) a job to its next stopping point and record the outcome:
/// done + summary, needs_approval + push, or failed + error. Never panics the
/// caller — intended for `tokio::spawn`.
pub async fn run(state: Arc<AppState>, job: Job) {
    let result = run_inner(&state, &job).await;
    match result {
        Ok(TurnOutcome::Completed) => {
            let summary = session_summary(&state, &job.session_id).await;
            let _ = db::jobs::set_status(&state.db, &job.id, "done", Some(&summary), None).await;
            state
                .log("jobs", "info", format!("'{}' finished: {}", job.name, clip(&summary, 120)))
                .await;
            // A "quiet" job (e.g. a scheduled monitor) suppresses this automatic
            // completion notification — it only reaches the user if the agent
            // chose to call notify_user during the run.
            if !is_quiet(&job) {
                push::notify_linked(
                    &state,
                    &job.user_id,
                    &job.name,
                    &summary,
                    "agent",
                    Some(push::Link { kind: "session", id: &job.session_id }),
                )
                .await;
            }
        }
        Ok(TurnOutcome::Suspended { pending }) => {
            let tool = db::pending_actions::list_pending(&state.db, &job.session_id)
                .await
                .ok()
                .and_then(|rows| rows.first().map(|r| r.tool_name.clone()))
                .unwrap_or_else(|| "a tool".to_string());
            let _ = db::jobs::set_status(&state.db, &job.id, "needs_approval", None, None).await;
            state
                .log(
                    "jobs",
                    "info",
                    format!("'{}' suspended: {pending} action(s) await approval", job.name),
                )
                .await;
            push::notify_linked(
                &state,
                &job.user_id,
                &format!("{} needs approval", job.name),
                &format!("Waiting on: {tool} — open episteme to approve or deny"),
                "approval",
                Some(push::Link { kind: "session", id: &job.session_id }),
            )
            .await;
        }
        Err(e) => {
            let _ =
                db::jobs::set_status(&state.db, &job.id, "failed", None, Some(&e.to_string())).await;
            state.log("jobs", "error", format!("'{}' failed: {e}", job.name)).await;
            push::notify_linked(
                &state,
                &job.user_id,
                &format!("{} failed", job.name),
                &e.to_string(),
                "error",
                Some(push::Link { kind: "session", id: &job.session_id }),
            )
            .await;
        }
    }
}

async fn run_inner(state: &Arc<AppState>, job: &Job) -> Result<TurnOutcome> {
    let providers: Vec<ProviderConfig> =
        db::settings::get(&state.db, "providers").await?.unwrap_or_default();
    let provider = if job.provider.is_empty() {
        providers.into_iter().next()
    } else {
        providers.into_iter().find(|p| p.name == job.provider)
    }
    .ok_or_else(|| anyhow!("no matching provider configured"))?;

    // Research jobs run their own orchestrator, not the agent loop, and never
    // park tools (all their inputs are read-only) — so they always complete.
    if job.kind == "research" {
        crate::research::run(state, job, provider).await?;
        return Ok(TurnOutcome::Completed);
    }

    // Drain the event stream; the transcript persists to the session anyway.
    let (tx, mut rx) = mpsc::channel::<AgentEvent>(64);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let outcome = agent::run_turn(
        Arc::clone(state),
        job.user_id.clone(),
        job.session_id.clone(),
        provider,
        tx,
        true, // unattended: gated tools park, never auto-approve
    )
    .await;
    let _ = drain.await;
    outcome
}

/// The last assistant message of a session — the job's outcome at a glance.
pub async fn session_summary(state: &AppState, session_id: &str) -> String {
    db::messages::list_for_session(&state.db, session_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| serde_json::from_str::<String>(&m.content).unwrap_or(m.content))
        .unwrap_or_else(|| "(no output)".to_string())
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Whether the job opted out of the automatic completion notification (its meta
/// carries `{"quiet": true}`, set for quiet scheduled agents).
fn is_quiet(job: &Job) -> bool {
    job.meta
        .as_deref()
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .and_then(|v| v["quiet"].as_bool())
        .unwrap_or(false)
}
