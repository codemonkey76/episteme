//! Web tools — search via the self-hosted SearXNG sidecar, plus a readable-text
//! page fetcher so the model can follow up on results. Both are read-only.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::integrations::graph::{find_ci, html_to_text};
use crate::integrations::websearch::{map_results, searxng_url, ssrf_guard};
use crate::state::AppState;

const RESULTS_DEFAULT: usize = 8;
const RESULTS_MAX: usize = 15;
/// Stop reading a page after this many bytes (pre-extraction).
const PAGE_MAX_BYTES: usize = 2 * 1024 * 1024;
/// Char budget for the extracted text (~3k tokens).
const PAGE_TEXT_MAX: usize = 12_000;

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

/// Pull the <title> out of an HTML page, if present.
fn page_title(html: &str) -> Option<String> {
    let start = find_ci(html, "<title")?;
    let open_end = html[start..].find('>')? + start + 1;
    let close = find_ci(&html[open_end..], "</title>")? + open_end;
    let title = html[open_end..close].trim();
    (!title.is_empty()).then(|| title.to_string())
}

async fn fetch(state: &AppState, args: Value) -> Result<Value> {
    let raw = args["url"].as_str().ok_or_else(|| anyhow!("url is required"))?;
    let url = ssrf_guard(raw)?;

    let mut response = state
        .http_client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| anyhow!("fetch failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("fetch failed: {status}"));
    }

    // Read up to PAGE_MAX_BYTES, then stop — don't trust Content-Length alone.
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let remaining = PAGE_MAX_BYTES - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if bytes.len() >= PAGE_MAX_BYTES {
            break;
        }
    }
    let html = String::from_utf8_lossy(&bytes);

    let title = page_title(&html);
    let mut text = html_to_text(&html);
    if text.chars().count() > PAGE_TEXT_MAX {
        text = text.chars().take(PAGE_TEXT_MAX).collect();
        text.push_str("\n…[truncated]");
    }

    Ok(json!({
        "url": url.as_str(),
        "title": title,
        "text": text,
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

    #[test]
    fn page_title_extraction() {
        assert_eq!(
            page_title("<html><head><TITLE lang=\"en\"> Hello World </TITLE></head></html>"),
            Some("Hello World".to_string())
        );
        assert_eq!(page_title("<html><head></head></html>"), None);
        assert_eq!(page_title("<title></title>"), None);
    }
}
