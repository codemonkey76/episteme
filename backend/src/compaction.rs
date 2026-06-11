//! Context compaction (Phase 10). Every turn replays the session's entire
//! history — fat tool results included — so long sessions degrade quietly.
//! Two layers fix that, both touching only the MODEL-FACING history; the full
//! transcript stays in `messages` for display and search:
//!
//! 1. Cheap per-turn dieting (no model call): tool results outside the recent
//!    window are clipped before the history is sent.
//! 2. Rolling summarization (one model call, purpose `compaction`): after a
//!    completed turn, once the live history outgrows the budget, everything
//!    but the recent window is summarized into `sessions.summary` and the
//!    `sessions.summary_until` cursor advances. The next turn loads only
//!    messages past the cursor and rides the summary in as a system message.
//!    Each compaction folds the previous summary in, so it rolls forever.

use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::db::{self, messages::Message};
use crate::model_router::{ChatMessage, ModelRouter, ProviderConfig};
use crate::state::AppState;

// These budgets are sized to the chat's context window — now ~128K tokens
// (the Ollama num_ctx cap; cloud models are larger). At ~3.5 chars/token that's
// ~450K chars, so the old 48K threshold summarized away ~90% of usable context:
// a session would forget tool results (e.g. a CSV it just read) long before the
// window was anywhere near full, and tasks that need two documents in context
// at once became impossible. The values below keep a generous slice of real
// history verbatim and only summarize once it genuinely approaches the window.

/// Messages at the tail of the history whose tool results stay verbatim.
const TRIM_KEEP_RECENT: usize = 12;
/// Tool results older than the recent window are clipped to this many chars.
/// Large enough to hold a full document/CSV read (~10–15K chars) so a
/// multi-file task keeps the source data in context.
const TOOL_RESULT_CLIP: usize = 16_000;
/// Model-facing size (chars, post-clip) past which a session is summarized.
/// ~220K chars ≈ 60K tokens of history — leaving ample room for tool schemas,
/// injected memory, and the reply within a 128K-token window.
const COMPACT_THRESHOLD: usize = 220_000;
/// Messages kept verbatim by a compaction (the boundary then snaps back to a
/// user message, so the kept window may grow a little).
const COMPACT_KEEP_RECENT: usize = 12;
/// Per-message cap in the transcript rendered for the summarizer.
const RENDER_MSG_CAP: usize = 2_000;
/// Total transcript cap for one summarizer call (newest part kept).
const RENDER_TOTAL_CAP: usize = 160_000;

/// Clip fat tool results outside the recent window — the cheap, every-turn
/// layer. Operates on the in-memory history `run_turn` built (tool messages
/// are `{call_id, name?, content}`); the stored transcript is untouched.
pub fn truncate_old_tool_results(history: &mut [ChatMessage]) {
    let keep_from = history.len().saturating_sub(TRIM_KEEP_RECENT);
    for m in &mut history[..keep_from] {
        if m.role != "tool" {
            continue;
        }
        if let Some(Value::String(s)) = m.content.get_mut("content") {
            if s.chars().count() > TOOL_RESULT_CLIP {
                let mut clipped: String = s.chars().take(TOOL_RESULT_CLIP).collect();
                clipped.push_str(" … [tool result truncated to save context]");
                *s = clipped;
            }
        }
    }
}

/// Compact the session if its model-facing history has outgrown the budget.
/// Best-effort and detached — spawned after a completed turn, like memory
/// extraction; a failure is logged and the session just stays uncompacted.
pub async fn maybe_compact(
    state: Arc<AppState>,
    user_id: String,
    session_id: String,
    provider: ProviderConfig,
) {
    if let Err(e) = compact(&state, &user_id, &session_id, &provider).await {
        tracing::warn!("compaction failed for session {session_id}: {e}");
    }
}

