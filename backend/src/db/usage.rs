use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::model_router::{ProviderConfig, TokenUsage};

/// Record one model request's token counts. Best-effort by design — callers
/// pass `Option<TokenUsage>` straight through and a None is a quiet no-op.
pub async fn record(
    pool: &SqlitePool,
    user_id: &str,
    provider: &ProviderConfig,
    purpose: &str,
    usage: Option<TokenUsage>,
) {
    let Some(u) = usage else { return };
    let result = sqlx::query(
        "INSERT INTO usage (id, user_id, provider, model_id, purpose, prompt_tokens, completion_tokens, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(&provider.name)
    .bind(&provider.model_id)
    .bind(purpose)
    .bind(u.prompt)
    .bind(u.completion)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await;
    if let Err(e) = result {
        tracing::warn!("usage record failed: {e}");
    }
}

/// One aggregated row of the admin usage summary.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UsageSummary {
    pub username: String,
    pub provider: String,
    pub model_id: String,
    pub purpose: String,
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

/// Totals per user/provider/model/purpose over the trailing `days`.
pub async fn summary(pool: &SqlitePool, days: i64) -> Result<Vec<UsageSummary>> {
    let since = (Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    Ok(sqlx::query_as::<_, UsageSummary>(
        "SELECT COALESCE(a.username, u.user_id) AS username,
                u.provider, u.model_id, u.purpose,
                COUNT(*) AS requests,
                SUM(u.prompt_tokens) AS prompt_tokens,
                SUM(u.completion_tokens) AS completion_tokens
         FROM usage u
         LEFT JOIN auth_users a ON a.id = u.user_id
         WHERE u.created_at >= ?
         GROUP BY username, u.provider, u.model_id, u.purpose
         ORDER BY SUM(u.prompt_tokens) + SUM(u.completion_tokens) DESC",
    )
    .bind(since)
    .fetch_all(pool)
    .await?)
}
