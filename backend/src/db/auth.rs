use anyhow::Result;
use sqlx::SqlitePool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub created_at: String,
}

/// Number of admin accounts. Zero means the first-run setup screen should show.
pub async fn count_users(pool: &SqlitePool) -> Result<i64> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM auth_users")
        .fetch_one(pool)
        .await?;
    Ok(n)
}

pub async fn create_user(
    pool: &SqlitePool,
    id: &str,
    username: &str,
    password_hash: &str,
    created_at: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO auth_users (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(username)
    .bind(password_hash)
    .bind(created_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_user_by_username(pool: &SqlitePool, username: &str) -> Result<Option<User>> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, created_at FROM auth_users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(pool)
    .await?)
}

pub async fn update_password(pool: &SqlitePool, user_id: &str, password_hash: &str) -> Result<()> {
    sqlx::query("UPDATE auth_users SET password_hash = ? WHERE id = ?")
        .bind(password_hash)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn create_session(
    pool: &SqlitePool,
    token: &str,
    user_id: &str,
    created_at: &str,
    expires_at: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO auth_sessions (token, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)",
    )
    .bind(token)
    .bind(user_id)
    .bind(created_at)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Resolve a session token to its user, enforcing expiry (`now` as RFC3339).
pub async fn session_user(pool: &SqlitePool, token: &str, now: &str) -> Result<Option<User>> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT u.id, u.username, u.password_hash, u.created_at
         FROM auth_sessions s
         JOIN auth_users u ON u.id = s.user_id
         WHERE s.token = ? AND s.expires_at > ?",
    )
    .bind(token)
    .bind(now)
    .fetch_optional(pool)
    .await?)
}

pub async fn delete_session(pool: &SqlitePool, token: &str) -> Result<()> {
    sqlx::query("DELETE FROM auth_sessions WHERE token = ?")
        .bind(token)
        .execute(pool)
        .await?;
    Ok(())
}

/// Drop every session for a user — used after a password change so old cookies
/// stop working.
pub async fn delete_user_sessions(pool: &SqlitePool, user_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
