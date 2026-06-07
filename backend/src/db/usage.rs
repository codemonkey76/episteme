use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::model_router::{ProviderConfig, TokenUsage};

/// One row of the admin-managed price table (settings key `model_prices`):
/// US$ per million tokens for any model id containing `model` (longest match
/// wins). Unpriced models — local Ollama, typically — simply show no cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPrice {
    pub model: String,
    #[serde(default)]
    pub prompt_per_mtok: f64,
    #[serde(default)]
    pub completion_per_mtok: f64,
}

/// Dollar cost of a usage row under the price table, None when no entry
/// matches. Case-insensitive substring match; the longest (most specific)
/// pattern wins, so "gpt-4o-mini" beats "gpt-4o" for gpt-4o-mini traffic.
pub fn cost_for(
    prices: &[ModelPrice],
    model_id: &str,
    prompt_tokens: i64,
    completion_tokens: i64,
) -> Option<f64> {
    let id = model_id.to_ascii_lowercase();
    let price = prices
        .iter()
        .filter(|p| !p.model.trim().is_empty() && id.contains(&p.model.trim().to_ascii_lowercase()))
        .max_by_key(|p| p.model.trim().len())?;
    Some(
        prompt_tokens as f64 / 1e6 * price.prompt_per_mtok
            + completion_tokens as f64 / 1e6 * price.completion_per_mtok,
    )
}

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
    /// Filled by the summary route from the price table; not a DB column.
    #[sqlx(default)]
    pub cost: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prices() -> Vec<ModelPrice> {
        vec![
            ModelPrice { model: "gpt-4o".into(), prompt_per_mtok: 2.5, completion_per_mtok: 10.0 },
            ModelPrice { model: "gpt-4o-mini".into(), prompt_per_mtok: 0.15, completion_per_mtok: 0.6 },
            ModelPrice { model: "claude-sonnet".into(), prompt_per_mtok: 3.0, completion_per_mtok: 15.0 },
        ]
    }

    #[test]
    fn cost_for_longest_match_wins_and_is_case_insensitive() {
        // 1M prompt + 1M completion at the mini rate, not the gpt-4o rate.
        let c = cost_for(&prices(), "GPT-4o-MINI-2024", 1_000_000, 1_000_000).unwrap();
        assert!((c - 0.75).abs() < 1e-9);
        let c = cost_for(&prices(), "gpt-4o-2024-08-06", 2_000_000, 0).unwrap();
        assert!((c - 5.0).abs() < 1e-9);
    }

    #[test]
    fn cost_for_unpriced_models_yield_none() {
        assert!(cost_for(&prices(), "qwen3:14b", 1_000_000, 0).is_none());
        assert!(cost_for(&[], "gpt-4o", 1, 1).is_none());
        // Blank patterns never match everything.
        let blank = vec![ModelPrice { model: "  ".into(), prompt_per_mtok: 9.0, completion_per_mtok: 9.0 }];
        assert!(cost_for(&blank, "anything", 1, 1).is_none());
    }
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
