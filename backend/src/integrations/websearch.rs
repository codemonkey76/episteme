//! Web search plumbing for the agent's `web_search` / `fetch_page` tools.
//! Search queries go to a self-hosted SearXNG sidecar (compose service
//! `searxng`); page fetches go straight to the target site through the shared
//! HTTP client, behind a basic SSRF guard.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::net::IpAddr;

use crate::integrations::graph::{find_ci, html_to_text};

/// Stop reading a page after this many bytes (pre-extraction).
pub const PAGE_MAX_BYTES: usize = 2 * 1024 * 1024;
/// Char budget for the extracted text (~3k tokens).
pub const PAGE_TEXT_MAX: usize = 12_000;
/// Image candidates remembered per page.
const PAGE_IMAGES_MAX: usize = 3;

/// Where the SearXNG sidecar lives. Env-only on purpose: it's compose
/// topology, not a user setting.
pub fn searxng_url() -> String {
    std::env::var("SEARXNG_URL").unwrap_or_else(|_| "http://searxng:8080".to_string())
}

/// A fetched page reduced to what research/tools need: readable text plus a
/// few image candidates scraped from the raw HTML (html_to_text drops <img>).
pub struct FetchedPage {
    pub url: String,
    pub title: Option<String>,
    pub text: String,
    pub images: Vec<PageImage>,
}

pub struct PageImage {
    pub url: String,
    pub alt: Option<String>,
}

/// Fetch a public page (SSRF-guarded, byte-capped) and reduce it to readable
/// text + image candidates. Shared by the fetch_page tool and deep research.
pub async fn fetch_readable(client: &reqwest::Client, raw_url: &str) -> Result<FetchedPage> {
    let url = ssrf_guard(raw_url)?;

    let mut response = client
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
    let images = page_images(&html, &url);
    let mut text = html_to_text(&html);
    if text.chars().count() > PAGE_TEXT_MAX {
        text = text.chars().take(PAGE_TEXT_MAX).collect();
        text.push_str("\n…[truncated]");
    }

    Ok(FetchedPage { url: url.to_string(), title, text, images })
}

/// Pull the <title> out of an HTML page, if present.
pub fn page_title(html: &str) -> Option<String> {
    let start = find_ci(html, "<title")?;
    let open_end = html[start..].find('>')? + start + 1;
    let close = find_ci(&html[open_end..], "</title>")? + open_end;
    let title = html[open_end..close].trim();
    (!title.is_empty()).then(|| title.to_string())
}

/// Scan raw HTML for content images (src + alt), resolving relative URLs
/// against the page. Skips data URIs, tracker-style names, and SVGs.
fn page_images(html: &str, base: &url::Url) -> Vec<PageImage> {
    let mut out: Vec<PageImage> = Vec::new();
    let mut rest = html;
    while let Some(pos) = find_ci(rest, "<img") {
        let tag_start = &rest[pos..];
        let Some(end) = tag_start.find('>') else { break };
        let tag = &tag_start[..end];

        if let Some(src) = attr_value(tag, "src") {
            let resolved = base.join(&src).map(|u| u.to_string()).unwrap_or_default();
            let lower = resolved.to_lowercase();
            let skip = resolved.is_empty()
                || lower.starts_with("data:")
                || lower.ends_with(".svg")
                || lower.contains("pixel")
                || lower.contains("spacer")
                || lower.contains("1x1")
                || out.iter().any(|i| i.url == resolved);
            if !skip && ssrf_guard(&resolved).is_ok() {
                out.push(PageImage {
                    url: resolved,
                    alt: attr_value(tag, "alt").filter(|a| !a.trim().is_empty()),
                });
                if out.len() >= PAGE_IMAGES_MAX {
                    break;
                }
            }
        }
        rest = &rest[pos + end..];
    }
    out
}

/// Value of an HTML attribute within a tag string.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let at = find_ci(tag, &format!("{name}="))? + name.len() + 1;
    let rest = &tag[at..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        // Unquoted value: up to whitespace or tag end.
        let end = rest.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(rest.len());
        return Some(rest[..end].to_string());
    }
    let inner = &rest[1..];
    let end = inner.find(quote)?;
    Some(inner[..end].to_string())
}

/// Map a SearXNG `/search?format=json` body to compact, model-friendly results.
pub fn map_results(body: &Value, max: usize) -> Vec<Value> {
    body["results"]
        .as_array()
        .into_iter()
        .flatten()
        .take(max)
        .map(|r| {
            json!({
                "title": r["title"].as_str().unwrap_or_default(),
                "url": r["url"].as_str().unwrap_or_default(),
                "snippet": r["content"].as_str().unwrap_or_default(),
                "engine": r["engine"].as_str().unwrap_or_default(),
            })
        })
        .collect()
}

