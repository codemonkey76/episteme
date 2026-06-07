use anyhow::Result;
use sqlx::SqlitePool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub created_at: String,
    pub role: String,   // admin | member
    pub status: String, // active | disabled
    /// Base32 TOTP secret; set = two-factor is enabled for this account.
    pub totp_secret: Option<String>,
    /// Secret awaiting first-code verification during enrollment.
    pub totp_pending: Option<String>,
    /// Last accepted 30s timestep — rejects replay of a still-valid code.
    pub totp_last_step: Option<i64>,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
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
    role: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO auth_users (id, username, password_hash, created_at, role, status)
         VALUES (?, ?, ?, ?, ?, 'active')",
    )
    .bind(id)
    .bind(username)
    .bind(password_hash)
    .bind(created_at)
    .bind(role)
    .execute(pool)
    .await?;
    Ok(())
}

/// All accounts, oldest first — for the admin's Users page.
pub async fn list_users(pool: &SqlitePool) -> Result<Vec<User>> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, created_at, role, status,
                totp_secret, totp_pending, totp_last_step
         FROM auth_users ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn get_user(pool: &SqlitePool, id: &str) -> Result<Option<User>> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, created_at, role, status,
                totp_secret, totp_pending, totp_last_step
         FROM auth_users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn set_status(pool: &SqlitePool, id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE auth_users SET status = ? WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Remove an account; their sessions cascade, their data is deleted here.
pub async fn delete_user(pool: &SqlitePool, id: &str) -> Result<()> {
    for table in ["sessions", "tasks", "notes", "memories", "suggestions"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE user_id = ?"))
            .bind(id)
            .execute(pool)
            .await?;
    }
    sqlx::query("DELETE FROM settings WHERE key LIKE '%:' || ?")
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM auth_users WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_user_by_username(pool: &SqlitePool, username: &str) -> Result<Option<User>> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, created_at, role, status,
                totp_secret, totp_pending, totp_last_step
         FROM auth_users WHERE username = ?",
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

// ── Two-factor (TOTP) ───────────────────────────────────────────────────────

/// Stage a new secret for enrollment; harmless to overwrite an abandoned one.
pub async fn set_totp_pending(pool: &SqlitePool, user_id: &str, secret: &str) -> Result<()> {
    sqlx::query("UPDATE auth_users SET totp_pending = ? WHERE id = ?")
        .bind(secret)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Promote the pending secret to active (first code verified). False when no
/// enrollment was in progress.
pub async fn enable_totp(pool: &SqlitePool, user_id: &str) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE auth_users SET totp_secret = totp_pending, totp_pending = NULL,
                totp_last_step = NULL
         WHERE id = ? AND totp_pending IS NOT NULL",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

pub async fn disable_totp(pool: &SqlitePool, user_id: &str) -> Result<()> {
    sqlx::query(
        "UPDATE auth_users SET totp_secret = NULL, totp_pending = NULL, totp_last_step = NULL
         WHERE id = ?",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM auth_recovery_codes WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Atomically claim a TOTP timestep: true exactly once per step, so a code
/// can't be replayed within its validity window.
pub async fn claim_totp_step(pool: &SqlitePool, user_id: &str, step: i64) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE auth_users SET totp_last_step = ?
         WHERE id = ? AND (totp_last_step IS NULL OR totp_last_step < ?)",
    )
    .bind(step)
    .bind(user_id)
    .bind(step)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

/// Store freshly generated recovery-code hashes (replacing any earlier set).
pub async fn replace_recovery_codes(
    pool: &SqlitePool,
    user_id: &str,
    hashes: &[String],
) -> Result<()> {
    sqlx::query("DELETE FROM auth_recovery_codes WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    let now = chrono::Utc::now().to_rfc3339();
    for hash in hashes {
        sqlx::query(
            "INSERT INTO auth_recovery_codes (user_id, code_hash, created_at) VALUES (?, ?, ?)",
        )
        .bind(user_id)
        .bind(hash)
        .bind(&now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Consume a recovery code: true exactly once per code.
pub async fn use_recovery_code(pool: &SqlitePool, user_id: &str, hash: &str) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE auth_recovery_codes SET used_at = ?
         WHERE user_id = ? AND code_hash = ? AND used_at IS NULL",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(user_id)
    .bind(hash)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

/// Unused recovery codes remaining — surfaced in Settings.
pub async fn recovery_codes_left(pool: &SqlitePool, user_id: &str) -> Result<i64> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM auth_recovery_codes WHERE user_id = ? AND used_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(n)
}

pub async fn create_session(
    pool: &SqlitePool,
    token: &str,
    user_id: &str,
    created_at: &str,
    expires_at: &str,
) -> Result<()> {
    create_session_as(pool, token, user_id, created_at, expires_at, None).await
}

/// Create a session, optionally recording the admin impersonating `user_id`.
pub async fn create_session_as(
    pool: &SqlitePool,
    token: &str,
    user_id: &str,
    created_at: &str,
    expires_at: &str,
    impersonator_id: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO auth_sessions (token, user_id, created_at, expires_at, impersonator_id)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(token)
    .bind(user_id)
    .bind(created_at)
    .bind(expires_at)
    .bind(impersonator_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// The impersonating admin for a session token, if any.
pub async fn session_impersonator(pool: &SqlitePool, token: &str) -> Result<Option<User>> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT u.id, u.username, u.password_hash, u.created_at, u.role, u.status,
                u.totp_secret, u.totp_pending, u.totp_last_step
         FROM auth_sessions s
         JOIN auth_users u ON u.id = s.impersonator_id
         WHERE s.token = ?",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?)
}

/// Resolve a session token to its user, enforcing expiry (`now` as RFC3339).
pub async fn session_user(pool: &SqlitePool, token: &str, now: &str) -> Result<Option<User>> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT u.id, u.username, u.password_hash, u.created_at, u.role, u.status,
                u.totp_secret, u.totp_pending, u.totp_last_step
         FROM auth_sessions s
         JOIN auth_users u ON u.id = s.user_id
         WHERE s.token = ? AND s.expires_at > ? AND u.status = 'active'",
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
