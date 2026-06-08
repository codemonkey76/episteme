//! GitHub integration — read-only access to the user's repositories via a
//! Personal Access Token. Connected per Episteme user from Settings →
//! Integrations: only the token is stored. The chat agent's `github_*` tools
//! all go through `request`, so it can look at commits/files/PRs and cite them
//! (e.g. in a helpdesk reply). Deliberately read-only — no write scopes used.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::AppState;

const API: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";

pub fn config_key(user_id: &str) -> String {
    format!("github_config:{user_id}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubConfig {
    /// Personal access token (fine-grained or classic, repo-read scope).
    pub token: String,
    /// GitHub login the token belongs to (display only).
    pub login: String,
    /// Default owner (user/org) so the agent can use a bare repo name.
    #[serde(default)]
    pub default_owner: Option<String>,
}

pub async fn config(state: &AppState, user_id: &str) -> Result<GithubConfig> {
    crate::db::settings::get::<GithubConfig>(&state.db, &config_key(user_id))
        .await?
        .ok_or_else(|| anyhow!("GitHub not connected — add it in Settings → Integrations"))
}

/// Verify a token and return its GitHub login (`GET /user`).
pub async fn verify(state: &AppState, token: &str) -> Result<String> {
    let res = state
        .http_client
        .get(format!("{API}/user"))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
        .header(reqwest::header::USER_AGENT, "episteme")
        .send()
        .await
        .map_err(|e| anyhow!("could not reach GitHub: {e}"))?;
    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = body["message"].as_str().unwrap_or("token rejected");
        return Err(anyhow!("GitHub rejected the token ({status}): {msg}"));
    }
    body["login"].as_str().map(str::to_string).ok_or_else(|| anyhow!("GitHub response missing login"))
}

/// Authenticated GitHub API call. `path` is relative to the API root, e.g.
/// `/repos/owner/name/commits/SHA`.
pub async fn request(
    state: &AppState,
    user_id: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let cfg = config(state, user_id).await?;
    let mut req = state
        .http_client
        .request(method, format!("{API}{path}"))
        .bearer_auth(&cfg.token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
        .header(reqwest::header::USER_AGENT, "episteme");
    if let Some(b) = body {
        req = req.json(b);
    }
    let res = req.send().await.map_err(|e| anyhow!("GitHub request failed: {e}"))?;
    let status = res.status();
    let parsed: Value = res.json().await.unwrap_or(Value::Null);
    if status.as_u16() == 401 {
        return Err(anyhow!("GitHub token invalid or expired — reconnect in Settings → Integrations"));
    }
    if !status.is_success() {
        let msg = parsed["message"].as_str().unwrap_or("GitHub API error");
        return Err(anyhow!("GitHub API {status}: {msg}"));
    }
    Ok(parsed)
}

/// Resolve a `repo` argument to `owner/name`: pass through when it already has
/// an owner, else prepend the configured default owner.
pub async fn resolve_repo(state: &AppState, user_id: &str, repo: &str) -> Result<String> {
    let repo = repo.trim().trim_start_matches('/');
    if repo.is_empty() {
        return Err(anyhow!("repo is required"));
    }
    if repo.contains('/') {
        return Ok(repo.to_string());
    }
    let cfg = config(state, user_id).await?;
    match cfg.default_owner.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(owner) => Ok(format!("{owner}/{repo}")),
        None => Err(anyhow!(
            "repo '{repo}' has no owner — use 'owner/{repo}', or set a default owner in Settings"
        )),
    }
}