/// Best-effort SSRF guard for `fetch_page`: only http(s), no localhost or
/// single-label (docker-internal) hostnames, no private/loopback/link-local
/// IP literals. Hostname→IP resolution isn't checked — the real backstop is
/// that nothing credentialed listens inside the compose network.
pub fn ssrf_guard(raw: &str) -> Result<url::Url> {
    let parsed = url::Url::parse(raw).map_err(|e| anyhow!("invalid url: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(anyhow!("only http(s) urls can be fetched"));
    }
    match parsed.host() {
        None => return Err(anyhow!("url has no host")),
        Some(url::Host::Domain(d)) => {
            if d.eq_ignore_ascii_case("localhost") || !d.contains('.') {
                return Err(anyhow!("refusing to fetch internal host '{d}'"));
            }
        }
        Some(url::Host::Ipv4(ip)) => {
            if !ip_is_public(IpAddr::V4(ip)) {
                return Err(anyhow!("refusing to fetch private address {ip}"));
            }
        }
        Some(url::Host::Ipv6(ip)) => {
            if !ip_is_public(IpAddr::V6(ip)) {
                return Err(anyhow!("refusing to fetch private address {ip}"));
            }
        }
    }
    Ok(parsed)
}

fn ip_is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation())
        }
        IpAddr::V6(v6) => {
            let seg0 = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (seg0 & 0xfe00) == 0xfc00   // ULA fc00::/7
                || (seg0 & 0xffc0) == 0xfe80   // link-local fe80::/10
                || v6.to_ipv4_mapped().is_some_and(|v4| !ip_is_public(IpAddr::V4(v4))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searxng_url_defaults_to_compose_hostname() {
        // Tests run without the env var; the default must point at the sidecar.
        if std::env::var("SEARXNG_URL").is_err() {
            assert_eq!(searxng_url(), "http://searxng:8080");
        }
    }

    #[test]
    fn map_results_shapes_and_caps() {
        let body = json!({ "results": [
            { "title": "One", "url": "https://a.example", "content": "first", "engine": "duckduckgo" },
            { "title": "Two", "url": "https://b.example", "content": "second", "engine": "brave" },
            { "title": "Three", "url": "https://c.example", "content": "third", "engine": "google" },
        ]});
        let mapped = map_results(&body, 2);
        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[0]["title"], "One");
        assert_eq!(mapped[1]["snippet"], "second");
        assert_eq!(mapped[1]["engine"], "brave");
        assert!(map_results(&json!({}), 5).is_empty());
    }

    #[test]
    fn ssrf_guard_accepts_public_urls() {
        assert!(ssrf_guard("https://example.com/page").is_ok());
        assert!(ssrf_guard("http://93.184.216.34/").is_ok());
    }

    #[test]
    fn ssrf_guard_rejects_private_and_internal() {
        for bad in [
            "http://127.0.0.1/",
            "http://10.0.0.5/",
            "http://172.16.1.1/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://[fe80::1]/",
            "http://[::ffff:10.0.0.1]/",
            "http://localhost/",
            "http://searxng:8080/",
            "file:///etc/passwd",
            "ftp://example.com/",
            "not a url",
        ] {
            assert!(ssrf_guard(bad).is_err(), "should reject {bad}");
        }
    }
}

#[cfg(test)]
mod fetch_tests {
    use super::*;

    #[test]
    fn page_images_resolves_and_filters() {
        let base = url::Url::parse("https://blog.example.com/posts/nvr/").unwrap();
        let html = r#"<html><body>
            <img src="/assets/diagram.png" alt="System diagram">
            <img src='cam.jpg' alt=''>
            <img src="data:image/gif;base64,AAAA">
            <img src="https://tracker.example.com/1x1.gif">
            <img src="https://cdn.example.com/icon.svg" alt="icon">
            <img src="http://127.0.0.1/internal.png">
            <img src="/assets/diagram.png" alt="dupe">
        </body></html>"#;
        let images = page_images(html, &base);
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].url, "https://blog.example.com/assets/diagram.png");
        assert_eq!(images[0].alt.as_deref(), Some("System diagram"));
        // Empty alt filtered to None; relative resolves against the page dir.
        assert_eq!(images[1].url, "https://blog.example.com/posts/nvr/cam.jpg");
        assert!(images[1].alt.is_none());
    }

    #[test]
    fn attr_value_handles_quote_styles() {
        assert_eq!(attr_value(r#"<img src="a.png" alt='b c'"#, "alt").as_deref(), Some("b c"));
        assert_eq!(attr_value("<img src=bare.png>", "src").as_deref(), Some("bare.png"));
        assert!(attr_value("<img alt=x>", "src").is_none());
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