async fn compact(
    state: &Arc<AppState>,
    user_id: &str,
    session_id: &str,
    provider: &ProviderConfig,
) -> Result<()> {
    let (old_summary, until) = db::sessions::compaction(&state.db, session_id).await?;
    let msgs =
        db::messages::list_for_session_after(&state.db, session_id, until.as_deref()).await?;
    if live_size(&msgs) <= COMPACT_THRESHOLD {
        return Ok(());
    }
    let Some(split) = split_point(&msgs) else {
        return Ok(());
    };
    let old_part = &msgs[..split];

    let mut content = crate::prompts::get(&state.db, "session_compact").await;
    if let Some(prev) = &old_summary {
        content.push_str("\n\nSummary of the conversation before this transcript (fold it in):\n");
        content.push_str(prev);
    }
    content.push_str("\n\nTranscript to compact:\n");
    content.push_str(&render(old_part));

    let history = vec![ChatMessage { role: "user".to_string(), content: Value::String(content) }];
    let (summary, used) = ModelRouter::complete_with_usage(provider, history).await?;
    db::usage::record(&state.db, user_id, provider, "compaction", used).await;
    let summary = summary.trim();
    if summary.is_empty() {
        anyhow::bail!("summarizer returned empty output");
    }

    // The cursor is the created_at of the last message the summary covers;
    // future turns load strictly newer rows.
    let cut = &old_part[old_part.len() - 1].created_at;
    db::sessions::set_compaction(&state.db, session_id, summary, cut).await?;
    state
        .log(
            "agent",
            "info",
            format!(
                "compacted session {session_id}: {} message(s) → {}-char summary",
                old_part.len(),
                summary.chars().count()
            ),
        )
        .await;
    Ok(())
}

/// The system message carrying the stored summary into a turn's history.
pub fn summary_message(summary: &str) -> ChatMessage {
    ChatMessage {
        role: "system".to_string(),
        content: Value::String(format!(
            "Summary of the earlier part of this conversation (older messages were \
compacted away to save context):\n{summary}"
        )),
    }
}

/// The model-facing text of a stored message: the JSON-decoded string, the
/// text part of a multimodal message (base64 images would drown everything),
/// or the raw JSON otherwise.
fn text_of(m: &Message) -> String {
    let parsed: Value = match serde_json::from_str(&m.content) {
        Ok(v) => v,
        Err(_) => return m.content.clone(),
    };
    match parsed {
        Value::String(s) => s,
        other if other.get("type").and_then(Value::as_str) == Some("multimodal") => {
            other.get("text").and_then(Value::as_str).unwrap_or_default().to_string()
        }
        other => other.to_string(),
    }
}

/// Approximate chars this slice occupies in the model-facing history, after
/// the per-turn tool-result clipping has done its part.
fn live_size(msgs: &[Message]) -> usize {
    let keep_from = msgs.len().saturating_sub(TRIM_KEEP_RECENT);
    msgs.iter()
        .enumerate()
        .map(|(i, m)| {
            let n = text_of(m).chars().count();
            if m.role == "tool" && i < keep_from {
                n.min(TOOL_RESULT_CLIP)
            } else {
                n
            }
        })
        .sum()
}

/// Boundary index for compaction: messages `[0, idx)` are summarized,
/// `[idx, ..)` stay verbatim. Snaps to a user message — starting the kept
/// window mid-turn could orphan a tool result from its tool_call, which some
/// providers reject. Prefers the nearest user message at or before the target
/// (keeps a little more); falls back to the first one after it. None = no
/// safe boundary, skip this round.
fn split_point(msgs: &[Message]) -> Option<usize> {
    if msgs.len() <= COMPACT_KEEP_RECENT {
        return None;
    }
    let want = msgs.len() - COMPACT_KEEP_RECENT;
    let at_or_before = (1..=want).rev().find(|&i| msgs[i].role == "user");
    let after = (want + 1..msgs.len()).find(|&i| msgs[i].role == "user");
    at_or_before.or(after)
}

/// Render messages as a plain transcript for the summarizer — roles labeled,
/// tool calls named, every piece clipped so one fat result can't eat the
/// whole budget.
fn render(msgs: &[Message]) -> String {
    let mut out = String::new();
    for m in msgs {
        match m.role.as_str() {
            "user" => out.push_str(&format!("[user] {}\n", clip(&text_of(m), RENDER_MSG_CAP))),
            "assistant" => {
                out.push_str(&format!("[assistant] {}\n", clip(&text_of(m), RENDER_MSG_CAP)))
            }
            "tool_call" => {
                // JSON array of {call_id, fn_name, fn_arguments}.
                let calls: Value = serde_json::from_str(&m.content).unwrap_or(Value::Null);
                for c in calls.as_array().map(Vec::as_slice).unwrap_or_default() {
                    let name = c["fn_name"].as_str().unwrap_or("?");
                    let args = clip(&c["fn_arguments"].to_string(), 300);
                    out.push_str(&format!("[assistant called tool] {name}({args})\n"));
                }
            }
            "tool" => {
                out.push_str(&format!("[tool result] {}\n", clip(&text_of(m), RENDER_MSG_CAP)))
            }
            _ => {}
        }
    }
    // If still over budget, keep the newest part — the previous summary
    // (prepended by the caller) already covers the oldest ground.
    let total = out.chars().count();
    if total > RENDER_TOTAL_CAP {
        out = format!("[… transcript head elided …]\n{}", skip_chars(&out, total - RENDER_TOTAL_CAP));
    }
    out
}

