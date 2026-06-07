use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

pub mod auth;
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
