# 🦞 Claw

**Universal autonomous AI agent runtime.**

A single binary that runs on any device — Linux, macOS, Windows, Android (Termux), iOS (iSH) — as a fully autonomous AI agent with configurable guardrails, persistent memory, mesh networking, and a WASM plugin system.

---

## Install

### One-line install (recommended)

Works on **macOS, Linux, Windows (WSL), Android (Termux), iOS (iSH)**:

```bash
curl -sSL https://raw.githubusercontent.com/props-nothing/claw/main/scripts/install.sh | sh
```

This auto-detects your OS and architecture, downloads the right binary, adds it to `PATH`, and runs `claw init`.

### Docker (servers)

```bash
# Quick start
docker run -d --name claw -p 3700:3700 \
  -e ANTHROPIC_API_KEY=sk-ant-... \
  -v claw-data:/home/claw/.claw \
  ghcr.io/props-nothing/claw:latest

# Or with docker compose
git clone https://github.com/props-nothing/claw && cd claw
# Edit config/claw.toml with your API keys
docker compose up -d
```

### From source

```bash
# Requires Rust 1.80+
cargo install --path claw-bin
claw init
```

---

## Platform-specific guides

### 🍎 macOS

```bash
curl -sSL https://raw.githubusercontent.com/props-nothing/claw/main/scripts/install.sh | sh
source ~/.zshrc
claw start
```

### 🐧 Ubuntu / Debian Server

```bash
# Install
curl -sSL https://raw.githubusercontent.com/props-nothing/claw/main/scripts/install.sh | sh
source ~/.bashrc

# Edit config
nano ~/.claw/claw.toml
# Add your API key under [services]

# Run as systemd service (auto-start on boot)
sudo cp scripts/claw.service /etc/systemd/system/
sudo useradd -r -m -s /bin/bash claw  # create service user if needed
sudo systemctl daemon-reload
sudo systemctl enable --now claw

# Check logs
journalctl -u claw -f
```

### 🪟 Windows

**Option A — WSL (recommended):**

```bash
# In WSL terminal:
curl -sSL https://raw.githubusercontent.com/props-nothing/claw/main/scripts/install.sh | sh
```

**Option B — Native PowerShell:**

```powershell
# Install Rust from https://rustup.rs, then:
git clone https://github.com/props-nothing/claw
cd claw
cargo install --path claw-bin
claw init
claw start
```

### 📱 Android (Termux)

