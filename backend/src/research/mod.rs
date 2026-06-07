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
const MAX_SUBQUESTIONS: usize = 8;
const URLS_PER_QUERY: usize = 3;
/// Memo char budget — at the cap, the memo is compacted (duplicates merged)
/// rather than going deaf; only when compactions run out are notes refused.
const MEMO_MAX_CHARS: usize = 24_000;
/// A compaction must shrink the memo below this to be adopted.
const MEMO_COMPACT_TARGET: usize = 19_000;
/// Compaction calls per run — bounds the extra model cost.
const MEMO_COMPACTIONS: usize = 2;
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

/// One pooled SERP result awaiting triage.
struct SerpCandidate {
    /// Which plan query surfaced it — drives the round-robin fallback order.
    query_idx: usize,
    title: String,
    url: String,
    snippet: String,
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
    // Failed fetches (404s, paywalls, timeouts) refund their budget slot;
    // this attempt cap bounds the total HTTP work so refunds can't run away.
    let mut attempts_left = fetch_budget * 2;

    // ── PLAN ────────────────────────────────────────────────────────────────
    progress(state, &job.session_id, "Planning the investigation…").await;
    let plan_system = prompt(state, "research_plan", &topic).await;
    let (mut queries, subquestions) =
        match complete_json(state, &job.user_id, &provider, &plan_system, &topic).await {
            Ok(v) => parse_plan(&v),
            Err(e) => {
                tracing::warn!("research plan failed ({e}); degrading to the topic as one query");
                (Vec::new(), Vec::new())
            }
        };
    if queries.is_empty() {
        queries.push(topic.clone());
    }
    let focus = focus_block(&subquestions);

