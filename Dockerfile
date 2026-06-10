# syntax=docker/dockerfile:1

# ── Stage 1: Build frontend ─────────────────────────────────────────────────
FROM node:22-alpine AS frontend-builder
WORKDIR /app/frontend

COPY frontend/package.json frontend/package-lock.json* ./
RUN npm ci --legacy-peer-deps

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
# node base (rather than bare debian) so stdio MCP servers launched as
# `npx -y <package>` work out of the box; uv/uvx covers Python-based ones.
FROM node:22-bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3 curl libicu72 && \
    rm -rf /var/lib/apt/lists/*

# PowerShell for the in-app PowerShell terminal (bash ships with the base image).
# Pinned tarball install; x64 to match the amd64 runtime.
ARG PWSH_VERSION=7.4.6
RUN curl -fsSL -o /tmp/pwsh.tar.gz \
      "https://github.com/PowerShell/PowerShell/releases/download/v${PWSH_VERSION}/powershell-${PWSH_VERSION}-linux-x64.tar.gz" && \
    mkdir -p /opt/microsoft/powershell/7 && \
    tar zxf /tmp/pwsh.tar.gz -C /opt/microsoft/powershell/7 && \
    chmod +x /opt/microsoft/powershell/7/pwsh && \
    ln -s /opt/microsoft/powershell/7/pwsh /usr/bin/pwsh && \
    rm /tmp/pwsh.tar.gz

# Exchange Online / Security & Compliance PowerShell for O365 admin from the
# terminal (device-code interactive auth). Installed AllUsers so spawned shells
# see it. Single-quoted so the shell doesn't touch pwsh's syntax; version pinned
# for reproducible builds. The Microsoft.Graph submodules (Connect-MgGraph, mail,
# users, groups, directory) cover the common admin tasks without the ~1.5GB full
# meta-module — all pinned to the same version so the submodules stay compatible.
RUN pwsh -NoLogo -NonInteractive -Command 'Set-PSRepository -Name PSGallery -InstallationPolicy Trusted; \
    Install-Module -Name ExchangeOnlineManagement -RequiredVersion 3.5.1 -Scope AllUsers -Force -AllowClobber; \
    $gv = "2.25.0"; \
    foreach ($m in @("Microsoft.Graph.Authentication","Microsoft.Graph.Users","Microsoft.Graph.Mail","Microsoft.Graph.Groups","Microsoft.Graph.Identity.DirectoryManagement")) { \
        Install-Module -Name $m -RequiredVersion $gv -Scope AllUsers -Force -AllowClobber \
    }'

COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /usr/local/bin/

# Keep package caches on the data volume so npx/uvx MCP servers don't
# re-download their packages every container restart.
ENV npm_config_cache=/data/npm-cache
ENV UV_CACHE_DIR=/data/uv-cache

WORKDIR /app

COPY --from=backend-builder /app/target/release/episteme ./
COPY --from=frontend-builder /app/backend/static ./static

EXPOSE 3000
ENV BIND=0.0.0.0:3000
ENV DATA_DIR=/data
ENV STATIC_DIR=/app/static
VOLUME /data

CMD ["./episteme"]