/// First `max` chars with an ellipsis — char-boundary safe.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

fn skip_chars(s: &str, n: usize) -> String {
    s.chars().skip(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> Message {
        Message {
            id: String::new(),
            session_id: String::new(),
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
            created_at: String::new(),
        }
    }

    fn tool_chat(content_len: usize) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: serde_json::json!({ "call_id": "c1", "content": "x".repeat(content_len) }),
        }
    }

    #[test]
    fn old_tool_results_clip_but_recent_stay_verbatim() {
        // 13 tool messages, each larger than the clip: only the first falls
        // outside the 12-message window and gets cut.
        let big = TOOL_RESULT_CLIP + 4_000;
        let mut history: Vec<ChatMessage> = (0..13).map(|_| tool_chat(big)).collect();
        truncate_old_tool_results(&mut history);

        let len = |m: &ChatMessage| m.content["content"].as_str().unwrap().chars().count();
        assert!(len(&history[0]) < big, "old result should be clipped");
        assert!(len(&history[0]) >= TOOL_RESULT_CLIP, "clipped to the budget, not smaller");
        assert!(history[0].content["content"].as_str().unwrap().contains("truncated"));
        assert_eq!(len(&history[1]), big, "recent results stay verbatim");
        assert_eq!(len(&history[12]), big);

        // Non-tool roles are never touched, even when old.
        let mut history = vec![ChatMessage {
            role: "assistant".to_string(),
            content: Value::String("y".repeat(big)),
        }];
        history.extend((0..12).map(|_| tool_chat(10)));
        truncate_old_tool_results(&mut history);
        assert_eq!(history[0].content.as_str().unwrap().len(), big);
    }

    #[test]
    fn split_point_snaps_to_a_user_message() {
        // 20 messages alternating user/assistant: target boundary is index 8,
        // which is a user row — exact snap.
        let msgs: Vec<Message> = (0..20)
            .map(|i| msg(if i % 2 == 0 { "user" } else { "assistant" }, "\"hi\""))
            .collect();
        assert_eq!(split_point(&msgs), Some(8));

        // User messages only at 0 and 15: boundary can't sit at the target
        // (index 8 is mid-turn), so it snaps forward to 15.
        let msgs: Vec<Message> = (0..20)
            .map(|i| msg(if i == 0 || i == 15 { "user" } else { "tool" }, "\"x\""))
            .collect();
        assert_eq!(split_point(&msgs), Some(15));

        // Too short to compact at all.
        let msgs: Vec<Message> = (0..5).map(|_| msg("user", "\"x\"")).collect();
        assert_eq!(split_point(&msgs), None);

        // No user message anywhere (index 0 doesn't count — an empty head
        // would summarize nothing): skip.
        let msgs: Vec<Message> = (0..20).map(|_| msg("tool", "\"x\"")).collect();
        assert_eq!(split_point(&msgs), None);
    }

    #[test]
    fn live_size_counts_multimodal_text_only_and_clips_old_tools() {
        let mm = r#"{"type":"multimodal","text":"four","images":[{"mime":"image/png","b64":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}]}"#;
        assert_eq!(live_size(&[msg("user", mm)]), 4);

        // 13 fat tool results (each over the clip): the one outside the recent
        // window counts clipped, the 12 recent count in full.
        let big = TOOL_RESULT_CLIP + 5_000;
        let fat = format!("\"{}\"", "x".repeat(big));
        let msgs: Vec<Message> = (0..13).map(|_| msg("tool", &fat)).collect();
        assert_eq!(live_size(&msgs), TOOL_RESULT_CLIP + 12 * big);
    }

    #[test]
    fn render_labels_roles_and_names_tool_calls() {
        let msgs = vec![
            msg("user", "\"plan my week\""),
            msg(
                "tool_call",
                r#"[{"call_id":"c1","fn_name":"list_tasks","fn_arguments":{"q":"week"}}]"#,
            ),
            msg("tool", "\"3 tasks found\""),
            msg("assistant", "\"Here is your week.\""),
        ];
        let out = render(&msgs);
        assert!(out.contains("[user] plan my week"));
        assert!(out.contains("[assistant called tool] list_tasks("));
        assert!(out.contains("[tool result] 3 tasks found"));
        assert!(out.contains("[assistant] Here is your week."));
    }
}
