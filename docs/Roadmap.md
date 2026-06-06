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

## Phase 8 — Background agent runs + approval queue

Today a gated tool in an unattended run is skipped, and a long agent turn
blocks the chat's SSE connection. This phase is the substrate for autonomy:

- **Background runs**: "do this in the background" from chat (and every
  scheduled-agent run) executes as a tracked job — a `jobs` table with status
  (running/needs-approval/done/failed), the session id, and a summary. Jobs
  survive inspection in History; a small UI affordance lists active jobs.
- **Approval queue**: in unattended runs, an "ask"-gated tool **parks** as a
  `pending_action` instead of being skipped. The run suspends, the user gets a
  push ("Morning briefing wants to log 0.5h to ticket #4521"), and approving
  resumes the run where it stopped (re-entering the agent loop with the tool
  result). Denying resumes with the decline message, as in live chat.
- Notify on completion (push + Logs); failures land with the error attached.
- Builds on: `pending_actions` + approve/reject routes, FCM, scheduler's
  unattended turns. New: job tracking, suspend/resume of a turn across
  process restarts (persist the pending call; resume re-runs from history).

## Phase 9 — Deep research

"Research X and write it up" → acknowledged in chat, runs as a Phase-8
background job, push notification when the report is ready.

- **Orchestrator** (sibling of the categorizer, not the generic agent loop):
  plan (topic → subquestions → queries) → search/fetch/distill per lead →
  reflect (gaps → follow-up queries, hard budget e.g. 20 fetches) →
  synthesize. Every fetched page goes through an extraction prompt and only
  the distilled, citation-tagged note enters the working memo — raw pages
  never accumulate in context (the scratchpad pattern), so local models cope.
- **Sources beyond the web**: the same pass can consult the user's own corpus
  — `search_documents`, `email_search`, `search_memories`, `search_history` —
  so reports cite internal material alongside web sources.
- **Output: a rich, self-contained report page** (decided): the synthesizer
  emits structured findings (sections, claims with citation ids, comparison
  data, image references) and a renderer builds a polished HTML page —
  comparison tables and charts (inline SVG/CSS, no external JS), relevant
  images collected during fetching, and a numbered sources section with
  per-claim citations. Stored server-side (`reports` table or as a document,
  embedded for future RAG) and viewable in a Reports window / browser route;
  mobile opens it in a webview. Decision point at implementation: hotlink
  images vs. download-and-store (privacy favors storing thumbnails locally).
- Cost is visible per run via usage tracking (`purpose = "research"`);
  provider selectable per run, defaulting to the chat default.

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
