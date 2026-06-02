# Episteme

A self-hosted AI workspace. One interface over local and cloud models, with an agentic tool-using loop and human-in-the-loop approvals for irreversible actions.

Your conversations and credentials stay on your machine. The only data that leaves it is what you explicitly route to a cloud provider you have configured.

## What's built

- **Streaming chat** — server-sent token streaming over a persistent axum/SSE backend
- **Sessions** — conversations persisted to SQLite; resume any past session
- **Model router** — route to Anthropic, OpenAI, Ollama, vLLM, or any OpenAI-compatible endpoint from a single UI; switching is choosing a name from a list
- **Agent loop** — the model can call tools, inspect results, and loop until it reaches a final answer
- **Approvals** — mutating tool calls are paused and surfaced for explicit user confirmation before they execute; read-only tools run immediately
- **Settings** — manage model providers and MCP servers through the UI (no recompile required)
- **Docker** — multi-stage build produces a single static binary + assets; `docker compose up` for a self-contained deployment

## What's next

- **MCP host** — the `rmcp` subprocess transport is stubbed; wiring it completes the tool-use path (Phase 3)
- **Email integration** — read + draft MCP server, inbox triage and reply drafting (Phase 4)
- **Approval resumption** — tokio channel to resume a paused agent turn seamlessly without a follow-up request (Phase 5)

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

Model providers and MCP servers are configured in the UI and stored in SQLite, not in `.env`.

## Security

Episteme can hold API tokens and (eventually) execute shell commands. Treat it as a privileged admin tool:

- **Do not expose it to a public network without TLS.** Terminate TLS at a reverse proxy (Caddy, nginx, Traefik) for anything beyond localhost.
- **Auth is on by default.** Never disable it.
- **Mutations require approval.** No tool that changes external state runs without you confirming it first.
- **Keep secrets out of version control.** API keys belong in `.env` (gitignored), not committed.

## Architecture

See [`docs/Architecture.md`](docs/Architecture.md) for the full design doc: system diagram, component descriptions, data flow, security model, and the reasoning behind each technology choice.
