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

/// Per-depth loop bounds: (min_rounds, max_rounds, total web-fetch budget). The
/// IterResearch loop runs at least `min` rounds and at most `max`, stopping early
/// when the model judges the draft comprehensive or the fetch budget runs out.
fn budgets(depth: &str) -> (usize, usize, usize) {
    match depth {
        "quick" => (1, 3, 8),
        "deep" => (3, 8, 40),
        _ => (2, 5, 20), // standard
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

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Note {
    source_id: String,
    finding: String,
    quote: Option<String>,
}

/// Serializable snapshot of a run's gathered state, saved after each round so an
/// interrupted run resumes from where it left off (see `db::research_checkpoint`).
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Checkpoint {
    category: String,
    focus: String,
    draft: String,
    memo: Vec<Note>,
    memo_chars: usize,
    sources: Vec<Source>,
    image_candidates: Vec<ImageCandidate>,
    all_queries: Vec<String>,
    fetched_urls: Vec<String>,
    fetches_left: usize,
    attempts_left: usize,
    compactions_left: usize,
}

/// One pooled SERP result awaiting triage.
struct SerpCandidate {
    /// Which plan query surfaced it — drives the round-robin fallback order.
    query_idx: usize,
    title: String,
    url: String,
    snippet: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
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

    // Resolve the research (writer) model: an explicit choice is also saved as
    // the default; otherwise use the saved default, else the first provider.
    let provider_name = if !provider_arg.is_empty() {
        let _ = db::settings::set(&state.db, "research_provider", &provider_arg.to_string()).await;
        provider_arg.to_string()
    } else {
        db::settings::get::<String>(&state.db, "research_provider")
            .await
            .ok()
            .flatten()
            .filter(|name| providers.iter().any(|p| p.name == *name))
            .unwrap_or_default()
    };

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
        &provider_name,
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
    let (min_rounds, max_rounds, fetch_budget) = budgets(&depth);

    // The research (writer) model — point at Opus/Sonnet — does the reasoning and
    // writing (plan, queries, evolve, stop, final report). The worker model (the
    // first configured, usually local) does the high-volume per-page distill and
    // triage, so a deep run doesn't make dozens of frontier-model extraction calls.
    let writer = provider;
    let worker = first_provider(state).await.unwrap_or_else(|| writer.clone());

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
    let mut compactions_left = MEMO_COMPACTIONS;
    // The evolving working draft — re-synthesized each round, it tells the next
    // round what's still missing and the stop check when we're done.
    let mut draft = String::new();
    let mut focus = String::new();
    let mut category = String::new();
    let mut start_round = 1usize;

    // ── RESUME ──────────────────────────────────────────────────────────────
    // Pick up from the last checkpoint if a prior run was interrupted (e.g. a
    // restart re-enqueued this job) — redoing only the round that was in flight.
    if let Some((ckpt_round, json)) =
        db::research_checkpoint::load(&state.db, &job.id).await.ok().flatten()
    {
        if let Ok(c) = serde_json::from_str::<Checkpoint>(&json) {
            progress(
                state,
                &job.session_id,
                &format!("Resuming where the last run left off (after round {ckpt_round})…"),
            )
            .await;
            category = c.category;
            focus = c.focus;
            draft = c.draft;
            memo = c.memo;
            memo_chars = c.memo_chars;
            sources = c.sources;
            image_candidates = c.image_candidates;
            all_queries = c.all_queries;
            fetched_urls = c.fetched_urls.into_iter().collect();
            fetches_left = c.fetches_left;
            attempts_left = c.attempts_left;
            compactions_left = c.compactions_left;
            start_round = (ckpt_round as usize) + 1;
        }
    }

    let distill_system = prompt(state, "research_distill", &topic).await;
    let triage_system = prompt(state, "research_triage", &topic).await;

    // ── PLAN + CATEGORY (skipped when resuming) ─────────────────────────────
    if start_round == 1 {
        progress(state, &job.session_id, "Planning the investigation…").await;
        let plan_system = prompt(state, "research_plan", &topic).await;
        let subquestions =
            match complete_json(state, &job.user_id, &writer, &plan_system, &topic).await {
                Ok(v) => parse_plan(&v).1,
                Err(e) => {
                    tracing::warn!("research plan failed ({e}); proceeding without sub-questions");
                    Vec::new()
                }
            };
        focus = focus_block(&subquestions);
        category = classify_category(state, &job.user_id, &worker, &topic).await;
    }

    // ── ITERATIVE ROUNDS: think → gather → evolve → decide (IterResearch) ────
    // Highest round actually completed — surfaced in the report's stats bar.
    // Seeded from the resume point so a resumed run still counts prior rounds.
    let mut rounds_done = start_round - 1;
    for round in start_round..=max_rounds {
        // THINK: queries from the plan + what the draft already covers (gaps).
        let mut queries =
            generate_queries(state, &job.user_id, &writer, &topic, &focus, &draft, round, &all_queries).await;
        if queries.is_empty() {
            if round == 1 {
                queries.push(topic.clone());
            } else {
                break; // nothing new worth chasing
            }
        }
        all_queries.extend(queries.iter().cloned());
        progress(state, &job.session_id, &format!("Round {round}: {}", queries.join(" · "))).await;

        let before = memo.len();
        // GATHER: web every round; the user's own corpus on the first round.
        gather_web(
            state, job, &worker, &distill_system, &triage_system, &focus, &queries,
            &mut fetches_left, &mut attempts_left, &mut fetched_urls,
            &mut memo, &mut memo_chars, &mut sources, &mut image_candidates,
        )
        .await;
        if round == 1 {
            progress(state, &job.session_id, "Checking your documents, email, memories, and chats…").await;
            gather_internal(
                state, job, &worker, &distill_system, &focus, &topic, &all_queries,
                &mut memo, &mut memo_chars, &mut sources,
            )
            .await;
        }

        // EVOLVE: fold this round's new notes into the working draft.
        let new_notes = memo_as_text(&memo[before.min(memo.len())..]);
        if !new_notes.trim().is_empty() {
            progress(state, &job.session_id, "Reviewing what we have so far…").await;
            if let Ok(updated) = evolve(state, &job.user_id, &writer, &topic, &draft, &new_notes).await {
                if !updated.trim().is_empty() {
                    draft = updated;
                }
            }
        }

        // Keep the memo under its cap (worker merges duplicates).
        if memo_chars >= MEMO_MAX_CHARS && compactions_left > 0 {
            compactions_left -= 1;
            compact_memo(state, job, &worker, &topic, &mut memo, &mut memo_chars, &sources).await;
        }
        // This round did its gather/evolve work — count it for the stats bar.
        rounds_done = round;

        // CHECKPOINT: snapshot the gathered state so a restart resumes from here.
        let snapshot = Checkpoint {
            category: category.clone(),
            focus: focus.clone(),
            draft: draft.clone(),
            memo: memo.clone(),
            memo_chars,
            sources: sources.clone(),
            image_candidates: image_candidates.clone(),
            all_queries: all_queries.clone(),
            fetched_urls: fetched_urls.iter().cloned().collect(),
            fetches_left,
            attempts_left,
            compactions_left,
        };
        if let Ok(json) = serde_json::to_string(&snapshot) {
            let _ = db::research_checkpoint::save(&state.db, &job.id, round as i64, &json).await;
        }

        if fetches_left == 0 || attempts_left == 0 {
            break;
        }
        // DECIDE: past the minimum, stop once the draft is comprehensive.
        if round >= min_rounds
            && should_stop(state, &job.user_id, &writer, &topic, &draft, &focus).await
        {
            break;
        }
    }

    if memo.is_empty() {
        let _ = db::research_checkpoint::delete(&state.db, &job.id).await;
        return Err(anyhow!(
            "no usable sources found — web search may be down and nothing internal matched"
        ));
    }

    // ── SYNTHESIZE (final visual report) ────────────────────────────────────
    progress(state, &job.session_id, "Writing the report…").await;
    let synth_system = prompt(state, "research_synthesize", &topic).await;
    let doc = synthesize(
        state, job, &writer, &topic, &focus, &category, &draft, &synth_system,
        &memo, &sources, &image_candidates,
    )
    .await;

    // ── IMAGES ─────────────────────────────────────────────────────────────
    let images = embed_images(state, &doc, &image_candidates).await;

    // ── RENDER + PERSIST ───────────────────────────────────────────────────
    let tz = state.home_tz(&job.user_id).await;
    let generated = chrono::Utc::now().with_timezone(&tz).format("%-d %B %Y").to_string();
    let stats = render::ReportStats {
        depth: depth.clone(),
        rounds: rounds_done,
        queries: all_queries.len(),
        sources: sources.len(),
        model: writer.model_id.clone(),
    };
    let html = render::render_report(&doc, &category, &stats, &sources, &images, &generated);
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

    // The run finished — drop the resume checkpoint.
    let _ = db::research_checkpoint::delete(&state.db, &job.id).await;

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

/// Memo char budget for the trimmed synthesis retry — small enough that a
/// model which stalled on the full prompt can finish.
const SYNTH_RETRY_MEMO_CHARS: usize = 8_000;

/// Write the report, degrading gracefully so a late stall never wastes the
/// gathering that already finished. Ladder: (1) synthesize from everything;
/// (2) on failure/timeout — almost always an over-large prompt for the model —
/// retry once with a hard-trimmed memo and no image catalogue; (3) only then
/// the grouped-notes fallback, which is still a complete, cited report.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
async fn synthesize(
    state: &Arc<AppState>,
    job: &Job,
    provider: &ProviderConfig,
    topic: &str,
    focus: &str,
    category: &str,
    draft: &str,
    synth_system: &str,
    memo: &[Note],
    sources: &[Source],
    image_candidates: &[ImageCandidate],
) -> ReportDoc {
    if let Some(doc) = try_synthesize(
        state, job, provider, synth_system, focus, category, draft, memo, sources, image_candidates,
    )
    .await
    {
        return doc;
    }
    progress(
        state,
        &job.session_id,
        "The report is taking a while — condensing the notes and trying once more…",
    )
    .await;
    let trimmed = trim_memo(memo, SYNTH_RETRY_MEMO_CHARS);
    if let Some(doc) = try_synthesize(
        state, job, provider, synth_system, focus, category, draft, &trimmed, sources, &[],
    )
    .await
    {
        return doc;
    }
    progress(state, &job.session_id, "Couldn't synthesize — assembling a notes-only report.").await;
    fallback_report(topic, memo, sources)
}

/// One synthesis attempt. None when the call failed/timed out or the model
/// produced nothing usable (no sections) — the caller then degrades.
#[allow(clippy::too_many_arguments)]
async fn try_synthesize(
    state: &Arc<AppState>,
    job: &Job,
    provider: &ProviderConfig,
    synth_system: &str,
    focus: &str,
    category: &str,
    draft: &str,
    memo: &[Note],
    sources: &[Source],
    image_candidates: &[ImageCandidate],
) -> Option<ReportDoc> {
    let user = synth_user_prompt(focus, category, draft, memo, sources, image_candidates);
    match complete_json(state, &job.user_id, provider, synth_system, &user).await {
        Ok(v) => {
            let doc: ReportDoc = serde_json::from_value(v).unwrap_or_default();
            (!doc.sections.is_empty()).then_some(doc)
        }
        Err(e) => {
            tracing::warn!("research synthesis attempt failed: {e}");
            None
        }
    }
}

/// The synthesis user prompt: focus block, cited notes, the source key, and
/// the candidate-image catalogue.
fn synth_user_prompt(
    focus: &str,
    category: &str,
    draft: &str,
    memo: &[Note],
    sources: &[Source],
    image_candidates: &[ImageCandidate],
) -> String {
    let guidance = category_guidance(category);
    format!(
        "{}{}{}Notes (each tagged with its source id):\n{}\n\nSources:\n{}\n\nCandidate images:\n{}",
        if guidance.is_empty() { String::new() } else { format!("{guidance}\n\n") },
        if focus.is_empty() { String::new() } else { format!("{}\n\n", focus.trim_start()) },
        if draft.trim().is_empty() {
            String::new()
        } else {
            format!("Working draft assembled while researching (use as a structural guide; cite from the notes):\n{draft}\n\n")
        },
        memo_as_text(memo),
        sources.iter().map(|s| format!("{} = {}", s.id, s.label)).collect::<Vec<_>>().join("\n"),
        if image_candidates.is_empty() {
            "(none)".to_string()
        } else {
            image_candidates
                .iter()
                .map(|i| format!("{} — {} ({})", i.id, i.caption, i.url))
                .collect::<Vec<_>>()
                .join("\n")
        },
    )
}

/// Keep notes from the front (web/higher-ranked first) until `max_chars`,
/// preserving at least one note so the retry is never empty.
fn trim_memo(memo: &[Note], max_chars: usize) -> Vec<Note> {
    let mut out: Vec<Note> = Vec::new();
    let mut used = 0usize;
    for n in memo {
        let cost = n.finding.len() + n.quote.as_deref().map_or(0, str::len);
        if !out.is_empty() && used + cost > max_chars {
            break;
        }
        used += cost;
        out.push(Note {
            source_id: n.source_id.clone(),
            finding: n.finding.clone(),
            quote: n.quote.clone(),
        });
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

/// Hard ceiling on any single research model call. A synthesis on a slow local
/// model can legitimately take minutes; hours means the provider has wedged.
/// Timing out here surfaces as an error so the orchestrator's per-stage
/// fallbacks run (synthesis → grouped-notes report) instead of the job hanging
/// in `running` forever. Belt to the model-router read-timeout's braces — this
/// also bounds the cloud/genai path, which has no per-request timeout.
const MODEL_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

async fn complete_with_timeout(
    provider: &ProviderConfig,
    history: Vec<crate::model_router::ChatMessage>,
) -> Result<(String, Option<crate::model_router::TokenUsage>)> {
    match tokio::time::timeout(MODEL_CALL_TIMEOUT, ModelRouter::complete_with_usage(provider, history))
        .await
    {
        Ok(res) => res,
        Err(_) => Err(anyhow!(
            "model call timed out after {}s — the provider may be unresponsive",
            MODEL_CALL_TIMEOUT.as_secs()
        )),
    }
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
    let (raw, used) = complete_with_timeout(provider, history("")).await?;
    db::usage::record(&state.db, user_id, provider, "research", used).await;
    if let Some(v) = json_slice(&raw) {
        return Ok(v);
    }
    let (raw, used) =
        complete_with_timeout(provider, history("\nReturn ONLY valid JSON, no prose.")).await?;
    db::usage::record(&state.db, user_id, provider, "research", used).await;
    json_slice(&raw).ok_or_else(|| anyhow!("model did not return parseable JSON"))
}

/// Plain-text model call recording usage under "research".
async fn complete_text(
    state: &AppState,
    user_id: &str,
    provider: &ProviderConfig,
    system: &str,
    user: &str,
) -> Result<String> {
    use crate::model_router::ChatMessage;
    let history = vec![
        ChatMessage { role: "system".into(), content: Value::String(system.to_string()) },
        ChatMessage { role: "user".into(), content: Value::String(user.to_string()) },
    ];
    let (raw, used) = complete_with_timeout(provider, history).await?;
    db::usage::record(&state.db, user_id, provider, "research", used).await;
    Ok(raw)
}

/// First configured provider — the "worker" model for high-volume distill/triage.
async fn first_provider(state: &AppState) -> Option<ProviderConfig> {
    db::settings::get::<Vec<ProviderConfig>>(&state.db, "providers")
        .await
        .ok()
        .flatten()?
        .into_iter()
        .next()
}

/// Classify the topic into a report category (shapes the final report).
async fn classify_category(
    state: &AppState,
    user_id: &str,
    worker: &ProviderConfig,
    topic: &str,
) -> String {
    let system = prompt(state, "research_category", topic).await;
    let raw = match complete_text(state, user_id, worker, &system, topic).await {
        Ok(t) => t.to_lowercase(),
        Err(_) => return "general".into(),
    };
    for cat in ["product", "comparison", "howto", "factcheck"] {
        if raw.contains(cat) {
            return cat.into();
        }
    }
    "general".into()
}

/// Extra per-category structure instructions folded into the final synthesis.
fn category_guidance(category: &str) -> &'static str {
    match category {
        "product" => "This is a PRODUCT report: rank the options best-first; for each give a heading, a 2-3 sentence summary, pros and cons, and an approximate price; include a quick-compare table (Name, Price, Best for) and end with a verdict (best overall, best value).",
        "comparison" => "This is a COMPARISON report: include a comparison table (criteria as rows, options as columns); a section per option covering strengths, weaknesses, and ideal use; and end with 'best for' verdicts.",
        "howto" => "This is a HOW-TO guide: open with a concise numbered quick-guide, then prerequisites, then detailed numbered steps each under its own heading, and a common-mistakes section.",
        "factcheck" => "This is a FACT-CHECK: restate the claim; give evidence-for and evidence-against sections; end with a verdict (Supported / Mixed evidence / Unsupported) and caveats.",
        _ => "",
    }
}

/// THINK: web search queries from the plan + the evolving draft's gaps.
#[allow(clippy::too_many_arguments)]
async fn generate_queries(
    state: &AppState,
    user_id: &str,
    writer: &ProviderConfig,
    topic: &str,
    focus: &str,
    draft: &str,
    round: usize,
    used: &[String],
) -> Vec<String> {
    let system = prompt(state, "research_queries", topic).await;
    let instruction = if round == 1 {
        "This is the first round — go broad across the key facets of the topic."
    } else {
        "We already have partial findings — generate targeted follow-ups for the gaps and weakly-sourced claims; do not repeat earlier queries."
    };
    let user = format!(
        "Research plan (sub-questions):{}\n\nReport so far:\n{}\n\nRound {round}. {instruction}",
        if focus.is_empty() { " (none)".to_string() } else { focus.to_string() },
        if draft.trim().is_empty() { "(nothing yet)" } else { draft },
    );
    let want = if round == 1 { 5 } else { 3 };
    match complete_json(state, user_id, writer, &system, &user).await {
        Ok(v) => v
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|q| q.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from))
            .filter(|q| !used.iter().any(|u| u.eq_ignore_ascii_case(q)))
            .take(want)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// EVOLVE: fold the round's new notes into the internal working draft.
async fn evolve(
    state: &AppState,
    user_id: &str,
    writer: &ProviderConfig,
    topic: &str,
    draft: &str,
    new_notes: &str,
) -> Result<String> {
    let system = prompt(state, "research_evolve", topic).await;
    let user = format!(
        "Current draft:\n{}\n\nNew notes this round:\n{}",
        if draft.trim().is_empty() { "(empty — start the draft)" } else { draft },
        new_notes,
    );
    Ok(complete_text(state, user_id, writer, &system, &user).await?.trim().to_string())
}

/// DECIDE: is the draft comprehensive enough to write the final report?
async fn should_stop(
    state: &AppState,
    user_id: &str,
    writer: &ProviderConfig,
    topic: &str,
    draft: &str,
    focus: &str,
) -> bool {
    let system = prompt(state, "research_should_stop", topic).await;
    let user = format!(
        "Sub-questions:{}\n\nWorking draft:\n{}",
        if focus.is_empty() { " (none)".to_string() } else { focus.to_string() },
        draft,
    );
    match complete_text(state, user_id, writer, &system, &user).await {
        Ok(t) => t.trim_start().to_ascii_uppercase().starts_with("YES"),
        Err(_) => false,
    }
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
    fn budgets_scale_with_depth_and_min_le_max() {
        for d in ["quick", "standard", "deep", "other"] {
            let (min, max, fetch) = budgets(d);
            assert!(min >= 1 && min <= max && fetch > 0);
        }
        assert!(budgets("deep").1 > budgets("standard").1);
        assert!(budgets("standard").1 > budgets("quick").1);
    }

    #[test]
    fn category_guidance_only_for_known_categories() {
        for c in ["product", "comparison", "howto", "factcheck"] {
            assert!(!category_guidance(c).is_empty(), "{c} should have guidance");
        }
        assert!(category_guidance("general").is_empty());
        assert!(category_guidance("nonsense").is_empty());
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
        assert_eq!(budgets("quick"), (1, 3, 8));
        assert_eq!(budgets("standard"), (2, 5, 20));
        assert_eq!(budgets("deep"), (3, 8, 40));
        assert_eq!(budgets("nonsense"), (2, 5, 20));
    }

    #[test]
    fn trim_memo_bounds_chars_but_keeps_at_least_one() {
        let memo: Vec<Note> = (0..10)
            .map(|i| Note { source_id: format!("S{i}"), finding: "x".repeat(100), quote: None })
            .collect();
        // ~250-char budget keeps 2 notes (100 each; the third would exceed).
        let trimmed = trim_memo(&memo, 250);
        assert_eq!(trimmed.len(), 2);
        // A budget smaller than one note still keeps exactly one (never empty).
        assert_eq!(trim_memo(&memo, 1).len(), 1);
        assert!(trim_memo(&[], 9_000).is_empty());
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
