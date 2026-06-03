# Episteme

A self-hosted AI workspace. One interface over local and cloud models, with an agentic tool-using loop and human-in-the-loop approvals for irreversible actions.

Your conversations and credentials stay on your machine. The only data that leaves it is what you explicitly route to a cloud provider you have configured.

## What's built

- **Streaming chat** — server-sent token streaming over a persistent axum/SSE backend, with keep-alive so slow local models don't drop the connection; assistant replies are persisted and survive a refresh
- **Sessions** — conversations persisted to SQLite; resume any past session
- **Model router** — route to Anthropic, OpenAI, Ollama, Gemini, Groq, DeepSeek, or any OpenAI-compatible endpoint from a single UI; switching is choosing a name from a list
- **Agent loop with native tools** — the model can call built-in tools (e.g. calendar management), inspect the result, and loop until it reaches a final answer; tool activity is surfaced live in the chat. Works with Ollama function-calling too
- **Persistent memory** — durable facts and preferences are auto-extracted from conversations (and addable by hand) and injected into future chats, so the assistant improves over time. The **Memories** window lets you view, filter, edit, and delete them
- **Email (Microsoft 365)** — folders, message list with search, reading pane, flagged / replied-to indicators, attachment viewer, AI-drafted replies/forwards, and **AI auto-sort** that files low-priority mail into folders and flags what needs attention
- **Calendar (Microsoft 365)** — an agenda view with manual add/delete, plus chat-driven scheduling: ask the AI to add appointments or reminders and it creates them via the calendar tools
- **Floating window workspace** — dockable/snappable windows (chat, email, calendar, memories, logs, settings); the layout and which windows were open are remembered across reloads
- **Logs** — a live, filterable log window fed by both frontend and backend events
- **Settings** — manage model providers, MCP servers, the Microsoft 365 connection, and auto-sort through the UI (no recompile required)
- **Docker** — multi-stage build produces a single static binary + assets; `docker compose up` for a self-contained deployment

## What's next

- **MCP host** — the `rmcp` subprocess transport is stubbed; native tools work today, wiring MCP adds third-party tool servers
- **Approval resumption** — the approval framework exists but the resume-a-paused-turn path is stubbed, so native tools currently auto-execute (see Security). A tokio channel to resume a paused turn would re-enable human-in-the-loop gating
- **Semantic memory** — memories are currently all injected (capped); relevance-based retrieval would scale to large stores
- **Notes / Tasks** — placeholder windows, not yet implemented

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

Open `http://localhost:3000`. Add a model provider in Settings (name, endpoint, API key) and start chatting.

**Dev mode** — run the backend and frontend separately for hot reload:

```sh
# Terminal 1: backend
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

## Configuration

Environment variables (`.env` or shell):

| Variable | Default | Description |
|---|---|---|
| `BIND` | `127.0.0.1:3000` | Address the HTTP server listens on |
| `DATA_DIR` | `data` | Directory for SQLite database and uploads |
| `STATIC_DIR` | `static` | Directory for compiled frontend assets |
| `RUST_LOG` | `episteme=debug,...` | Log filter (tracing-subscriber syntax) |
| `ANTHROPIC_API_KEY` | — | Optional pre-seeded key (can also be set in the UI) |
| `OPENAI_API_KEY` | — | Optional pre-seeded key (can also be set in the UI) |

Model providers, MCP servers, and the Microsoft 365 integration (email + calendar) are configured in the UI and stored in SQLite, not in `.env`. Microsoft 365 uses delegated OAuth — set it up under **Settings → Integrations** (the panel includes the exact Azure app-registration steps and Graph permissions). If you connected before calendar support was added, disconnect and reconnect to grant the `Calendars.ReadWrite` scope.

## Security

Episteme can hold API tokens and (eventually) execute shell commands. Treat it as a privileged admin tool:

- **Do not expose it to a public network without TLS.** Terminate TLS at a reverse proxy (Caddy, nginx, Traefik) for anything beyond localhost.
- **Native tools currently auto-execute.** The approval/resume flow is not yet wired, so when you ask the AI to create a calendar event (or auto-sort runs), it acts immediately. Every action is recorded in the Logs window, which is the audit surface for now. Re-enabling approval gating is on the roadmap.
- **Keep secrets out of version control.** API keys and OAuth client secrets belong in `.env`/the SQLite DB (both gitignored), not committed.

## Architecture

See [`docs/Architecture.md`](docs/Architecture.md) for the full design doc: system diagram, component descriptions, data flow, security model, and the reasoning behind each technology choice.