    // ── GATHER (web) + REFLECT rounds ──────────────────────────────────────
    let distill_system = prompt(state, "research_distill", &topic).await;
    let triage_system = prompt(state, "research_triage", &topic).await;
    let mut compactions_left = MEMO_COMPACTIONS;
    let mut round = 0usize;
    loop {
        all_queries.extend(queries.iter().cloned());
        gather_web(
            state,
            job,
            &provider,
            &distill_system,
            &triage_system,
            &focus,
            &queries,
            &mut fetches_left,
            &mut attempts_left,
            &mut fetched_urls,
            &mut memo,
            &mut memo_chars,
            &mut sources,
            &mut image_candidates,
        )
        .await;

        // A full memo re-opens via compaction (merge duplicates) at stage
        // boundaries, so further rounds and the internal pass aren't refused.
        if memo_chars >= MEMO_MAX_CHARS && compactions_left > 0 {
            compactions_left -= 1;
            progress(state, &job.session_id, "Consolidating notes…").await;
            compact_memo(state, job, &provider, &topic, &mut memo, &mut memo_chars, &sources)
                .await;
        }

        if round >= reflect_rounds
            || fetches_left == 0
            || attempts_left == 0
            || memo_chars >= MEMO_MAX_CHARS
        {
            break;
        }
        round += 1;

        let reflect_system = prompt(state, "research_reflect", &topic).await;
        let memo_text = memo_as_text(&memo);
        let user = format!(
            "Earlier queries: {}{}\n\nCollected notes:\n{}",
            all_queries.join("; "),
            focus,
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
    // Same re-open before the internal pass: the user's own data shouldn't be
    // the leg that gets refused because the web filled the memo.
    if memo_chars >= MEMO_MAX_CHARS && compactions_left > 0 {
        progress(state, &job.session_id, "Consolidating notes…").await;
        compact_memo(state, job, &provider, &topic, &mut memo, &mut memo_chars, &sources).await;
    }
    progress(state, &job.session_id, "Checking your documents, email, memories, and chats…").await;
    gather_internal(
        state,
        job,
        &provider,
        &distill_system,
        &focus,
        &topic,
        &all_queries,
        &mut memo,
        &mut memo_chars,
        &mut sources,
    )
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
        "{}Notes (each tagged with its source id):\n{}\n\nSources:\n{}\n\nCandidate images:\n{}",
        // The report should answer what the plan set out to answer.
        if focus.is_empty() { String::new() } else { format!("{}\n\n", focus.trim_start()) },
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

    // Feed the report back into the documents corpus (markdown, not the HTML
    // with its data-URI images), so later chats — and the next research run's
    // internal pass — can retrieve it by meaning. Detached, like an upload;
    // failures mark the document row, never the finished report.
    let markdown = render::render_markdown(&doc, &sources);
    let doc_name = format!("Research report: {title}.md");
    match db::documents::insert(
        &state.db,
        &job.user_id,
        &doc_name,
        "text/markdown",
        markdown.len() as i64,
    )
    .await
    {
        Ok(doc_row) => {
            let st = Arc::clone(state);
            let uid = job.user_id.clone();
            tokio::spawn(async move {
                crate::documents::index(
                    &st,
                    &uid,
                    &doc_row.id,
                    &doc_row.filename,
                    "text/markdown",
                    markdown.into_bytes(),
                )
                .await;
            });
        }
        Err(e) => tracing::warn!("report→RAG ingestion skipped: {e}"),
    }

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
    triage_system: &str,
    focus: &str,
    queries: &[String],
    fetches_left: &mut usize,
    attempts_left: &mut usize,
    fetched_urls: &mut HashSet<String>,
    memo: &mut Vec<Note>,
    memo_chars: &mut usize,
    sources: &mut Vec<Source>,
    image_candidates: &mut Vec<ImageCandidate>,
) {
    // Pool every query's results, then triage once: one cheap call decides
    // which pages deserve the fetch budget, instead of blindly reading each
    // query's top-3 (which spends slots on SEO filler while the substantive
    // result at #5 is skipped).
    let mut candidates: Vec<SerpCandidate> = Vec::new();
    for (qi, query) in queries.iter().enumerate() {
        if *fetches_left == 0 || *attempts_left == 0 || *memo_chars >= MEMO_MAX_CHARS {
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

        for r in &results {
            let Some(url) = r["url"].as_str().filter(|u| !u.is_empty()) else { continue };
            if fetched_urls.contains(url) || candidates.iter().any(|c| c.url == url) {
                continue;
            }
            candidates.push(SerpCandidate {
                query_idx: qi,
                title: r["title"].as_str().unwrap_or("").to_string(),
                url: url.to_string(),
                snippet: r["snippet"].as_str().unwrap_or("").chars().take(200).collect(),
            });
        }
    }
    if candidates.is_empty() {
        return;
    }

    let want = (*fetches_left).min(queries.len() * URLS_PER_QUERY);
    let order = triage(state, job, provider, triage_system, focus, &candidates, want).await;

    for idx in order {
        if *fetches_left == 0 || *attempts_left == 0 || *memo_chars >= MEMO_MAX_CHARS {
            return;
        }
        let url = candidates[idx].url.clone();
        *attempts_left -= 1;
        fetched_urls.insert(url.clone());

        let page = match fetch_readable(&state.http_client, &url).await {
            Ok(p) => p,
            Err(e) => {
                // Dead page: the budget slot is refunded (only the attempt
                // cap was consumed), so a flaky web doesn't shrink the run.
                tracing::info!("research fetch {url} failed: {e}");
                continue;
            }
        };
        *fetches_left -= 1;
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
            "Source: {label} ({url}){focus}\n\nImage candidates:\n{}\n\nPage text:\n{text}",
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

#[allow(clippy::too_many_arguments)]
async fn gather_internal(
    state: &Arc<AppState>,
    job: &Job,
    provider: &ProviderConfig,
    distill_system: &str,
    focus: &str,
    topic: &str,
    queries: &[String],
    memo: &mut Vec<Note>,
    memo_chars: &mut usize,
    sources: &mut Vec<Source>,
) {
    let user_id = &job.user_id;
    let mut batches: Vec<(String, String, String)> = Vec::new(); // (id, label, text)

    // Keyword legs (chat FTS, email $search) AND every term together, so the
    // whole multi-word topic almost never matches — probe with the plan's
    // focused queries first, the topic as a last try.
    let keyword_probes: Vec<&str> = queries
        .iter()
        .map(String::as_str)
        .take(3)
        .chain(std::iter::once(topic))
        .collect();

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

    // Chat history (FTS), merged across probes.
    let mut chat_seen: HashSet<String> = HashSet::new();
    for probe in &keyword_probes {
        if chat_seen.len() >= 5 {
            break;
        }
        if let Ok(hits) = db::messages::search(&state.db, user_id, probe, 3).await {
            for h in hits {
                if chat_seen.len() < 5 && chat_seen.insert(h.message_id) {
                    batches.push((
                        format!("chat:{}", h.session_title),
                        format!("chat: {}", h.session_title),
                        h.snippet,
                    ));
                }
            }
        }
    }

    // Email: keyword $search per probe plus the semantic index (meaning
    // matches keyword search can't make). Soft-fail when M365 unconnected.
    let mut email_hits: Vec<(String, String, String)> = Vec::new(); // (id, subject, preview)
    let mut email_seen: HashSet<String> = HashSet::new();
    for probe in &keyword_probes {
        if email_hits.len() >= 6 {
            break;
        }
        let Ok(body) = graph_get(
            state,
            user_id,
            &format!("{GRAPH}/me/messages"),
            &[
                ("$search", &format!("\"{}\"", probe.replace('"', "")) as &str),
                ("$select", "id,subject,from,bodyPreview"),
                ("$top", "3"),
            ],
        )
        .await
        else {
            break; // unconnected/unauthorized: further probes won't fare better
        };
        for m in body["value"].as_array().into_iter().flatten() {
            let Some(id) = m["id"].as_str() else { continue };
            if email_seen.insert(id.to_string()) {
                email_hits.push((
                    id.to_string(),
                    m["subject"].as_str().unwrap_or("(no subject)").to_string(),
                    m["bodyPreview"].as_str().unwrap_or("").to_string(),
                ));
            }
        }
    }
    if let Ok(hits) = crate::email_index::search(state, user_id, "", topic, 5).await {
        for h in hits {
            if email_seen.insert(h.message_id.clone()) {
                email_hits.push((h.message_id, h.subject, h.snippet));
            }
        }
    }
    email_hits.truncate(6);
    for (i, (id, subject, preview)) in email_hits.iter().enumerate() {
        // Full body for the top hits; the stored preview/snippet for the rest.
        let text = if i < 2 {
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
                Err(_) => preview.clone(),
            }
        } else {
            preview.clone()
        };
        batches.push((format!("email:{subject}"), format!("email: {subject}"), text));
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
        let user =
            format!("Source: {label}{focus}\n\nImage candidates:\n(none)\n\nPage text:\n{clipped}");
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

/// Merge duplicate/overlapping notes once the memo hits its cap, so gathering
/// re-opens instead of dropping every later (often reflect-targeted) finding.
/// Adopt-only-if-better: the rewrite must keep attribution to existing source
/// ids, retain a sane share of the notes, and actually shrink — otherwise the
/// original memo stands and the run degrades exactly as before.
async fn compact_memo(
    state: &Arc<AppState>,
    job: &Job,
    provider: &ProviderConfig,
    topic: &str,
    memo: &mut Vec<Note>,
    memo_chars: &mut usize,
    sources: &[Source],
) {
    let system = prompt(state, "research_memo_compact", topic).await;
    let user = format!(
        "Compact these notes to well under {MEMO_COMPACT_TARGET} characters.\n\nNotes:\n{}",
        memo_as_text(memo)
    );
    let Ok(v) = complete_json(state, &job.user_id, provider, &system, &user).await else {
        tracing::warn!("memo compaction call failed; keeping the full memo");
        return;
    };

    let valid: HashSet<&str> = sources.iter().map(|s| s.id.as_str()).collect();
    let mut new_memo: Vec<Note> = Vec::new();
    let mut new_chars = 0usize;
    for note in v["notes"].as_array().into_iter().flatten() {
        let (Some(source), Some(finding)) = (note["source"].as_str(), note["finding"].as_str())
        else {
            continue;
        };
        // Invented source ids would break citations — drop those notes.
        if !valid.contains(source) || finding.trim().is_empty() {
            continue;
        }
        let quote =
            note["quote"].as_str().filter(|q| !q.trim().is_empty()).map(String::from);
        new_chars += finding.len() + quote.as_deref().map_or(0, str::len);
        new_memo.push(Note { source_id: source.to_string(), finding: finding.to_string(), quote });
    }

    let shrunk = new_chars < *memo_chars && new_chars <= MEMO_COMPACT_TARGET;
    let preserved = new_memo.len() * 3 >= memo.len(); // lost no more than ⅔ of the notes
    if shrunk && preserved {
        tracing::info!(
            "memo compacted: {} notes/{} chars → {} notes/{} chars",
            memo.len(),
            *memo_chars,
            new_memo.len(),
            new_chars
        );
        *memo = new_memo;
        *memo_chars = new_chars;
    } else {
        tracing::warn!(
            "memo compaction rejected (shrunk: {shrunk}, preserved: {preserved}); keeping the full memo"
        );
    }
}

/// Pick which pooled SERP candidates to fetch, best first. One cheap model
/// call; any failure (or a trivial pool) degrades to the round-robin order
/// that matches the old top-N-per-query behavior.
async fn triage(
    state: &Arc<AppState>,
    job: &Job,
    provider: &ProviderConfig,
    triage_system: &str,
    focus: &str,
    candidates: &[SerpCandidate],
    want: usize,
) -> Vec<usize> {
    let query_of: Vec<usize> = candidates.iter().map(|c| c.query_idx).collect();
    if candidates.len() <= want {
        // Nothing to choose between — read them all, still query-interleaved.
        return fallback_order(&query_of, candidates.len());
    }
    let listing = candidates
        .iter()
        .enumerate()
        .map(|(n, c)| format!("{}. {} — {}\n   {}", n + 1, c.title, c.url, c.snippet))
        .collect::<Vec<_>>()
        .join("\n");
    let user = format!("Pick up to {want} results to read.{focus}\n\nSearch results:\n{listing}");
    match complete_json(state, &job.user_id, provider, triage_system, &user).await {
        Ok(v) => {
            let picks = parse_picks(&v, candidates.len(), want);
            if picks.is_empty() {
                fallback_order(&query_of, want)
            } else {
                picks
            }
        }
        Err(e) => {
            tracing::warn!("research triage failed ({e}); using rank order");
            fallback_order(&query_of, want)
        }
    }
}

/// Validate a triage response: 1-based result numbers → unique in-range
/// indices, capped at `want`.
fn parse_picks(v: &Value, len: usize, want: usize) -> Vec<usize> {
    let mut seen = HashSet::new();
    v["picks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .filter(|n| (1..=len as u64).contains(n))
        .map(|n| n as usize - 1)
        .filter(|i| seen.insert(*i))
        .take(want)
        .collect()
}

/// Triage-less order: round-robin across queries (everyone's first result,
/// then everyone's second…) up to URLS_PER_QUERY each — the pre-triage
/// behavior, preserving cross-query spread.
fn fallback_order(query_of: &[usize], want: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let queries = query_of.iter().copied().max().map_or(0, |m| m + 1);
    for round in 0..URLS_PER_QUERY {
        for q in 0..queries {
            if out.len() >= want {
                return out;
            }
            if let Some(idx) =
                query_of.iter().enumerate().filter(|(_, qq)| **qq == q).nth(round).map(|(i, _)| i)
            {
                out.push(idx);
            }
        }
    }
    out
}

/// Split a plan response into (queries, subquestions), both capped.
fn parse_plan(v: &Value) -> (Vec<String>, Vec<String>) {
    let strings = |key: &str, cap: usize| -> Vec<String> {
        v[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|q| q.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from))
            .take(cap)
            .collect()
    };
    (strings("queries", MAX_PLAN_QUERIES), strings("subquestions", MAX_SUBQUESTIONS))
}

/// The sub-question block appended to distill/reflect/synthesize inputs so
/// every stage knows what the investigation is trying to answer — without it,
/// reflect can't judge coverage and distill extracts unfocused trivia.
/// Empty when the plan produced none.
fn focus_block(subquestions: &[String]) -> String {
    if subquestions.is_empty() {
        return String::new();
    }
    format!(
        "\n\nSub-questions guiding this investigation:\n{}",
        subquestions.iter().map(|q| format!("- {q}")).collect::<Vec<_>>().join("\n")
    )
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
    fn parse_picks_validates_dedupes_and_caps() {
        let v = serde_json::json!({ "picks": [3, 1, 3, 99, 0, 2, 4] });
        // 8 candidates, want 3: 3→idx2, 1→idx0, dup 3 skipped, 99/0 out of
        // range, 2→idx1; capped before 4.
        assert_eq!(parse_picks(&v, 8, 3), vec![2, 0, 1]);
        assert!(parse_picks(&serde_json::json!({}), 8, 3).is_empty());
        assert!(parse_picks(&serde_json::json!({ "picks": ["a", null] }), 8, 3).is_empty());
    }

    #[test]
    fn fallback_order_round_robins_across_queries() {
        // Candidates pooled in query order: q0 has 3, q1 has 2.
        let query_of = [0, 0, 0, 1, 1];
        // Everyone's first result, then everyone's second…
        assert_eq!(fallback_order(&query_of, 4), vec![0, 3, 1, 4]);
        assert_eq!(fallback_order(&query_of, 2), vec![0, 3]);
        // URLS_PER_QUERY caps each query's share even when want is larger.
        let many = [0, 0, 0, 0, 0];
        assert_eq!(fallback_order(&many, 5), vec![0, 1, 2]);
        assert!(fallback_order(&[], 3).is_empty());
    }

    #[test]
    fn parse_plan_caps_and_cleans_both_lists() {
        let v = serde_json::json!({
            "queries": ["a", " ", "b", "c", "d", "e", "f", "g"],
            "subquestions": ["q1", "q2"],
        });
        let (queries, subs) = parse_plan(&v);
        assert_eq!(queries.len(), MAX_PLAN_QUERIES); // blank dropped, capped
        assert!(!queries.contains(&" ".to_string()));
        assert_eq!(subs, ["q1", "q2"]);
        // Missing keys yield empties, not errors.
        assert_eq!(parse_plan(&serde_json::json!({})), (vec![], vec![]));
    }

    #[test]
    fn focus_block_lists_subquestions_or_vanishes() {
        assert_eq!(focus_block(&[]), "");
        let block = focus_block(&["How much does it cost?".to_string(), "Is it safe?".to_string()]);
        assert!(block.starts_with("\n\nSub-questions"));
        assert!(block.contains("- How much does it cost?"));
        assert!(block.contains("- Is it safe?"));
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
