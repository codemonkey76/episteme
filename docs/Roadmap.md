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
  package), `/api/transcribe` forwards it (multipart) to the first configured
  Groq provider's Whisper endpoint (`whisper-large-v3-turbo`; OpenAI
  `whisper-1` fallback) and the transcript lands in the composer for review.
- **Usage tracking**: `StreamChunk.usage` carries provider-reported token
  counts (genai `capture_usage`; Ollama eval counts); recorded per
  user/provider/model/purpose in the `usage` table from every call site
  (chat, memory, style, auto-sort, commitments, email-ai); admin summary at
  `/api/usage/summary` and a table in Settings → System.

---

Phases 1–7 shipped (June 2026). The second arc below makes the agent able to
act and research on its own, safely.

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
Mobile follow-up deferred: a global Jobs/Approvals tab.

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
`research`; SearXNG outages degrade to internal-corpus-only. Follow-ups:
ingest reports into documents-RAG; mobile Reports view.

## Phase 10 — Context compaction

Every turn replays the session's entire history, including fat tool results
(8k-char email bodies, 12k-char pages). Long sessions degrade quietly.

- Past a size threshold, old turns are summarized (one `complete_with_usage`
  call, purpose `compaction`) into a compact preamble; the recent window stays
  verbatim. Tool results older than N turns are aggressively truncated first
  (cheap, no model call) before summarization kicks in.
- Persisted alongside the session so compaction happens once, not per turn;
  the full transcript remains in `messages` for display/search — only the
  model-facing history shrinks.
- Generalizes the scratchpad lessons from Phase 9.

## Backlog (quick wins, any time)

- **Dollar costs**: per-model price table → $ column in Settings → System.
- **Semantic email**: embed incoming mail (Phase-2 infra) so email search
  works by meaning, not just Graph `$search`.
- **Web push**: browser notifications for commitment cards and job/agent
  output (currently mobile-only via FCM).
