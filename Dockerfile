# syntax=docker/dockerfile:1

# ── Stage 1: Build frontend ─────────────────────────────────────────────────
FROM node:22-alpine AS frontend-builder
WORKDIR /app/frontend

COPY frontend/package.json frontend/package-lock.json* ./
RUN npm ci

COPY frontend/ .
# Output goes to ../backend/static relative to /app/frontend → /app/backend/static
RUN npm run build

# ── Stage 2: Build backend ───────────────────────────────────────────────────
FROM rust:1.88-slim AS backend-builder
WORKDIR /app

RUN apt-get update && \
    apt-get install -y --no-install-recommends libssl-dev pkg-config && \
    rm -rf /var/lib/apt/lists/*

# Cache dependency compilation separately from application code.
COPY Cargo.toml Cargo.lock ./
COPY backend/Cargo.toml backend/Cargo.toml
RUN mkdir -p backend/src && \
    echo 'fn main() {}' > backend/src/main.rs && \
    cargo build --release 2>/dev/null; \
    rm -rf backend/src

# Build the real application.
COPY backend/ backend/
# sqlx migrate! macro needs the migration files at build time.
# Touch main.rs so Cargo knows it changed.
RUN touch backend/src/main.rs && cargo build --release

# ── Stage 3: Runtime ────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=backend-builder /app/target/release/episteme ./
COPY --from=frontend-builder /app/backend/static ./static

EXPOSE 3000
ENV BIND=0.0.0.0:3000
ENV DATA_DIR=/data
ENV STATIC_DIR=/app/static
VOLUME /data

CMD ["./episteme"]
