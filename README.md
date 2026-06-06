<img src="assets/icon-96.png" align="left" width="72" height="72" alt="Episteme icon" style="margin-right: 12px">

# Episteme

A self-hosted AI workspace. One interface over local and cloud models, with an agentic tool-using loop and human-in-the-loop approvals for irreversible actions.

Your conversations and credentials stay on your machine. The only data that leaves it is what you explicitly route to a cloud provider you have configured.

## What's built

- **Streaming chat** — server-sent token streaming over a persistent axum/SSE backend, with keep-alive so slow local models don't drop the connection; assistant replies are persisted and survive a refresh
- **Sessions** — conversations persisted to SQLite; resume any past session
- **Model router** — route to Anthropic, OpenAI, Ollama, Gemini, Groq, DeepSeek, or any OpenAI-compatible endpoint from a single UI; switching is choosing a name from a list
- **Agent loop with native tools** — the model can call built-in tools (e.g. calendar management), inspect the result, and loop until it reaches a final answer; tool activity is surfaced live in the chat. Works with Ollama function-calling too
- **MCP host** — connect third-party MCP tool servers (stdio or streamable HTTP) from Settings; their tools are offered to the model alongside native ones, namespaced per server (`server__tool`). Connection status and tool counts show live in Settings, and connect/error events land in the Logs window. The Docker image ships Node (`npx`) and uv (`uvx`), so common stdio servers work out of the box — package caches persist to the data volume
- **Persistent memory** — durable facts and preferences are auto-extracted from conversations (and addable by hand) and injected into future chats, so the assistant improves over time. The **Memories** window lets you view, filter, edit, and delete them
- **Style learning for email drafts** — when you edit an AI-drafted reply before sending, the diff is analyzed and durable writing-style lessons (tone, length, sign-offs) are saved as `style` memories that steer every future draft — the AI converges on your voice
- **Email (Microsoft 365)** — folders, message list with search, reading pane, flagged / replied-to indicators, attachment viewer, AI-drafted replies/forwards, and **AI auto-sort** that files low-priority mail into folders and flags what needs attention
- **Email agent tools** — the chat agent can search, list, and read your mail ("what did Jo say about the invoice?") and prepare replies/forwards/new messages as **drafts in your Drafts folder** — sending always stays with you (there is deliberately no send tool)
- **Web search** — a self-hosted [SearXNG](https://github.com/searxng/searxng) sidecar ships in docker-compose; the agent searches the web and reads pages through it (`web_search` / `fetch_page`), so queries never go to a search provider you don't control. Copy `searxng/settings.yml.example` to `searxng/settings.yml` and set a `secret_key` to enable it
- **Calendar (Microsoft 365)** — an agenda view with manual add/delete, plus chat-driven scheduling: ask the AI to add appointments or reminders and it creates them via the calendar tools
- **Tasks** — a to-do window (priorities, due dates, search) the AI can also drive: "remind me to buy milk tomorrow" creates a task via the task tools, and the window refreshes live when chat changes the list
- **Notes** — freeform markdown notes with search and inline rendering, also AI-driven: "save a note about…" creates one, "add X to that note" appends, and "check my notes" recalls — refreshing the window live
- **Inbox zero on reply** — replying to a message marks its follow-up flag complete and files it from the Inbox into a "Processed" folder automatically, so the Inbox only holds mail still awaiting action
- **Commitment detection** — every email you send is scanned for promises with a date/time ("I'll do the maintenance Saturday 9pm"); episteme pops an accept/dismiss card offering to add it as a calendar event or task
- **Per-tool approval** — every tool (native and MCP) has an "ask first" toggle in **Settings → Tools**; flagged tools pause the chat with an inline approve/deny card and only run once you allow them. Approvals and denials are logged
- **Floating window workspace** — dockable/snappable windows (chat, email, calendar, memories, logs, settings); the layout and which windows were open are remembered across reloads
- **Logs** — a live, filterable log window fed by both frontend and backend events
- **Multi-user accounts** — the first account is the admin; new people join via single-use invite links (created in **Settings → Users**, emailed by you, valid 14 days). Every user gets their own sessions, tasks, notes, memories, suggestions, timezone, and Microsoft 365 connection; providers, MCP servers, tool policies, and the shared Azure app registration stay admin-managed. Members can be disabled (sessions revoked instantly) or deleted with their data. Passwords hashed with argon2; HttpOnly session cookies protect every route
- **Settings** — manage model providers, MCP servers, the Microsoft 365 connection, and auto-sort through the UI (no recompile required)
- **Docker** — multi-stage build produces a single static binary + assets; `docker compose up` for a self-contained deployment

## What's next

See [docs/Roadmap.md](docs/Roadmap.md) for the phased plan; headlines:

- **Semantic memory** — memories are currently all injected (capped); relevance-based retrieval (Ollama embeddings) would scale to large stores
- **Documents + RAG** — upload PDFs/docs into a searchable knowledge base the agent can pull from
- **Multimodal chat** — paste images into chat (the model router already supports it; the composer doesn't yet)
- **Scheduled agents + push** — user-defined recurring agent runs ("summarize overnight email at 7am") with mobile push notifications
- **Voice input & usage tracking** — mobile STT, per-user token/cost accounting

## Mobile app

A Flutter app (Android-first; the codebase is iOS-ready but unbuilt) lives in [`mobile/`](mobile/). It talks to the same backend over HTTPS — point it at your deployed domain at the login screen. Mobile swaps the floating-window workspace for bottom tabs: Chat, Email, Calendar, Tasks, Notes (Tasks and Notes are live; the rest land in later phases).

```sh
cd mobile
flutter pub get
flutter run -d linux          # desktop dev loop
flutter build apk --debug     # Android APK -> build/app/outputs/flutter-apk/
```

Gradle needs a JDK ≤ 21 — if your system Java is newer, point `org.gradle.java.home` at one in `~/.gradle/gradle.properties`.

## Stack

| Layer | Technology |
|---|---|
| Backend | Rust, axum, tokio, sqlx + SQLite |
| Model routing | `genai` (native protocols for Anthropic, OpenAI, Ollama, and others) |
| MCP host | `rmcp` (official Rust SDK) |
| Frontend | Vue 3 + TypeScript, Vite, Pinia |

## Running locally

**Prerequisites:** Rust (stable), Node.js 22+

```sh
# 1. Clone and enter the repo
git clone <repo-url>
cd episteme

# 2. Copy the example env file and edit as needed
cp .env.example .env

# 3. Build the frontend
cd frontend && npm install && npm run build && cd ..

# 4. Run the backend (serves the built frontend from backend/static)
cargo run --release
```

Open `http://localhost:3000`. On first launch you'll be prompted to **create an admin account**; after that, add a model provider in Settings (name, endpoint, API key) and start chatting.

> When running over plain HTTP like this (no TLS), set `AUTH_COOKIE_INSECURE=1` in `.env` — otherwise the `Secure` session cookie is rejected by the browser and you can't stay logged in.

**Dev mode** — run the backend and frontend separately for hot reload:

```sh
# Terminal 1: backend (set AUTH_COOKIE_INSECURE=1 in .env for http login)
cargo run

# Terminal 2: frontend dev server (proxies API calls to :3000)
cd frontend && npm run dev
```

## Docker

```sh
# Copy and edit .env first (API keys, etc.)
cp .env.example .env

docker compose up
```

Data is persisted to a named Docker volume (`episteme_data`). The container exposes port 3000.

### Deploying with HTTPS

The Compose file includes a [Caddy](https://caddyserver.com/) reverse proxy that terminates TLS and obtains/renews a Let's Encrypt certificate automatically. The `episteme` service is internal-only; Caddy owns ports 80 and 443.

**Prerequisites:**

1. A domain you control (Let's Encrypt won't issue for a bare IP).
2. A DNS `A`/`AAAA` record pointing that domain at the server's public IP.
3. Ports **80** and **443** open to the internet — port 80 is required for the ACME challenge, not just a redirect.

**Configure and deploy:**

```sh
# In .env (gitignored — your domain is never committed):
echo "EPISTEME_DOMAIN=episteme.example.com" >> .env

docker compose up -d --build

# Watch the certificate get issued on first boot:
docker compose logs -f caddy
```

Issued certs are persisted in the `caddy_data` volume, so restarts reuse them (and don't hit Let's Encrypt rate limits). Caddy auto-renews from there.

> **Debugging tip:** Let's Encrypt limits failed validations to 5/hour per hostname. Confirm DNS resolves and ports 80/443 are reachable *before* the first `up`. If you get rate-limited while troubleshooting, temporarily prepend `{ acme_ca https://acme-staging-v02.api.letsencrypt.org/directory }` to the `Caddyfile` to use the staging CA (untrusted cert, unlimited retries), then remove it once it works.

## Configuration

Environment variables (`.env` or shell):

| Variable | Default | Description |
|---|---|---|
| `BIND` | `127.0.0.1:3000` | Address the HTTP server listens on |
| `DATA_DIR` | `data` | Directory for SQLite database and uploads |
| `STATIC_DIR` | `static` | Directory for compiled frontend assets |
| `RUST_LOG` | `episteme=debug,...` | Log filter (tracing-subscriber syntax) |
| `EPISTEME_DOMAIN` | — | Public domain for the Caddy reverse proxy / Let's Encrypt cert (Docker HTTPS deploy only) |
| `EPISTEME_SHARED_DIR` | `./shared` | Host folder mounted read-only at `/data/shared` in the container — the path to give filesystem MCP servers. Keep it narrow; everything under it is readable by the AI |
| `AUTH_COOKIE_INSECURE` | — | Set (e.g. `1`) to allow the session cookie over plain HTTP. Needed for local `cargo run` / Vite dev; leave **unset** in production |
| `ANTHROPIC_API_KEY` | — | Optional pre-seeded key (can also be set in the UI) |
| `OPENAI_API_KEY` | — | Optional pre-seeded key (can also be set in the UI) |

Model providers, MCP servers, and the Microsoft 365 integration (email + calendar) are configured in the UI and stored in SQLite, not in `.env`. Microsoft 365 uses delegated OAuth — set it up under **Settings → Integrations** (the panel includes the exact Azure app-registration steps and Graph permissions). If you connected before calendar support was added, disconnect and reconnect to grant the `Calendars.ReadWrite` scope.

## Security

Episteme can hold API tokens and (eventually) execute shell commands. Treat it as a privileged admin tool:

- **Accounts are invite-only.** The first-run setup screen creates the admin; everyone else needs a single-use invite link from the admin — there is no open sign-up. Data is scoped per user (a member can never read another user's email, chats, or notes); instance-wide settings and the Logs window are admin-only. No password recovery — if the admin password is lost, delete the `auth_users`/`auth_sessions` rows to re-trigger setup. 2FA is not yet implemented (the toggle in Settings is a placeholder).
- **Do not expose it to a public network without TLS.** Terminate TLS at a reverse proxy for anything beyond localhost — the bundled Caddy service (see [Deploying with HTTPS](#deploying-with-https)) does this for you. The session cookie is `Secure` by default, so HTTPS is required for login to work (override with `AUTH_COOKIE_INSECURE` only for local http).
- **Tools auto-execute by default.** Flag individual tools as "ask first" under **Settings → Tools** — the chat then pauses with an approve/deny card before that tool runs (non-read-only MCP tools are marked "ask suggested"). Tools you haven't flagged act immediately. Only add MCP servers you trust: a stdio server is an arbitrary local process. Every tool execution, approval, and denial is recorded in the Logs window.
- **Keep secrets out of version control.** API keys and OAuth client secrets belong in `.env`/the SQLite DB (both gitignored), not committed.

## Architecture

See [`docs/Architecture.md`](docs/Architecture.md) for the full design doc: system diagram, component descriptions, data flow, security model, and the reasoning behind each technology choice.
