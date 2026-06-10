use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

pub mod auth;
pub mod integrations;
pub mod sessions;
pub mod messages;
pub mod settings;
pub mod pending_actions;
pub mod push_tokens;
pub mod reports;
pub mod documents;
pub mod email_index;
pub mod logs;
pub mod memories;
pub mod tasks;
pub mod notes;
pub mod suggestions;
pub mod usage;
pub mod invites;
pub mod jobs;
pub mod research_checkpoint;
pub mod terminal_history;
pub mod terminal_output;

pub async fn init(url: &str) -> Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await?;

    sqlx::migrate!("src/db/migrations").run(&pool).await?;

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Jobs + parked approvals: the suspend/resume bookkeeping that the
    /// approval-queue flow depends on (exactly-once resume, pending counts,
    /// the per-user global queue).
    #[tokio::test]
    async fn job_parking_and_resume_gate() {
        let pool = init("sqlite::memory:").await.expect("migrations should run");
        sqlx::query("INSERT INTO auth_users (id, username, password_hash, role, created_at) VALUES ('u1', 'test', 'x', 'admin', '2026-01-01')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO sessions (id, user_id, title, created_at, updated_at) VALUES ('s1', 'u1', '⚙ digest', '2026-01-01', '2026-01-01')")
            .execute(&pool).await.unwrap();

        let job = jobs::insert(&pool, "u1", "s1", "background", "digest", "", None).await.unwrap();
        assert_eq!(job.status, "running");

        // Two parked calls suspend the job.
        let a = pending_actions::insert_parked(&pool, "s1", "create_note", "{}", "call_1").await.unwrap();
        let b = pending_actions::insert_parked(&pool, "s1", "email_draft", "{}", "call_2").await.unwrap();
        assert_eq!(a.call_id.as_deref(), Some("call_1"));
        jobs::set_status(&pool, &job.id, "needs_approval", None, None).await.unwrap();

        // Global queue shows both, joined with the session title.
        let queue = pending_actions::list_pending_for_user(&pool, "u1").await.unwrap();
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0].session_title, "⚙ digest");
        assert!(pending_actions::list_pending_for_user(&pool, "u2").await.unwrap().is_empty());

        // Deciding one leaves the job suspended; deciding both clears it.
        pending_actions::resolve(&pool, &a.id, true).await.unwrap();
        assert_eq!(pending_actions::count_pending(&pool, "s1").await.unwrap(), 1);
        pending_actions::resolve(&pool, &b.id, false).await.unwrap();
        assert_eq!(pending_actions::count_pending(&pool, "s1").await.unwrap(), 0);

        // Exactly-once resume gate.
        let suspended = jobs::suspended_for_session(&pool, "s1").await.unwrap().unwrap();
        assert!(jobs::try_resume(&pool, &suspended.id).await.unwrap());
        assert!(!jobs::try_resume(&pool, &suspended.id).await.unwrap());
        assert_eq!(jobs::get(&pool, &job.id).await.unwrap().unwrap().status, "running");

        // Idempotence guard data: a decided action keeps its status.
        let decided = pending_actions::get(&pool, &a.id).await.unwrap().unwrap();
        assert_eq!(decided.status, "approved");
    }

    /// Terminal command history: insert with blank/dup suppression, plus shell
    /// and substring search scoping.
    #[tokio::test]
    async fn terminal_history_insert_and_search() {
        let pool = init("sqlite::memory:").await.expect("migrations should run");

        terminal_history::insert(&pool, "u1", "bash", "ls -la").await.unwrap();
        // Blank and an immediate duplicate are both ignored.
        terminal_history::insert(&pool, "u1", "bash", "   ").await.unwrap();
        terminal_history::insert(&pool, "u1", "bash", "ls -la").await.unwrap();
        terminal_history::insert(&pool, "u1", "bash", "docker ps").await.unwrap();
        terminal_history::insert(&pool, "u1", "pwsh", "Get-ChildItem").await.unwrap();
        terminal_history::insert(&pool, "u2", "bash", "whoami").await.unwrap();

        // Scoped to the user; newest first; dup collapsed.
        let all = terminal_history::list(&pool, "u1", None, None, 100).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].command, "Get-ChildItem");

        // Shell filter.
        let bash = terminal_history::list(&pool, "u1", Some("bash"), None, 100).await.unwrap();
        assert_eq!(bash.len(), 2);

        // Substring search.
        let found = terminal_history::list(&pool, "u1", None, Some("docker"), 100).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "docker ps");

        // Other users are isolated.
        assert_eq!(terminal_history::list(&pool, "u3", None, None, 100).await.unwrap().len(), 0);
    }

    /// Legacy single-config settings keys migrate into integration rows (as the
    /// default instance) and the old key is removed.
    #[tokio::test]
    async fn integrations_import_legacy() {
        let pool = init("sqlite::memory:").await.expect("migrations should run");

        let key = "phoneus_config:u1";
        settings::set(&pool, key, &serde_json::json!({
            "base_url": "https://p", "email": "a@b.c", "token": "tok"
        }))
        .await
        .unwrap();

        integrations::import_legacy(&pool, "u1").await.unwrap();

        let rows = integrations::list_by_kind(&pool, "u1", "phoneus").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_default);
        assert!(rows[0].config.contains("tok"));
        // The legacy key is gone, and a second import is a no-op.
        assert!(settings::get::<serde_json::Value>(&pool, key).await.unwrap().is_none());
        integrations::import_legacy(&pool, "u1").await.unwrap();
        assert_eq!(integrations::count_by_kind(&pool, "u1", "phoneus").await.unwrap(), 1);
    }

    /// Soft-delete hides a memory from the active list/count but keeps it
    /// restorable from the archive.
    #[tokio::test]
    async fn memory_soft_delete_restore() {
        let pool = init("sqlite::memory:").await.expect("migrations should run");

        let a = memories::insert(&pool, "u1", "likes tea", "preference", "auto", None).await.unwrap();
        memories::insert(&pool, "u1", "likes coffee", "preference", "auto", None).await.unwrap();
        assert_eq!(memories::count_active(&pool, "u1").await.unwrap(), 2);

        // Soft-delete the first (as if merged); it leaves the active list.
        memories::soft_delete(&pool, "u1", &a.id, Some("other-id")).await.unwrap();
        assert_eq!(memories::count_active(&pool, "u1").await.unwrap(), 1);
        assert_eq!(memories::list(&pool, "u1", None, None, 100).await.unwrap().len(), 1);

        // It shows in the archive and restores cleanly.
        let archived = memories::list_deleted(&pool, "u1", 100).await.unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, a.id);
        memories::restore(&pool, "u1", &a.id).await.unwrap();
        assert_eq!(memories::count_active(&pool, "u1").await.unwrap(), 2);
        assert!(memories::list_deleted(&pool, "u1", 100).await.unwrap().is_empty());
    }

    /// Terminal output archive: tail replays in order; search is scoped to the
    /// user, strips ANSI, and returns a context snippet.
    #[tokio::test]
    async fn terminal_output_restore_and_search() {
        let pool = init("sqlite::memory:").await.expect("migrations should run");

        terminal_output::append(&pool, "t1", "u1", "bash", b"first\n", "first\n").await.unwrap();
        // Raw keeps the colour codes; the search text is the stripped form.
        terminal_output::append(
            &pool,
            "t1",
            "u1",
            "bash",
            b"\x1b[31mdocker ps\x1b[0m\nCONTAINER\n",
            "docker ps\nCONTAINER\n",
        )
        .await
        .unwrap();
        terminal_output::append(&pool, "t2", "u1", "pwsh", b"other\n", "other\n").await.unwrap();
        terminal_output::append(&pool, "t1", "u2", "bash", b"secret\n", "secret\n").await.unwrap();

        // Tail replays this user's chunks for the terminal in order, raw bytes —
        // and never another user's, even with the same terminal id.
        let tail = terminal_output::restore_tail(&pool, "u1", "t1").await.unwrap();
        assert_eq!(tail, b"first\n\x1b[31mdocker ps\x1b[0m\nCONTAINER\n");
        let other = terminal_output::restore_tail(&pool, "u2", "t1").await.unwrap();
        assert_eq!(other, b"secret\n");

        // Search matches across the user's archive (ANSI-stripped), newest first.
        let hits = terminal_output::search(&pool, "u1", "docker", 100).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].terminal_id, "t1");
        assert!(hits[0].snippet.contains("docker ps"));

        // Scoped to the user; other users' output is invisible.
        assert_eq!(terminal_output::search(&pool, "u1", "secret", 100).await.unwrap().len(), 0);
        assert_eq!(terminal_output::search(&pool, "u2", "secret", 100).await.unwrap().len(), 1);
    }

    /// Report share links: mint, public lookup by token, owner scoping, revoke.
    #[tokio::test]
    async fn report_share_links() {
        let pool = init("sqlite::memory:").await.expect("migrations should run");
        sqlx::query("INSERT INTO auth_users (id, username, password_hash, role, created_at) VALUES ('u1', 'a', 'x', 'admin', '2026-01-01'), ('u2', 'b', 'x', 'member', '2026-01-01')")
            .execute(&pool).await.unwrap();
        let id = reports::insert(&pool, "u1", None, "NVR options", "<h1>report</h1>").await.unwrap();

        // Fresh report: found but private.
        assert_eq!(reports::share_token(&pool, "u1", &id).await.unwrap(), Some(None));
        // Another user can't see or touch it.
        assert_eq!(reports::share_token(&pool, "u2", &id).await.unwrap(), None);
        assert!(!reports::set_share_token(&pool, "u2", &id, Some("tok")).await.unwrap());

        // Owner mints a token; the public lookup then resolves to the HTML.
        assert!(reports::set_share_token(&pool, "u1", &id, Some("tok123")).await.unwrap());
        assert_eq!(reports::share_token(&pool, "u1", &id).await.unwrap(), Some(Some("tok123".into())));
        let (title, html) = reports::get_shared(&pool, "tok123").await.unwrap().unwrap();
        assert_eq!(title, "NVR options");
        assert!(html.contains("report"));

        // Revoke: the token stops resolving.
        assert!(reports::set_share_token(&pool, "u1", &id, None).await.unwrap());
        assert!(reports::get_shared(&pool, "tok123").await.unwrap().is_none());
        // The share state also shows in the listing.
        let metas = reports::list_for_user(&pool, "u1", 10).await.unwrap();
        assert!(metas[0].share_token.is_none());
    }

    /// Job recovery: startup orphan sweep and user-initiated cancel.
    #[tokio::test]
    async fn job_cancel_and_orphan_sweep() {
        let pool = init("sqlite::memory:").await.expect("migrations should run");
        sqlx::query("INSERT INTO auth_users (id, username, password_hash, role, created_at) VALUES ('u1', 'test', 'x', 'admin', '2026-01-01')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO sessions (id, user_id, title, created_at, updated_at) VALUES ('s1', 'u1', '🔎 topic', '2026-01-01', '2026-01-01')")
            .execute(&pool).await.unwrap();

        let job = jobs::insert(&pool, "u1", "s1", "research", "Research: topic", "", None)
            .await
            .unwrap();

        // Cancel needs ownership and an in-flight status.
        assert!(!jobs::cancel(&pool, "u2", &job.id).await.unwrap()); // not theirs
        assert!(jobs::cancel(&pool, "u1", &job.id).await.unwrap());
        let after = jobs::get(&pool, &job.id).await.unwrap().unwrap();
        assert_eq!(after.status, "failed");
        assert_eq!(after.error.as_deref(), Some("cancelled"));
        // A finished job can't be cancelled again.
        assert!(!jobs::cancel(&pool, "u1", &job.id).await.unwrap());

        // Orphan recovery lists only what's still running; the cancelled job
        // (now failed) isn't among them.
        let bg = jobs::insert(&pool, "u1", "s1", "background", "Digest", "", None).await.unwrap();
        let research =
            jobs::insert(&pool, "u1", "s1", "research", "Another", "", None).await.unwrap();
        let orphans = jobs::list_orphaned_running(&pool).await.unwrap();
        let ids: Vec<&str> = orphans.iter().map(|j| j.id.as_str()).collect();
        assert!(ids.contains(&bg.id.as_str()) && ids.contains(&research.id.as_str()));
        assert!(!ids.contains(&job.id.as_str()));

        // The non-restartable kind fails with the restart message.
        jobs::fail_orphaned(&pool, &bg.id).await.unwrap();
        let swept = jobs::get(&pool, &bg.id).await.unwrap().unwrap();
        assert_eq!(swept.status, "failed");
        assert!(swept.error.unwrap().contains("restart"));
    }

    /// TOTP lifecycle: pending → enabled, replay-proof step claims, and
    /// single-use recovery codes.
    #[tokio::test]
    async fn totp_lifecycle_and_replay_protection() {
        let pool = init("sqlite::memory:").await.expect("migrations should run");
        sqlx::query("INSERT INTO auth_users (id, username, password_hash, role, created_at) VALUES ('u1', 'test', 'x', 'admin', '2026-01-01')")
            .execute(&pool).await.unwrap();

        // Enable without an enrollment in progress is refused.
        assert!(!auth::enable_totp(&pool, "u1").await.unwrap());

        // Stage → promote: secret moves, pending clears.
        auth::set_totp_pending(&pool, "u1", "JBSWY3DPEHPK3PXP").await.unwrap();
        assert!(auth::enable_totp(&pool, "u1").await.unwrap());
        let user = auth::get_user(&pool, "u1").await.unwrap().unwrap();
        assert_eq!(user.totp_secret.as_deref(), Some("JBSWY3DPEHPK3PXP"));
        assert!(user.totp_pending.is_none());

        // A timestep can be claimed exactly once; later steps still work,
        // earlier ones (replays) never.
        assert!(auth::claim_totp_step(&pool, "u1", 100).await.unwrap());
        assert!(!auth::claim_totp_step(&pool, "u1", 100).await.unwrap());
        assert!(!auth::claim_totp_step(&pool, "u1", 99).await.unwrap());
        assert!(auth::claim_totp_step(&pool, "u1", 101).await.unwrap());

        // Recovery codes: single-use, counted, gone on disable.
        auth::replace_recovery_codes(&pool, "u1", &["h1".into(), "h2".into()]).await.unwrap();
        assert_eq!(auth::recovery_codes_left(&pool, "u1").await.unwrap(), 2);
        assert!(auth::use_recovery_code(&pool, "u1", "h1").await.unwrap());
        assert!(!auth::use_recovery_code(&pool, "u1", "h1").await.unwrap());
        assert!(!auth::use_recovery_code(&pool, "u1", "nope").await.unwrap());
        assert_eq!(auth::recovery_codes_left(&pool, "u1").await.unwrap(), 1);

        auth::disable_totp(&pool, "u1").await.unwrap();
        let user = auth::get_user(&pool, "u1").await.unwrap().unwrap();
        assert!(user.totp_secret.is_none() && user.totp_pending.is_none());
        // Disable also reset the step guard: an old step claims again.
        assert!(auth::claim_totp_step(&pool, "u1", 1).await.unwrap());
        assert_eq!(auth::recovery_codes_left(&pool, "u1").await.unwrap(), 0);
    }

    /// Email index: dedupe lookup, per-mailbox listing, and the size cap.
    #[tokio::test]
    async fn email_index_round_trip_and_prune() {
        let pool = init("sqlite::memory:").await.expect("migrations should run");
        sqlx::query("INSERT INTO auth_users (id, username, password_hash, role, created_at) VALUES ('u1', 'test', 'x', 'admin', '2026-01-01')")
            .execute(&pool).await.unwrap();

        let blob = crate::integrations::embeddings::to_blob(&[0.1, 0.2]);
        for (id, mailbox, received) in
            [("m1", "", "2026-06-01"), ("m2", "", "2026-06-02"), ("m3", "shared@x.com", "2026-06-03")]
        {
            email_index::insert(&pool, "u1", id, mailbox, "Subj", "Jo <jo@x.com>", "snip", received, &blob)
                .await
                .unwrap();
        }
        // Duplicate insert is ignored, not an error.
        email_index::insert(&pool, "u1", "m1", "", "Other", "x", "y", "2026-06-09", &blob)
            .await
            .unwrap();

        let seen = email_index::existing_ids(&pool, "u1", &["m1", "m9"]).await.unwrap();
        assert_eq!(seen, ["m1"]);

        // Listing is per-mailbox, newest first; other users see nothing.
        let own = email_index::list_for_mailbox(&pool, "u1", "").await.unwrap();
        assert_eq!(own.iter().map(|r| r.message_id.as_str()).collect::<Vec<_>>(), ["m2", "m1"]);
        assert_eq!(own[0].subject, "Subj");
        assert!(email_index::list_for_mailbox(&pool, "u2", "").await.unwrap().is_empty());

        // Prune keeps the newest rows across mailboxes.
        email_index::prune(&pool, "u1", 2).await.unwrap();
        assert!(email_index::list_for_mailbox(&pool, "u1", "").await.unwrap().len() == 1);
        assert_eq!(email_index::list_for_mailbox(&pool, "u1", "shared@x.com").await.unwrap().len(), 1);
    }

    /// Compaction state round-trips and the created_at cursor slices the
    /// model-facing history while the full transcript stays listable.
    #[tokio::test]
    async fn compaction_cursor_slices_model_history() {
        let pool = init("sqlite::memory:").await.expect("migrations should run");
        sqlx::query("INSERT INTO auth_users (id, username, password_hash, role, created_at) VALUES ('u1', 'test', 'x', 'admin', '2026-01-01')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO sessions (id, user_id, title, created_at, updated_at) VALUES ('s1', 'u1', 'long chat', '2026-01-01', '2026-01-01')")
            .execute(&pool).await.unwrap();

        let m1 = messages::insert(&pool, "s1", "user", "\"old\"", None, None).await.unwrap();
        let m2 = messages::insert(&pool, "s1", "assistant", "\"older reply\"", None, None).await.unwrap();
        let m3 = messages::insert(&pool, "s1", "user", "\"new\"", None, None).await.unwrap();
        assert!(m1.created_at <= m2.created_at && m2.created_at <= m3.created_at);

        // Fresh session: no summary, cursor None, everything is live.
        assert_eq!(sessions::compaction(&pool, "s1").await.unwrap(), (None, None));
        assert_eq!(messages::list_for_session_after(&pool, "s1", None).await.unwrap().len(), 3);

        // Compact through m2: only m3 remains model-facing.
        sessions::set_compaction(&pool, "s1", "user and assistant exchanged olds", &m2.created_at)
            .await
            .unwrap();
        let (summary, until) = sessions::compaction(&pool, "s1").await.unwrap();
        assert_eq!(summary.as_deref(), Some("user and assistant exchanged olds"));
        let live =
            messages::list_for_session_after(&pool, "s1", until.as_deref()).await.unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, m3.id);

        // The display transcript is untouched.
        assert_eq!(messages::list_for_session(&pool, "s1").await.unwrap().len(), 3);
    }

    /// End-to-end check that the migrations run on the bundled SQLite (FTS5
    /// included) and the message_fts triggers index/search/cleanup correctly.
    #[tokio::test]
    async fn message_search_via_fts5() {
        let pool = init("sqlite::memory:").await.expect("migrations should run");

        sqlx::query("INSERT INTO auth_users (id, username, password_hash, role, created_at) VALUES ('u1', 'test', 'x', 'admin', '2026-01-01')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO sessions (id, user_id, title, created_at, updated_at) VALUES ('s1', 'u1', 'Router chat', '2026-01-01', '2026-01-01')")
            .execute(&pool).await.unwrap();

        // JSON-string content (the normal shape) and a multimodal object.
        messages::insert(&pool, "s1", "user", "\"my OPNsense router drops VPN packets\"", None, None).await.unwrap();
        messages::insert(&pool, "s1", "user", r#"{"type":"multimodal","text":"screenshot of the firewall rules","images":[{"mime":"image/png","b64":"AAAA"}]}"#, None, None).await.unwrap();
        messages::insert(&pool, "s1", "tool", "\"router internals\"", Some("c1"), None).await.unwrap();

        // Plain-text term from a JSON-string row.
        let hits = messages::search(&pool, "u1", "opnsense vpn", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_title, "Router chat");
        assert!(hits[0].snippet.to_lowercase().contains("opnsense"));

        // Multimodal rows index only their text part; base64 never matches.
        assert_eq!(messages::search(&pool, "u1", "firewall", 10).await.unwrap().len(), 1);
        assert!(messages::search(&pool, "u1", "AAAA", 10).await.unwrap().is_empty());

        // Tool rows aren't indexed; other users see nothing; deletes clean up.
        assert!(messages::search(&pool, "u1", "internals", 10).await.unwrap().is_empty());
        assert!(messages::search(&pool, "u2", "opnsense", 10).await.unwrap().is_empty());
        sqlx::query("DELETE FROM sessions WHERE id = 's1'").execute(&pool).await.unwrap();
        assert!(messages::search(&pool, "u1", "opnsense", 10).await.unwrap().is_empty());
    }
}
