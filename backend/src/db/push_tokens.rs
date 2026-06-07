use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;

/// Upsert a device token; re-registering moves it to the current user (a
/// device that logs into a different account must stop notifying the old one).
pub async fn register(pool: &SqlitePool, user_id: &str, token: &str, platform: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO push_tokens (token, user_id, platform, created_at) VALUES (?, ?, ?, ?)
         ON CONFLICT(token) DO UPDATE SET user_id = excluded.user_id, platform = excluded.platform",
    )
    .bind(token)
    .bind(user_id)
    .bind(platform)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// FCM device tokens: everything the mobile app registered (not web).
pub async fn list_mobile(pool: &SqlitePool, user_id: &str) -> Result<Vec<String>> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT token FROM push_tokens WHERE user_id = ? AND platform != 'web'")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(t,)| t).collect())
}

/// Web Push subscription JSONs registered by browsers.
pub async fn list_web(pool: &SqlitePool, user_id: &str) -> Result<Vec<String>> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT token FROM push_tokens WHERE user_id = ? AND platform = 'web'")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(t,)| t).collect())
}

/// Drop a token its push service reported as unregistered/invalid.
pub async fn remove(pool: &SqlitePool, token: &str) -> Result<()> {
    sqlx::query("DELETE FROM push_tokens WHERE token = ?")
        .bind(token)
        .execute(pool)
        .await?;
    Ok(())
}
