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

## Phase 7 — Extras

- **Voice input**: mic button in the Flutter chat tab → `/api/transcribe` →
  Whisper via Groq (provider already supported) or local.
- **Usage tracking**: token counts captured in `model_router::stream`/
  `complete` per user/provider into a `usage` table; admin view in Settings.
