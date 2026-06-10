//! Named integration instances (helpdesk/phoneus/github) per user. Replaces the
//! old one-config-per-type settings keys; see migration 026 and
//! `integrations::registry` for instance resolution.

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Integration {
    pub id: String,
    #[serde(skip)]
    pub user_id: String,
    pub kind: String,
    pub name: String,
    pub is_default: bool,
    /// Type-specific JSON (token, base_url, email, …). Never serialized to the
    /// UI verbatim — routes return a masked view.
    #[serde(skip)]
    pub config: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Integration {
    /// Deserialize the type-specific config into a concrete struct.
    pub fn parse_config<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        Ok(serde_json::from_str(&self.config)?)
    }
}

const COLS: &str =
    "id, user_id, kind, name, is_default, config, created_at, updated_at";

pub async fn list(pool: &SqlitePool, user_id: &str) -> Result<Vec<Integration>> {
    Ok(sqlx::query_as::<_, Integration>(&format!(
        "SELECT {COLS} FROM integrations WHERE user_id = ? ORDER BY kind, name"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

pub async fn list_by_kind(pool: &SqlitePool, user_id: &str, kind: &str) -> Result<Vec<Integration>> {
    Ok(sqlx::query_as::<_, Integration>(&format!(
        "SELECT {COLS} FROM integrations WHERE user_id = ? AND kind = ? ORDER BY name"
    ))
    .bind(user_id)
    .bind(kind)
    .fetch_all(pool)
    .await?)
}

pub async fn get(pool: &SqlitePool, user_id: &str, id: &str) -> Result<Option<Integration>> {
    Ok(sqlx::query_as::<_, Integration>(&format!(
        "SELECT {COLS} FROM integrations WHERE id = ? AND user_id = ?"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn count_by_kind(pool: &SqlitePool, user_id: &str, kind: &str) -> Result<i64> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations WHERE user_id = ? AND kind = ?")
            .bind(user_id)
            .bind(kind)
            .fetch_one(pool)
            .await?;
    Ok(n)
}

pub async fn insert(
    pool: &SqlitePool,
    user_id: &str,
    kind: &str,
    name: &str,
    is_default: bool,
    config: &str,
) -> Result<Integration> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO integrations (id, user_id, kind, name, is_default, config, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(kind)
    .bind(name)
    .bind(is_default)
    .bind(config)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    if is_default {
        set_default(pool, user_id, &id).await?;
    }
    Ok(Integration {
        id,
        user_id: user_id.to_string(),
        kind: kind.to_string(),
        name: name.to_string(),
        is_default,
        config: config.to_string(),
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Update an instance's name and/or config (config empty = leave as-is, so a
/// rename needn't re-send the token).
pub async fn update(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
    name: &str,
    config: Option<&str>,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    match config {
        Some(cfg) => {
            sqlx::query(
                "UPDATE integrations SET name = ?, config = ?, updated_at = ? WHERE id = ? AND user_id = ?",
            )
            .bind(name)
            .bind(cfg)
            .bind(&now)
            .bind(id)
            .bind(user_id)
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query("UPDATE integrations SET name = ?, updated_at = ? WHERE id = ? AND user_id = ?")
                .bind(name)
                .bind(&now)
                .bind(id)
                .bind(user_id)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

pub async fn delete(pool: &SqlitePool, user_id: &str, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM integrations WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Make `id` the default for its kind (clears the previous default).
pub async fn set_default(pool: &SqlitePool, user_id: &str, id: &str) -> Result<()> {
    let Some(target) = get(pool, user_id, id).await? else {
        return Ok(());
    };
    sqlx::query("UPDATE integrations SET is_default = 0 WHERE user_id = ? AND kind = ?")
        .bind(user_id)
        .bind(&target.kind)
        .execute(pool)
        .await?;
    sqlx::query("UPDATE integrations SET is_default = 1 WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// One-time migration of the legacy single-config settings keys into rows, as
/// the default instance of their kind. Idempotent: deletes each key it imports,
/// and skips kinds the user already has rows for.
pub async fn import_legacy(pool: &SqlitePool, user_id: &str) -> Result<()> {
    for (kind, label) in [("helpdesk", "Helpdesk"), ("phoneus", "PhoneUs"), ("github", "GitHub")] {
        let key = format!("{kind}_config:{user_id}");
        let legacy: Option<Value> = crate::db::settings::get(pool, &key).await?;
        let Some(cfg) = legacy else { continue };
        // Don't duplicate if rows already exist for this kind.
        if count_by_kind(pool, user_id, kind).await? == 0 {
            insert(pool, user_id, kind, label, true, &cfg.to_string()).await?;
        }
        crate::db::settings::delete(pool, &key).await?;
    }
    Ok(())
}
