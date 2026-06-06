//! Web search plumbing for the agent's `web_search` / `fetch_page` tools.
//! Search queries go to a self-hosted SearXNG sidecar (compose service
//! `searxng`); page fetches go straight to the target site through the shared
//! HTTP client, behind a basic SSRF guard.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::net::IpAddr;

/// Where the SearXNG sidecar lives. Env-only on purpose: it's compose
/// topology, not a user setting.
pub fn searxng_url() -> String {
    std::env::var("SEARXNG_URL").unwrap_or_else(|_| "http://searxng:8080".to_string())
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
