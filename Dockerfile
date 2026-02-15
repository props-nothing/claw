# 🦞 Claw — Universal Autonomous AI Agent Runtime
# Multi-stage build for minimal image size.
# Supports linux/amd64 and linux/arm64 via Docker Buildx.
#
# Build:
#   docker build -t claw .
#
# Multi-arch:
#   docker buildx build --platform linux/amd64,linux/arm64 -t claw .
#
# Run:
#   docker run -d --name claw -p 3700:3700 \
#     -e ANTHROPIC_API_KEY=sk-ant-... \
#     -v claw-data:/home/claw/.claw \
#     claw

# ── Builder stage ───────────────────────────────────────────────
FROM rust:1.93-bookworm AS builder

WORKDIR /build

# Install dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies — copy manifests first, build a dummy, then copy real source
COPY Cargo.toml Cargo.lock ./
COPY claw-bin/Cargo.toml claw-bin/
COPY crates/ crates/

# Build
RUN cargo build --release --bin claw \
    && strip target/release/claw

# ── Runtime stage ───────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    git \
    nodejs \
    npm \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -s /bin/bash claw

# Copy binary
COPY --from=builder /build/target/release/claw /usr/local/bin/claw

# Copy default config + web UI
COPY --chown=claw:claw config/claw.toml /home/claw/.claw/claw.toml
COPY --chown=claw:claw web/ /home/claw/.claw/web/

# Create data directories
RUN mkdir -p /home/claw/.claw/screenshots /home/claw/.claw/plugins /home/claw/.claw/skills \
    && chown -R claw:claw /home/claw/.claw

USER claw
WORKDIR /home/claw

ENV CLAW_CONFIG=/home/claw/.claw/claw.toml

EXPOSE 3700

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
    CMD curl -f http://localhost:3700/health || exit 1

ENTRYPOINT ["claw"]
CMD ["start"]
