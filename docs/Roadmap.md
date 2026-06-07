# Roadmap

Phased plan toward a fully functional AI workspace. Phase 1 (email agent tools
+ SearXNG web search) shipped; the phases below are sequenced so each builds on
the previous one's infrastructure. Each gets a detailed design pass when picked
up — this records the intent and the decisions already made.

## Phase 2 — Embedding infra + semantic memory ✅ (shipped)

Ollama-only embeddings (`nomic-embed-text`, overridable via the
`embedding_model` settings key) in `integrations/embeddings.rs`; `embedding
BLOB` on `memories` (f32 LE, brute-force cosine in Rust — no sqlite-vec);
`memory::inject()` selects top-30 by relevance + newest 10 once the store
exceeds 50, with recency fallback when Ollama is unreachable; lazy backfill
embeds pre-existing rows; `search_memories` agent tool. Requires
`ollama pull nomic-embed-text` on the Ollama host.

## Phase 3 — Documents + RAG ✅ (shipped)

`documents` + `document_chunks` tables; uploads as base64 JSON (same shape as
email attachments, 25 MB cap); text/markdown/HTML/CSV/JSON native extraction,
PDF via `pdf-extract`; ~1200-char chunks with 200 overlap on paragraph/line
boundaries; chunks embedded detached with live indexing status; cosine top-k
`search_documents` tool with substring fallback; Documents window
(upload/drag-drop/delete) in the web UI. Mobile tab still to come.

## Phase 4 — Multimodal chat input ✅ (shipped)

`ChatRequest` accepts `images: [{mime, b64}]` (max 4 × 6 MB, 40 MB body
limit on the chat route); stored as the `{type:"multimodal"}` shape the model
router already handled. Web composer: paste/drop with pending thumbnails and
image rendering in history; queued messages carry their images. Mobile: attach
button (file_picker), preview strip, image bubbles. Memory extraction and
semantic injection read only the text part of multimodal messages.

## Phase 5 — Conversation search ✅ (shipped)

