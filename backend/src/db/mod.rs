use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

pub mod auth;
pub mod sessions;
pub mod messages;
pub mod settings;
pub mod pending_actions;
pub mod documents;
pub mod logs;
pub mod memories;
pub mod tasks;
pub mod notes;
pub mod suggestions;
pub mod invites;

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
