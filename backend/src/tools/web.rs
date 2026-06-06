//! Web tools — search via the self-hosted SearXNG sidecar, plus a readable-text
//! page fetcher so the model can follow up on results. Both are read-only.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::integrations::websearch::{fetch_readable, map_results, searxng_url};
use crate::state::AppState;

const RESULTS_DEFAULT: usize = 8;
const RESULTS_MAX: usize = 15;

pub fn schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "web_search",
            "description": "Search the web and return ranked results (title, url, snippet). Use fetch_page on a result url to read it in full.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search terms." },
                    "category": { "type": "string", "enum": ["general", "news", "science", "it"], "description": "Optional result category. Default general." },
                    "time_range": { "type": "string", "enum": ["day", "week", "month", "year"], "description": "Optional recency filter." },
                    "max_results": { "type": "integer", "description": "Default 8, max 15." }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "fetch_page",
            "description": "Fetch a public web page and return its readable text (truncated). Use http(s) urls, e.g. from web_search results.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string" }
                },
                "required": ["url"]
            }
        }),
    ]
}

pub fn handles(name: &str) -> bool {
    matches!(name, "web_search" | "fetch_page")
}

pub async fn execute(state: &AppState, _user_id: &str, name: &str, args: Value) -> Result<Value> {
    match name {
        "web_search" => search(state, args).await,
        "fetch_page" => fetch(state, args).await,
        _ => Err(anyhow!("unknown web tool '{name}'")),
    }
}

async fn search(state: &AppState, args: Value) -> Result<Value> {
    let query = args["query"]
        .as_str()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or_else(|| anyhow!("query is required"))?;
    let max = args["max_results"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(RESULTS_DEFAULT)
        .clamp(1, RESULTS_MAX);

    let mut params: Vec<(&str, &str)> = vec![("q", query), ("format", "json")];
    let category = args["category"].as_str().unwrap_or_default();
    if matches!(category, "general" | "news" | "science" | "it") {
        params.push(("categories", category));
    }
    let time_range = args["time_range"].as_str().unwrap_or_default();
    if matches!(time_range, "day" | "week" | "month" | "year") {
        params.push(("time_range", time_range));
    }

    let response = state
        .http_client
        .get(format!("{}/search", searxng_url()))
        .query(&params)
        .send()
        .await
        .map_err(|e| anyhow!("web search unavailable: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        // 403 is the classic "json not in search.formats" misconfiguration.
        return Err(anyhow!(
            "web search unavailable: SearXNG returned {status} (is `json` enabled under search.formats in searxng/settings.yml?)"
        ));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|e| anyhow!("web search returned invalid JSON: {e}"))?;

    Ok(json!({ "query": query, "results": map_results(&body, max) }))
}

async fn fetch(state: &AppState, args: Value) -> Result<Value> {
    let raw = args["url"].as_str().ok_or_else(|| anyhow!("url is required"))?;
    let page = fetch_readable(&state.http_client, raw).await?;
    Ok(json!({
        "url": page.url,
        "title": page.title,
        "text": page.text,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_expose_both_tools() {
        let schemas = schemas();
        let names: Vec<&str> = schemas.iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["web_search", "fetch_page"]);
        assert!(handles("web_search") && handles("fetch_page"));
        assert!(!handles("web_fetch"));
    }

}
