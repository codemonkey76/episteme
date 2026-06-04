use anyhow::Result;
use chrono::{Duration, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Invite {
    pub code: String,
    pub label: String,
    pub created_at: String,
    pub expires_at: String,
    pub used_by: Option<String>,
    pub used_at: Option<String>,
}

/// Default invite lifetime.
const EXPIRY_DAYS: i64 = 14;

pub async fn create(pool: &SqlitePool, label: &str) -> Result<Invite> {
    let code = Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires = now + Duration::days(EXPIRY_DAYS);
    sqlx::query(
        "INSERT INTO invites (code, label, created_at, expires_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&code)
    .bind(label)
    .bind(now.to_rfc3339())
    .bind(expires.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(Invite {
        code,
        label: label.to_string(),
        created_at: now.to_rfc3339(),
        expires_at: expires.to_rfc3339(),
        used_by: None,
        used_at: None,
    })
}

/// Newest first, both pending and redeemed.
pub async fn list(pool: &SqlitePool) -> Result<Vec<Invite>> {
    Ok(sqlx::query_as::<_, Invite>(
        "SELECT code, label, created_at, expires_at, used_by, used_at
         FROM invites ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?)
}

/// A code that can still be redeemed: exists, unused, unexpired.
pub async fn get_valid(pool: &SqlitePool, code: &str) -> Result<Option<Invite>> {
    Ok(sqlx::query_as::<_, Invite>(
        "SELECT code, label, created_at, expires_at, used_by, used_at
         FROM invites WHERE code = ? AND used_by IS NULL AND expires_at > ?",
    )
    .bind(code)
    .bind(Utc::now().to_rfc3339())
    .fetch_optional(pool)
    .await?)
}

/// Atomically claim a code — returns false if it was already used/expired
/// (two people racing the same link: exactly one wins).
pub async fn mark_used(pool: &SqlitePool, code: &str, user_id: &str) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE invites SET used_by = ?, used_at = ?
         WHERE code = ? AND used_by IS NULL AND expires_at > ?",
    )
    .bind(user_id)
    .bind(Utc::now().to_rfc3339())
    .bind(code)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

/// Revoke an unused invite.
pub async fn delete(pool: &SqlitePool, code: &str) -> Result<()> {
    sqlx::query("DELETE FROM invites WHERE code = ? AND used_by IS NULL")
        .bind(code)
        .execute(pool)
        .await?;
    Ok(())
}
