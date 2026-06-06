//! Deep research orchestrator: plan → gather (web + internal corpus) →
//! reflect → synthesize → render. Runs as a `kind = "research"` job (Phase-8
//! substrate). The scratchpad pattern keeps context bounded: every fetched
//! page is distilled into citation-tagged notes immediately and the raw text
//! is dropped — only the memo crosses stage boundaries.

pub mod render;

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use base64::Engine;
use serde_json::Value;

use crate::db::{self, jobs::Job};
use crate::integrations::embeddings;
use crate::integrations::graph::{graph_get, html_to_text, GRAPH};
use crate::integrations::websearch::{fetch_readable, map_results, searxng_url};
use crate::model_router::{ModelRouter, ProviderConfig};
use crate::state::AppState;
use render::{EmbeddedImage, ReportDoc, Source};

/// Per-depth budgets: web page fetches and reflect rounds.
fn budgets(depth: &str) -> (usize, usize) {
    match depth {
        "quick" => (6, 0),
        "deep" => (20, 2),
        _ => (12, 1), // standard
    }
}

const MAX_PLAN_QUERIES: usize = 6;
const URLS_PER_QUERY: usize = 3;
/// Memo char budget — once full, no further web notes are accepted.
const MEMO_MAX_CHARS: usize = 24_000;
/// Distill input cap per source.
const DISTILL_TEXT_MAX: usize = 10_000;
/// Image embedding caps.
const MAX_REPORT_IMAGES: usize = 4;
const IMAGE_MAX_BYTES: usize = 1_536 * 1024;

struct Note {
    source_id: String,
    finding: String,
    quote: Option<String>,
}

struct ImageCandidate {
    /// Short id (I1, I2…) the models reference — never a retyped URL.
    id: String,
    url: String,
    /// Page the image was found on; sent as Referer (hotlink protection).
    page_url: String,
    caption: String,
}

/// Validate inputs, create the 🔎 session + research job, and enqueue it.
/// Shared by the `deep_research` chat tool and the Reports-window launcher.
/// Returns (job_id, session_id).
pub async fn launch(
    state: &Arc<AppState>,
    user_id: &str,
    topic: &str,
    depth: &str,
    provider_arg: &str,
) -> Result<(String, String)> {
    let topic = topic.trim();
    if topic.is_empty() {
        return Err(anyhow!("topic is required"));
    }
    let depth = match depth {
        d @ ("quick" | "deep") => d,
        _ => "standard",
    };

    let providers: Vec<ProviderConfig> =
        db::settings::get(&state.db, "providers").await?.unwrap_or_default();
    if providers.is_empty() {
        return Err(anyhow!("no model providers configured"));
    }
    if !provider_arg.is_empty() && !providers.iter().any(|p| p.name == provider_arg) {
        return Err(anyhow!("provider '{provider_arg}' not found"));
    }

    let session = db::sessions::create(&state.db, user_id, &format!("🔎 {topic}")).await?;
    db::messages::insert(
        &state.db,
        &session.id,
        "user",
        &serde_json::to_string(topic).unwrap_or_default(),
        None,
        None,
    )
    .await?;

    let name_clipped: String = topic.chars().take(60).collect();
    let meta = serde_json::json!({ "topic": topic, "depth": depth }).to_string();
    let job = crate::jobs::start(
        state,
        user_id,
        &session.id,
        provider_arg,
        "research",
        &format!("Research: {name_clipped}"),
        Some(&meta),
    )
    .await?;
    let job_id = job.id.clone();
    state.job_tx.send(job).map_err(|_| anyhow!("job queue unavailable"))?;
    Ok((job_id, session.id))
}

