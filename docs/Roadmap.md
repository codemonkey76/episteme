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

## Phase 3 — Documents + RAG

No way to give the assistant a PDF, contract, or reference folder today.

- Tables: `documents` + `document_chunks(id, document_id, user_id, content,
  embedding BLOB)`.
- Multipart upload route (`/api/documents`); text/markdown extracted natively,
  PDF via a Rust crate (evaluate `pdf-extract` at implementation time).
- Chunk ~1k chars with overlap, embed per chunk (Phase 2 infra).
- `search_documents(query, limit)` tool doing cosine top-k.
- Frontend: `views/Documents.vue` registered in `windows/registry.ts` +
  sidebar button (follow the Notes window pattern). Mobile later.

## Phase 4 — Multimodal chat input

The model router **already supports** image content
(`{type:"multimodal", text, images:[{mime,b64}]}` — `model_router/mod.rs`);
only the chat route and composers don't accept it.

- Extend `ChatRequest` (`routes/chat.rs`) to take the multimodal shape.
- Web: image paste/drop in the chat composer (`views/Chat.vue`), reusing
  `RichTextEditor.vue`'s paste/FileReader logic.
- Mobile: wire the already-present `file_picker` into the chat tab.

## Phase 5 — Conversation search

- FTS5 virtual table over `messages` if the bundled SQLite in sqlx enables it
  (verify first); otherwise `LIKE` over `messages.content` using the existing
  `(session_id, created_at)` index.
- `/api/sessions/search?q=` + a search box in the sessions list.
- Optional `search_history` agent tool.

## Phase 6 — Scheduled agents + push notifications

- Worker modeled on `categorizer::spawn_worker`: per-user list of
  `{name, schedule (cron), instructions, provider, enabled}` in settings.
- Each run executes a one-shot agent turn with tools enabled; anything with an
  "ask" policy is skipped and surfaced rather than auto-approved. Output lands
  as a suggestions-style card and/or note.
- Push: `firebase_messaging` in the Flutter app (the Firebase project already
  exists for App Distribution), `/api/push/register` token route, backend
  sends via FCM HTTP v1 with a service-account key. Notify on scheduled-agent
  output, commitment cards, and auto-sort "needs attention".

## Phase 7 — Extras

- **Voice input**: mic button in the Flutter chat tab → `/api/transcribe` →
  Whisper via Groq (provider already supported) or local.
- **Usage tracking**: token counts captured in `model_router::stream`/
  `complete` per user/provider into a `usage` table; admin view in Settings.