FTS5 (confirmed in sqlx's bundled SQLite) virtual table `message_fts` synced
by insert/delete triggers that extract plain text from the JSON-encoded
content (multimodal rows index only `$.text` — never base64); backfill in the
migration; `/api/sessions/search?q=` ranked by FTS rank with snippets; search
box in the History window; `search_history` agent tool. Covered by an
in-memory integration test (`db::tests::message_search_via_fts5`).

## Phase 6 — Scheduled agents + push notifications ✅ (shipped)

Per-user agents `{name, time HH:MM, days, provider, instructions, enabled}`
(Settings → Agents), fired by a minute-tick worker in each user's home
timezone, once per local date (catching up after restarts). Runs execute
`agent::run_turn` with `unattended=true` — "ask"-gated tools are skipped with
an explanatory tool result, never auto-approved — into a fresh "⏰" session
visible in History; outcome is logged and pushed. Push: FCM HTTP v1 via a
service-account JSON at `$FCM_SERVICE_ACCOUNT` (default
`{DATA_DIR}/firebase-service-account.json`); `push_tokens` table +
`/api/push/register`; dead tokens pruned on UNREGISTERED. Notifies on
scheduled-agent output, commitment detection, and auto-sort flags. Mobile:
`firebase_core`/`firebase_messaging`, google-services Gradle plugin applied
only when `google-services.json` exists (CI builds stay green without it),
token registered after login.

## Phase 7 — Extras ✅ (shipped)

- **Voice input**: mic button in the Flutter chat tab records AAC (`record`
  package), `/api/transcribe` forwards it (multipart) to the self-hosted
  Whisper sidecar (compose service `whisper`, Speaches CPU image; model via
  `WHISPER_MODEL`, default `Systran/faster-whisper-small`), falling back to a
  configured Groq (`whisper-large-v3-turbo`) or OpenAI (`whisper-1`) provider.
  The transcript lands in the composer for review.
- **Usage tracking**: `StreamChunk.usage` carries provider-reported token
  counts (genai `capture_usage`; Ollama eval counts); recorded per
  user/provider/model/purpose in the `usage` table from every call site
  (chat, memory, style, auto-sort, commitments, email-ai); admin summary at
  `/api/usage/summary` and a table in Settings → System.

---

Phases 1–7 shipped (June 2026). The second arc below — also fully shipped —
made the agent able to act and research on its own, safely.

## Phase 8 — Background agent runs + approval queue ✅ (shipped)

`jobs` table + `jobs` module wrap every unattended run (chat-triggered
`start_background_task` tool and all scheduled-agent fires) with status
running/needs_approval/done/failed, summary, and push notifications. Gated
tools **park**: `run_turn` returns `TurnOutcome::Suspended`, each gated call
becomes a `pending_actions` row carrying its `call_id`; deciding it (web Jobs
window's global queue, chat cards, or mobile) executes/declines the tool via
the shared `agent::execute_tool`, writes the result row, and — when the
session's last pending row is decided — resumes the job through an atomic
`try_resume` gate and the job queue on AppState (which also breaks the
agent→tool→job async-recursion cycle). Everything persists in SQLite, so
suspend/resume survives restarts. Also fixed en route: assistant text emitted
alongside tool calls is now persisted per-iteration (it was lost on replay).
Mobile follow-up shipped: the Activity tab (Jobs segment) — global
approval queue with approve/deny, recent runs with status, badge on the tab
icon, jump-to-session into Chat.

## Phase 9 — Deep research ✅ (shipped)

`deep_research(topic, depth quick|standard|deep)` tool → `kind='research'`
job (migration 018 rebuilds jobs for the kind + a `meta` JSON column).
`research::run` orchestrates plan → gather (SearXNG + `fetch_readable`, which
also scrapes ≤3 image candidates/page) → distill-per-source (scratchpad memo
of citation-tagged notes, 24k-char cap) → internal corpus pass (documents/
memories cosine, chat FTS, email `$search`) → reflect rounds within budget
(6/12/20 fetches, 0/1/2 rounds) → synthesize to a structured ReportDoc, with
tolerant JSON parsing, one retry, and a grouped-notes fallback report. Images
the model picks (only from real page scans) are fetched (SSRF-guarded,
`image/*`, ≤1.5MB) and embedded as data URIs. The pure renderer emits one
self-contained HTML doc (light/dark CSS, per-claim `[n]` anchors, tables,
inline-SVG bar charts, numbered sources; every model string HTML-escaped).
Reports persist in a `reports` table, served raw at `/api/reports/:id/html`,
browsed in a Reports window (iframe + open-in-tab + delete). Four editable
prompts (`research_plan/distill/reflect/synthesize`); usage purpose
`research`; SearXNG outages degrade to internal-corpus-only. Mobile Reports
shipped: Activity tab (Reports segment) with a research launcher and a
no-JS WebView viewer (HTML fetched over the authed client, rendered via
loadHtmlString — no cookies in the WebView). Follow-up shipped (June 2026):
finished reports are rendered to plain markdown (`render_markdown` — no HTML
boilerplate, data-URI images, or citation anchors) and fed through the
documents pipeline as `Research report: {title}.md`, so later chats and the
next research run's internal pass retrieve past reports by meaning. They
appear in the Documents window like any upload and can be deleted there.

Quality pass (June 2026): the plan's sub-questions — previously generated and
discarded — now thread through every stage (distill extracts against them,
reflect judges coverage by them, synthesize answers them); the internal
corpus legs probe chat FTS and email `$search` with the plan's short queries
instead of the whole AND-everything topic string, and the email leg also
queries the semantic email index; failed page fetches refund their budget
slot (bounded by a 2× attempt cap) so flaky sources no longer shrink a run.

Second pass (June 2026): **SERP triage** — each round pools every query's
results and one model call (`research_triage` prompt) picks which pages
deserve the fetch budget (authority/recency/domain-diversity), falling back
to the old round-robin rank order on any failure; **memo compaction** — at
the 24k cap the memo is merged (`research_memo_compact` prompt, ≤2 calls per
run) at stage boundaries instead of refusing new notes, adopt-only-if-better
(must shrink below 19k, keep ≥⅓ of the notes, and cite only existing source
ids). Still open: chunked distill for long pages, recency/authority signals
passed to synthesis, single-source claim flagging, a separate cheap distill
model.

## Phase 10 — Context compaction ✅ (shipped)

Two layers in the new `compaction` module, both shrinking only the
model-facing history `run_turn` builds — the full transcript stays in
`messages` for display/FTS. (1) Cheap, every turn: tool results outside the
most recent 12 messages are clipped to 1,500 chars. (2) Rolling summarization:
after a completed turn (attended chat and unattended jobs alike), a detached
task measures the live history; past 48k chars it summarizes everything but
the recent window — boundary snapped to a user message so a kept tool result
is never orphaned from its tool_call — into `sessions.summary` (migration
019), advancing a `sessions.summary_until` created_at cursor. The next turn
loads only rows past the cursor and injects the summary as a system message
(after the tool preamble and memories); each compaction folds the previous
summary in, so it rolls forever. Multimodal messages count and render only
their text part — base64 never reaches the summarizer. Editable prompt
`session_compact`; usage purpose `compaction`; failures log and leave the
session uncompacted.

## Backlog (quick wins, any time)

- **Semantic email** ✅ (shipped): the auto-sort worker embeds every message
  it fetches (`from + subject + preview`, Phase-2 Ollama infra) into
  `email_embeddings` (migration 020, capped at the newest 20k rows/user);
  `email_search` merges Graph `$search` keyword hits with cosine top-k
  semantic hits (floor 0.5, marked `matched_by: "meaning"`). Coverage is
  "mail seen by auto-sort" — only mailboxes with sorting enabled get indexed.
- **Web push** ✅ (shipped): native Web Push (VAPID, no Firebase JS) via the
  `web-push` crate. Keys auto-generate into settings on first use; browsers
  subscribe from Settings → Account → Browser notifications (`/sw.js` +
  `GET /api/push/vapid`), registering their subscription as a
  `platform="web"` push token. All notify call sites fan out through
  `integrations::push::notify` → FCM (mobile) + Web Push (browsers); dead
  subscriptions are pruned on send like FCM tokens.
- **Research reports into RAG** ✅ (shipped): see the Phase 9 follow-up note.
- **Dollar costs**: per-model price table → $ column in Settings → System.