Install [Termux](https://f-droid.org/en/packages/com.termux/) from F-Droid, then:

```bash
# In Termux:
pkg update && pkg install curl
curl -sSL https://raw.githubusercontent.com/props-nothing/claw/main/scripts/install.sh | sh
source ~/.bashrc
claw start
```

If the pre-built binary doesn't work, the installer auto-falls back to building from source:

```bash
pkg install rust openssl git
# The install script handles the rest
```

### 📱 iOS (iSH)

Install [iSH](https://ish.app) from the App Store. iSH runs Alpine Linux, so Claw builds and runs natively:

```bash
# In iSH:
apk add curl bash
curl -sSL https://raw.githubusercontent.com/props-nothing/claw/main/scripts/install.sh | bash
source ~/.profile
claw start
```

For building from source in iSH:

```bash
apk add build-base openssl-dev rust cargo git
git clone https://github.com/props-nothing/claw && cd claw
cargo build --release --bin claw
cp target/release/claw /usr/local/bin/
claw init && claw start
```

> **Note:** iSH is slow for compilation (~30-60 min). Pre-built `aarch64-unknown-linux-musl` binaries are much faster to install.

### 🐳 Docker (any architecture)

```bash
# AMD64 (Intel/AMD servers)
docker pull ghcr.io/props-nothing/claw:latest

# ARM64 (Raspberry Pi, AWS Graviton, Apple Silicon)
docker pull ghcr.io/props-nothing/claw:latest

# The image is multi-arch — Docker picks the right one automatically
docker run -d --name claw -p 3700:3700 \
  -e ANTHROPIC_API_KEY=sk-ant-... \
  -v claw-data:/home/claw/.claw \
  ghcr.io/props-nothing/claw:latest
```

---

## Quick Start

```bash
# 1. Set your API key (pick one method):

# Option A: In config file (~/.claw/claw.toml)
#   [services]
#   anthropic_api_key = "sk-ant-..."

# Option B: Environment variable
export ANTHROPIC_API_KEY="sk-ant-..."

# 2. Start the agent
claw start

# 3. Or chat directly in terminal
claw chat
```

### Connect Telegram

Add to your `~/.claw/claw.toml`:

```toml
[channels.telegram]
type = "telegram"
token = "YOUR_BOT_TOKEN"
```

Then `claw start` — your bot is live.

---

## Architecture

```
              ┌─────────────┐
              │   Channels   │  ← Telegram, Discord, WhatsApp, Slack, WebChat
              └──────┬───────┘
                     │
              ┌──────────────┐
              │  Agent Loop  │  ← Receive → Recall → Think → Guard → Act → Remember → Respond
              └──────┬───────┘
                     │
         ┌───────────┼───────────┬──────────────┐
         ▼           ▼           ▼              ▼
    ┌─────────┐ ┌─────────┐ ┌────────┐  ┌───────────┐
    │   LLM   │ │ Memory  │ │ Plugins│  │   Mesh    │
    │ Router  │ │  Store  │ │  WASM  │  │  Network  │
    └─────────┘ └─────────┘ └────────┘  └───────────┘
```

## Autonomy Levels

| Level | Name       | Behavior                                                   |
| ----- | ---------- | ---------------------------------------------------------- |
| L0    | Manual     | Every action requires approval                             |
| L1    | Assisted   | Routine actions auto-approved, novel actions need approval |
| L2    | Supervised | Acts freely, sends periodic summaries                      |
| L3    | Autonomous | Pursues goals independently, escalates high-risk only      |
| L4    | Full Auto  | Fully self-directed within budget/scope constraints        |

## Configuration

All configuration lives in `claw.toml` (default: `~/.claw/claw.toml`):

```toml
[agent]
model = "anthropic/claude-sonnet-4-20250514"
thinking_level = "medium"

[services]
anthropic_api_key = "sk-ant-..."
# openai_api_key = "sk-..."

[autonomy]
level = 1
daily_budget_usd = 10.0
approval_threshold = 7

[channels.telegram]
type = "telegram"
token = "YOUR_BOT_TOKEN"

[mesh]
enabled = true
mdns = true
capabilities = ["shell", "browser", "camera"]
```

## Key Features

- **🧠 Three-tier memory**: Working (conversation), Episodic (past interactions), Semantic (knowledge + vector search)
- **🛡️ Guardrails**: Risk assessment, budget tracking, tool allow/deny lists, human-in-the-loop approval
- **🔌 WASM plugins**: Write plugins in any language, run them sandboxed with capability-gated access
- **🌐 Mesh networking**: Multiple devices form a swarm, delegate tasks to the best device
- **📱 Cross-platform**: Single Rust binary for Linux/macOS/Windows/Android/iOS
- **🤖 Goal planning**: Persistent goal stack with decomposition, execution, and self-reflection

## CLI Commands

```
claw start       Start the agent runtime
claw chat        Interactive terminal chat
claw status      Show runtime status
claw config      Show current configuration
claw plugin      Manage WASM plugins
claw doctor      Audit security configuration
claw init        Create a new claw.toml
```

## Project Structure

```
crates/
├── claw-core       Core types, traits, event bus
├── claw-config     Configuration system (claw.toml)
├── claw-llm        LLM provider abstraction (Anthropic, OpenAI, local)
├── claw-memory     Three-tier memory (working, episodic, semantic)
├── claw-autonomy   Autonomy levels, guardrails, goal planner
├── claw-plugin     WASM plugin host + registry
├── claw-channels   Channel adapters (Telegram, Discord, WebChat)
├── claw-mesh       P2P mesh networking (libp2p)
├── claw-runtime    Agent loop — ties everything together
├── claw-server     HTTP/WebSocket API server
├── claw-device     Device control (browser CDP, Android ADB, iOS simulator)
└── claw-cli        CLI interface
claw-bin/           Binary entry point
scripts/
├── install.sh      Universal install script
├── cross-build.sh  Cross-compile for all platforms
└── claw.service    Systemd unit file for Linux servers
```

## License

MIT