/// Execute one research job end-to-end. The session collects progress
/// messages; the report lands in the reports table; the final assistant
/// message carries the outcome (drives the job summary + push).
pub async fn run(state: &Arc<AppState>, job: &Job, provider: ProviderConfig) -> Result<()> {
    let topic = first_user_message(state, &job.session_id)
        .await
        .ok_or_else(|| anyhow!("research session has no topic"))?;
    let depth = job
        .meta
        .as_deref()
        .and_then(|m| serde_json::from_str::<Value>(m).ok())
        .and_then(|v| v["depth"].as_str().map(String::from))
        .unwrap_or_else(|| "standard".to_string());
    let (fetch_budget, reflect_rounds) = budgets(&depth);

    let mut memo: Vec<Note> = Vec::new();
    let mut memo_chars = 0usize;
    let mut sources: Vec<Source> = Vec::new();
    let mut image_candidates: Vec<ImageCandidate> = Vec::new();
    let mut fetched_urls: HashSet<String> = HashSet::new();
    let mut all_queries: Vec<String> = Vec::new();
    let mut fetches_left = fetch_budget;

    // ── PLAN ────────────────────────────────────────────────────────────────
    progress(state, &job.session_id, "Planning the investigation…").await;
    let plan_system = prompt(state, "research_plan", &topic).await;
    let mut queries: Vec<String> = match complete_json(state, &job.user_id, &provider, &plan_system, &topic).await
    {
        Ok(v) => v["queries"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|q| q.as_str().map(String::from))
            .take(MAX_PLAN_QUERIES)
            .collect(),
        Err(e) => {
            tracing::warn!("research plan failed ({e}); degrading to the topic as one query");
            Vec::new()
        }
    };
    if queries.is_empty() {
        queries.push(topic.clone());
    }

    // ── GATHER (web) + REFLECT rounds ──────────────────────────────────────
    let distill_system = prompt(state, "research_distill", &topic).await;
    let mut round = 0usize;
    loop {
        all_queries.extend(queries.iter().cloned());
        gather_web(
            state,
            job,
            &provider,
            &distill_system,
            &queries,
            &mut fetches_left,
            &mut fetched_urls,
            &mut memo,
            &mut memo_chars,
            &mut sources,
            &mut image_candidates,
        )
        .await;

        if round >= reflect_rounds || fetches_left == 0 || memo_chars >= MEMO_MAX_CHARS {
            break;
        }
        round += 1;

        let reflect_system = prompt(state, "research_reflect", &topic).await;
        let memo_text = memo_as_text(&memo);
        let user = format!(
            "Earlier queries: {}\n\nCollected notes:\n{}",
            all_queries.join("; "),
            memo_text
        );
        match complete_json(state, &job.user_id, &provider, &reflect_system, &user).await {
            Ok(v) if !v["done"].as_bool().unwrap_or(true) => {
                queries = v["queries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|q| q.as_str().map(String::from))
                    .filter(|q| !all_queries.contains(q))
                    .take(3)
                    .collect();
                if queries.is_empty() {
                    break;
                }
                progress(state, &job.session_id, &format!("Digging deeper: {}", queries.join(" · ")))
                    .await;
            }
            _ => break,
        }
    }

    // ── INTERNAL corpus ────────────────────────────────────────────────────
    progress(state, &job.session_id, "Checking your documents, email, memories, and chats…").await;
    gather_internal(state, job, &provider, &distill_system, &topic, &mut memo, &mut memo_chars, &mut sources)
        .await;

    if memo.is_empty() {
        return Err(anyhow!(
            "no usable sources found — web search may be down and nothing internal matched"
        ));
    }

    // ── SYNTHESIZE ─────────────────────────────────────────────────────────
    progress(state, &job.session_id, "Writing the report…").await;
    let synth_system = prompt(state, "research_synthesize", &topic).await;
    let synth_user = format!(
        "Notes (each tagged with its source id):\n{}\n\nSources:\n{}\n\nCandidate images:\n{}",
        memo_as_text(&memo),
        sources
            .iter()
            .map(|s| format!("{} = {}", s.id, s.label))
            .collect::<Vec<_>>()
            .join("\n"),
        if image_candidates.is_empty() {
            "(none)".to_string()
        } else {
            image_candidates
                .iter()
                .map(|i| format!("{} — {} ({})", i.id, i.caption, i.url))
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    let doc: ReportDoc = match complete_json(state, &job.user_id, &provider, &synth_system, &synth_user).await {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(e) => {
            tracing::warn!("research synthesis failed ({e}); using fallback report");
            ReportDoc::default()
        }
    };
    let doc = if doc.sections.is_empty() { fallback_report(&topic, &memo, &sources) } else { doc };

    // ── IMAGES ─────────────────────────────────────────────────────────────
    let images = embed_images(state, &doc, &image_candidates).await;

    // ── RENDER + PERSIST ───────────────────────────────────────────────────
    let tz = state.home_tz(&job.user_id).await;
    let generated = chrono::Utc::now().with_timezone(&tz).format("%-d %B %Y").to_string();
    let html = render::render_report(&doc, &sources, &images, &generated);
    let title = if doc.title.trim().is_empty() { topic.clone() } else { doc.title.clone() };
    db::reports::insert(&state.db, &job.user_id, Some(&job.id), &title, &html).await?;

    state
        .log("research", "info", format!("report ready: {title} ({} sources)", sources.len()))
        .await;
    // Final assistant message LAST — it becomes the job summary and push body.
    progress(
        state,
        &job.session_id,
        &format!("Report ready: \"{title}\" — {} sources. Open it in the Reports window.", sources.len()),
    )
    .await;
    Ok(())
}

// ── Stages ──────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn gather_web(
    state: &Arc<AppState>,
    job: &Job,
    provider: &ProviderConfig,
    distill_system: &str,
    queries: &[String],
    fetches_left: &mut usize,
    fetched_urls: &mut HashSet<String>,
    memo: &mut Vec<Note>,
    memo_chars: &mut usize,
    sources: &mut Vec<Source>,
    image_candidates: &mut Vec<ImageCandidate>,
) {
    for query in queries {
        if *fetches_left == 0 || *memo_chars >= MEMO_MAX_CHARS {
            return;
        }
        let results = match search(state, query).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("research search '{query}' failed: {e}");
                progress(state, &job.session_id, "Web search unavailable — continuing with other sources.")
                    .await;
                return; // SearXNG down: skip the whole web leg this round
            }
        };
        progress(state, &job.session_id, &format!("Searched \"{query}\" — {} results", results.len()))
            .await;

        let urls: Vec<String> = results
            .iter()
            .filter_map(|r| r["url"].as_str().map(String::from))
            .filter(|u| !fetched_urls.contains(u))
            .take(URLS_PER_QUERY)
            .collect();
        for url in urls {
            if *fetches_left == 0 || *memo_chars >= MEMO_MAX_CHARS {
                return;
            }
            *fetches_left -= 1;
            fetched_urls.insert(url.clone());

            let page = match fetch_readable(&state.http_client, &url).await {
                Ok(p) => p,
                Err(e) => {
                    tracing::info!("research fetch {url} failed: {e}");
                    continue;
                }
            };
            let label = page.title.clone().unwrap_or_else(|| url.clone());
            let source_id = format!("S{}", sources.len() + 1);

            let candidates_text = page
                .images
                .iter()
                .enumerate()
                .map(|(n, i)| format!("{}. {} (alt: {})", n + 1, i.url, i.alt.as_deref().unwrap_or("-")))
                .collect::<Vec<_>>()
                .join("\n");
            let text: String = page.text.chars().take(DISTILL_TEXT_MAX).collect();
            let user = format!(
                "Source: {label} ({url})\n\nImage candidates:\n{}\n\nPage text:\n{text}",
                if candidates_text.is_empty() { "(none)" } else { &candidates_text },
            );
            let Ok(v) = complete_json(state, &job.user_id, provider, distill_system, &user).await else {
                continue;
            };
            if !v["relevant"].as_bool().unwrap_or(false) {
                continue;
            }

            let mut used = false;
            for note in v["notes"].as_array().into_iter().flatten() {
                let Some(finding) = note["finding"].as_str().filter(|f| !f.trim().is_empty()) else {
                    continue;
                };
                let entry = Note {
                    source_id: source_id.clone(),
                    finding: finding.to_string(),
                    quote: note["quote"].as_str().filter(|q| !q.trim().is_empty()).map(String::from),
                };
                let cost = entry.finding.len() + entry.quote.as_deref().map_or(0, str::len);
                if *memo_chars + cost > MEMO_MAX_CHARS {
                    break;
                }
                *memo_chars += cost;
                memo.push(entry);
                used = true;
            }
            for img in v["images"].as_array().into_iter().flatten() {
                // Accept the candidate number (preferred — no URL retyping) or
                // an exact URL; either way only images actually on the page.
                let picked = img["n"]
                    .as_u64()
                    .filter(|n| *n >= 1)
                    .and_then(|n| page.images.get(n as usize - 1))
                    .or_else(|| {
                        img["url"].as_str().and_then(|u| page.images.iter().find(|p| p.url == u))
                    });
                if let Some(p) = picked {
                    if !image_candidates.iter().any(|c| c.url == p.url) {
                        image_candidates.push(ImageCandidate {
                            id: format!("I{}", image_candidates.len() + 1),
                            url: p.url.clone(),
                            page_url: page.url.clone(),
                            caption: img["caption"]
                                .as_str()
                                .filter(|c| !c.trim().is_empty())
                                .or(p.alt.as_deref())
                                .unwrap_or("")
                                .to_string(),
                        });
                    }
                }
            }
            if used {
                sources.push(Source { id: source_id, label, url: Some(page.url) });
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn gather_internal(
    state: &Arc<AppState>,
    job: &Job,
    provider: &ProviderConfig,
    distill_system: &str,
    topic: &str,
    memo: &mut Vec<Note>,
    memo_chars: &mut usize,
    sources: &mut Vec<Source>,
) {
    let user_id = &job.user_id;
    let mut batches: Vec<(String, String, String)> = Vec::new(); // (id, label, text)

    // Documents: cosine top chunks (skip silently when embeddings unavailable).
    if let Ok(qvec) = embeddings::embed(&state.db, &state.http_client, topic).await {
        if let Ok(chunks) = db::documents::search_pool(&state.db, user_id, 20_000).await {
            let mut scored: Vec<(&db::documents::SearchChunk, f32)> = chunks
                .iter()
                .filter_map(|c| {
                    let blob = c.embedding.as_deref()?;
                    Some((c, embeddings::cosine(&qvec, &embeddings::from_blob(blob))))
                })
                .filter(|(_, s)| *s >= 0.3)
                .collect();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (chunk, _) in scored.iter().take(5) {
                batches.push((
                    format!("doc:{}", chunk.filename),
                    chunk.filename.clone(),
                    chunk.content.clone(),
                ));
            }
        }
        if let Ok(memories) = db::memories::list(&state.db, user_id, None, None, 2000).await {
            let mut scored: Vec<(&db::memories::Memory, f32)> = memories
                .iter()
                .filter_map(|m| {
                    let blob = m.embedding.as_deref()?;
                    Some((m, embeddings::cosine(&qvec, &embeddings::from_blob(blob))))
                })
                .filter(|(_, s)| *s >= 0.35)
                .collect();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (m, _) in scored.iter().take(5) {
                batches.push((
                    format!("memory:{}", m.category),
                    format!("memory ({})", m.category),
                    m.content.clone(),
                ));
            }
        }
    }

    // Chat history (FTS).
    if let Ok(hits) = db::messages::search(&state.db, user_id, topic, 5).await {
        for h in hits {
            batches.push((
                format!("chat:{}", h.session_title),
                format!("chat: {}", h.session_title),
                h.snippet,
            ));
        }
    }

    // Email ($search; read the top 2 bodies). Soft-fail when M365 unconnected.
    if let Ok(body) = graph_get(
        state,
        user_id,
        &format!("{GRAPH}/me/messages"),
        &[
            ("$search", &format!("\"{}\"", topic.replace('"', "")) as &str),
            ("$select", "id,subject,from,bodyPreview"),
            ("$top", "5"),
        ],
    )
    .await
    {
        let messages: Vec<&Value> = body["value"].as_array().into_iter().flatten().collect();
        for (i, m) in messages.iter().enumerate() {
            let subject = m["subject"].as_str().unwrap_or("(no subject)").to_string();
            let text = if i < 2 {
                // Full body for the top hits.
                if let Some(id) = m["id"].as_str() {
                    match graph_get(
                        state,
                        user_id,
                        &format!("{GRAPH}/me/messages/{id}"),
                        &[("$select", "body")],
                    )
                    .await
                    {
                        Ok(full) => {
                            let raw = full["body"]["content"].as_str().unwrap_or("");
                            if full["body"]["contentType"].as_str() == Some("html") {
                                html_to_text(raw)
                            } else {
                                raw.to_string()
                            }
                        }
                        Err(_) => m["bodyPreview"].as_str().unwrap_or("").to_string(),
                    }
                } else {
                    continue;
                }
            } else {
                m["bodyPreview"].as_str().unwrap_or("").to_string()
            };
            batches.push((format!("email:{subject}"), format!("email: {subject}"), text));
        }
    }

    // Distill each internal batch like a web page.
    for (id, label, text) in batches {
        if *memo_chars >= MEMO_MAX_CHARS || text.trim().is_empty() {
            break;
        }
        if sources.iter().any(|s| s.id == id) {
            continue;
        }
        let clipped: String = text.chars().take(DISTILL_TEXT_MAX).collect();
        let user = format!("Source: {label}\n\nImage candidates:\n(none)\n\nPage text:\n{clipped}");
        let Ok(v) = complete_json(state, &job.user_id, provider, distill_system, &user).await else { continue };
        if !v["relevant"].as_bool().unwrap_or(false) {
            continue;
        }
        let mut used = false;
        for note in v["notes"].as_array().into_iter().flatten() {
            let Some(finding) = note["finding"].as_str().filter(|f| !f.trim().is_empty()) else {
                continue;
            };
            let cost = finding.len();
            if *memo_chars + cost > MEMO_MAX_CHARS {
                break;
            }
            *memo_chars += cost;
            memo.push(Note {
                source_id: id.clone(),
                finding: finding.to_string(),
                quote: note["quote"].as_str().filter(|q| !q.trim().is_empty()).map(String::from),
            });
            used = true;
        }
        if used {
            sources.push(Source { id, label, url: None });
        }
    }
}

/// Fetch the model-picked images and embed them as data URIs (caps applied).
/// Synthesis picks by id (or legacy exact URL); if it picked none, fall back
/// to the distill-approved candidates so usable images still ship.
async fn embed_images(
    state: &Arc<AppState>,
    doc: &ReportDoc,
    candidates: &[ImageCandidate],
) -> Vec<EmbeddedImage> {
    // Only candidates from real page scans are fetchable, never invented URLs.
    let mut picks: Vec<(&ImageCandidate, String)> = doc
        .images
        .iter()
        .filter_map(|pick| {
            let cand = candidates.iter().find(|c| {
                c.id == pick.id.trim() || (!pick.source_url.is_empty() && c.url == pick.source_url)
            })?;
            let caption =
                if pick.caption.trim().is_empty() { cand.caption.clone() } else { pick.caption.clone() };
            Some((cand, caption))
        })
        .collect();
    if picks.is_empty() {
        picks = candidates.iter().map(|c| (c, c.caption.clone())).collect();
    }

    let mut out = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for (cand, caption) in picks {
        if out.len() >= MAX_REPORT_IMAGES || !seen.insert(&cand.url) {
            continue;
        }
        let url = match crate::integrations::websearch::ssrf_guard(&cand.url) {
            Ok(u) => u,
            Err(e) => {
                tracing::info!("research image {} skipped: {e}", cand.url);
                continue;
            }
        };
        // Referer matters: product CDNs hotlink-protect, and we *are* loading
        // this image for that page's content.
        let response = match state
            .http_client
            .get(url)
            .header(reqwest::header::REFERER, cand.page_url.clone())
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::info!("research image {} skipped: fetch failed: {e}", cand.url);
                continue;
            }
        };
        if !response.status().is_success() {
            tracing::info!("research image {} skipped: HTTP {}", cand.url, response.status());
            continue;
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .to_string();
        if !mime.starts_with("image/") || mime == "image/svg+xml" {
            tracing::info!("research image {} skipped: content-type '{mime}'", cand.url);
            continue;
        }
        let Ok(bytes) = response.bytes().await else { continue };
        if bytes.is_empty() || bytes.len() > IMAGE_MAX_BYTES {
            tracing::info!("research image {} skipped: {} bytes", cand.url, bytes.len());
            continue;
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        out.push(EmbeddedImage { caption, data_uri: format!("data:{mime};base64,{b64}") });
    }
    out
}

/// When synthesis fails entirely: a plain but complete report from the memo.
fn fallback_report(topic: &str, memo: &[Note], sources: &[Source]) -> ReportDoc {
    use render::{Paragraph, Section};
    let mut by_source: Vec<Section> = Vec::new();
    for src in sources {
        let paragraphs: Vec<Paragraph> = memo
            .iter()
            .filter(|n| n.source_id == src.id)
            .map(|n| Paragraph { text: n.finding.clone(), cites: vec![n.source_id.clone()] })
            .collect();
        if !paragraphs.is_empty() {
            by_source.push(Section { heading: src.label.clone(), paragraphs });
        }
    }
    ReportDoc {
        title: format!("Research notes: {topic}"),
        intro: "The synthesis step could not produce a structured report; these are the \
                collected findings, grouped by source."
            .to_string(),
        sections: by_source,
        ..Default::default()
    }
}

// ── Plumbing ────────────────────────────────────────────────────────────────

async fn search(state: &AppState, query: &str) -> Result<Vec<Value>> {
    let response = state
        .http_client
        .get(format!("{}/search", searxng_url()))
        .query(&[("q", query), ("format", "json")])
        .send()
        .await
        .map_err(|e| anyhow!("web search unavailable: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("SearXNG returned {status}"));
    }
    let body: Value = response.json().await.map_err(|e| anyhow!("invalid SearXNG JSON: {e}"))?;
    Ok(map_results(&body, 8))
}

async fn prompt(state: &AppState, key: &str, topic: &str) -> String {
    crate::prompts::get(&state.db, key).await.replace("{topic}", topic)
}

/// One-shot model call returning tolerantly parsed JSON; retries once with a
/// stern reminder. Usage recorded under purpose "research".
async fn complete_json(
    state: &AppState,
    user_id: &str,
    provider: &ProviderConfig,
    system: &str,
    user: &str,
) -> Result<Value> {
    use crate::model_router::ChatMessage;
    let history = |extra: &str| {
        vec![
            ChatMessage { role: "system".into(), content: Value::String(format!("{system}{extra}")) },
            ChatMessage { role: "user".into(), content: Value::String(user.to_string()) },
        ]
    };
    let (raw, used) = ModelRouter::complete_with_usage(provider, history("")).await?;
    db::usage::record(&state.db, user_id, provider, "research", used).await;
    if let Some(v) = json_slice(&raw) {
        return Ok(v);
    }
    let (raw, used) =
        ModelRouter::complete_with_usage(provider, history("\nReturn ONLY valid JSON, no prose."))
            .await?;
    db::usage::record(&state.db, user_id, provider, "research", used).await;
    json_slice(&raw).ok_or_else(|| anyhow!("model did not return parseable JSON"))
}

/// Extract the outermost JSON value from model output, tolerating code fences
/// and surrounding prose (mirrors memory::parse).
fn json_slice(raw: &str) -> Option<Value> {
    let obj = raw.find('{').and_then(|start| {
        let end = raw.rfind('}')?;
        (end > start).then(|| serde_json::from_str::<Value>(&raw[start..=end]).ok())?
    });
    if obj.is_some() {
        return obj;
    }
    let start = raw.find('[')?;
    let end = raw.rfind(']')?;
    (end > start).then(|| serde_json::from_str::<Value>(&raw[start..=end]).ok())?
}

async fn first_user_message(state: &AppState, session_id: &str) -> Option<String> {
    db::messages::list_for_session(&state.db, session_id)
        .await
        .ok()?
        .into_iter()
        .find(|m| m.role == "user")
        .map(|m| serde_json::from_str::<String>(&m.content).unwrap_or(m.content))
}

/// Progress note into the research session (visible live when opened).
async fn progress(state: &AppState, session_id: &str, text: &str) {
    let _ = db::messages::insert(
        &state.db,
        session_id,
        "assistant",
        &serde_json::to_string(text).unwrap_or_default(),
        None,
        None,
    )
    .await;
}

fn memo_as_text(memo: &[Note]) -> String {
    memo.iter()
        .map(|n| match &n.quote {
            Some(q) => format!("[{}] {} (\"{}\")", n.source_id, n.finding, q),
            None => format!("[{}] {}", n.source_id, n.finding),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_slice_tolerates_fences_and_prose() {
        let raw = "Sure!\n```json\n{\"queries\": [\"a\"]}\n```\nHope that helps.";
        assert_eq!(json_slice(raw).unwrap()["queries"][0], "a");
        assert!(json_slice("no json here").is_none());
        assert_eq!(json_slice("[1, 2]").unwrap()[1], 2);
        // Outermost braces win even with nested objects.
        let nested = "x {\"a\": {\"b\": 1}} y";
        assert_eq!(json_slice(nested).unwrap()["a"]["b"], 1);
    }

    #[test]
    fn budgets_by_depth() {
        assert_eq!(budgets("quick"), (6, 0));
        assert_eq!(budgets("standard"), (12, 1));
        assert_eq!(budgets("deep"), (20, 2));
        assert_eq!(budgets("nonsense"), (12, 1));
    }

    #[test]
    fn fallback_report_groups_by_source() {
        let memo = vec![
            Note { source_id: "S1".into(), finding: "f1".into(), quote: None },
            Note { source_id: "S2".into(), finding: "f2".into(), quote: None },
        ];
        let sources = vec![
            Source { id: "S1".into(), label: "A".into(), url: None },
            Source { id: "S2".into(), label: "B".into(), url: None },
        ];
        let doc = fallback_report("topic", &memo, &sources);
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.sections[0].heading, "A");
        assert_eq!(doc.sections[0].paragraphs[0].cites, vec!["S1"]);
    }

    #[test]
    fn memo_text_includes_source_tags_and_quotes() {
        let memo = vec![Note {
            source_id: "doc:x.pdf".into(),
            finding: "the fee is $50".into(),
            quote: Some("a fee of fifty dollars".into()),
        }];
        let text = memo_as_text(&memo);
        assert_eq!(text, "[doc:x.pdf] the fee is $50 (\"a fee of fifty dollars\")");
    }
}
