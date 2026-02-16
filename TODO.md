# 🦞 Claw — TODO

> Organized by priority. Updated 2026-02-16.
> **Goal: Beat every competitor — OpenClaw (201k★), ZeroClaw (6.8k★), PicoClaw (13.3k★) — on features, performance, and developer experience.**

---

## Legend

| Tag         | Meaning                    |
| ----------- | -------------------------- |
| ✅ **DONE** | Fully working, tested      |
| 🟡 **STUB** | Code exists but incomplete |
| 🔴 **TODO** | Not started                |

---

## ✅ Completed (for reference)

<details>
<summary>Click to expand completed items</summary>

### Core Runtime

- [x] Agent loop — receive→recall→think→guard→act→remember→respond
- [x] Auto-continuation on `max_tokens` (inject system message + loop)
- [x] Lazy stop detection — re-prompt when model cops out with "you can customize..."
- [x] Wall-clock timeout per request (`request_timeout_secs`, default 300s)
- [x] Per-session run locks (`SessionManager::run_lock()`) — prevents interleaving
- [x] Model fallback in agent loop — switch to `fallback_model` after 3 LLM failures
- [x] Concurrent message processing via `SharedAgentState` + `tokio::spawn`

### LLM Providers

- [x] OpenAI — complete + stream (SSE parser, tool call accumulation, usage tracking)
- [x] Anthropic — complete + stream + extended thinking
- [x] Ollama/Local — complete + stream
- [x] Model router with failover
- [x] Retry with exponential backoff (3 attempts, 1s/2s/4s)
- [x] Circuit breaker (5 failures → 60s cool-off → half-open probe)
- [x] Cost estimation per model

### Tools (38 builtin + 45 device = 83 total)

- [x] `shell_exec` — non-interactive shell commands
- [x] `file_read`, `file_write`, `file_list`, `file_edit`, `file_find`, `file_grep`
- [x] `apply_patch` — multi-file search-and-replace
- [x] `process_start`, `process_list`, `process_kill`, `process_output`
- [x] `terminal_open`, `terminal_run`, `terminal_view`, `terminal_input`, `terminal_close` (real PTY)
- [x] `memory_store`, `memory_search`, `memory_delete`, `memory_list`
- [x] `goal_create`, `goal_list`, `goal_complete_step`, `goal_update_status`
- [x] `web_search`, `http_fetch`, `llm_generate`
- [x] `mesh_peers`, `mesh_delegate`, `mesh_status`
- [x] `channel_send_file` — send files/images through channel adapters
- [x] `sub_agent_spawn`, `sub_agent_wait`, `sub_agent_status` — sub-agent spawning with isolated context
- [x] `cron_schedule`, `cron_list`, `cron_cancel` — scheduled/recurring tasks

### Web UI

- [x] Dark-themed SPA (vanilla JS, no build step)
- [x] 8 pages: Dashboard, Chat, Sessions, Goals, Memory, Tools, Logs, Settings
- [x] SSE streaming chat with tool calls (collapsible) + approval buttons
- [x] Session resume from localStorage, clickable session rows
- [x] Auto-scroll, markdown rendering, status polling
- [x] Web assets embedded in binary via `rust-embed` (no external files needed)
- [x] Inline screenshot rendering — auto-expanded image previews in tool results + chat, click to open full-size

### Device Control (`claw-device` crate)

- [x] **Browser automation** — Chrome DevTools Protocol (CDP) via native `tokio-tungstenite` WebSocket, headless/headed (1920×1080 viewport), navigate/click/type/screenshot/evaluate JS/DOM snapshot/scroll/file upload/PDF export, tab management
- [x] **Android control** — ADB bridge: list devices, screenshot, tap/swipe/type, key events, shell commands, install/launch/stop apps, UI dump, push/pull files
- [x] **iOS control** — simctl + AppleScript (tap/type/buttons) + idb fallback: list devices/simulators, screenshot, tap/swipe/type, button presses (Simulator keyboard shortcuts), install/launch/terminate apps, boot/shutdown simulators, open URLs
- [x] **44 device tools** exposed to LLM: 14 browser + 15 Android + 15 iOS tools, fully integrated into agent dispatch
- [x] **Screenshots saved to disk** — `~/.claw/screenshots/`, served via `/api/v1/screenshots/{file}` (no base64 in LLM context)

### Sessions

- [x] Session creation with channel/target routing
- [x] `get_or_insert` — resume by UUID, backfill channel/target
- [x] `record_message` — increment count on user + assistant messages
- [x] `set_name` — auto-label from first user message
- [x] SQLite persistence (60s background flush)
- [x] Session restore on startup (only sessions with messages)
- [x] `cleanup_empty_sessions` on startup
- [x] `/api/v1/sessions/{id}/messages` endpoint with SQLite fallback

### Memory

- [x] Working memory — per-session context with LLM-powered auto-compaction
- [x] Episodic memory — record after each turn, keyword search, SQLite persist + load on startup
- [x] Semantic memory — fact store with vector search, embedding generation, SQLite persist + load
- [x] Session messages — JSON blob persistence to SQLite
- [x] Context window auto-detect per model
- [x] Tool result truncation (first 60% + last 20%)
- [x] Overflow recovery — force compaction → retry on context error
- [x] **Memory deletion** — `memory_delete` tool, per-fact and per-category removal from semantic memory + SQLite
- [x] **Memory listing** — `memory_list` tool, browse all stored facts with category counts
- [x] **Multi-strategy search** — RECALL combines vector search + keyword search + extracted-keyword search with deduplication
- [x] **Word-level scored search** — semantic search scores facts by matching words across category+key+value (replaces substring match)
- [x] **Keyword extraction** — strips English+Dutch stop words, preserves domain terms/URLs for better recall

### Self-Learning & Intelligence

- [x] **Operator trust prompt** — system prompt instructs model it's authorized to use user-provided credentials
- [x] **Automatic lesson extraction** — detects error→correction→success patterns in conversations
- [x] **LLM-powered lesson summarization** — extracts structured lessons with trigger/mistake/correction/rule
- [x] **Learned lessons recall** — `learned_lessons` category auto-loaded during RECALL phase
- [x] **Self-learning instructions** in system prompt — guides agent to store and apply lessons

### Autonomy & Security

- [x] 5 autonomy levels (L0–L4)
- [x] 3 guardrail rules + allow/deny lists
- [x] Budget tracker (daily USD + per-loop tool calls)
- [x] Approval flow — API, Web UI, Telegram inline keyboards, CLI
- [x] Per-IP rate limiting (token bucket, 60 burst, 10/sec refill)

### Goal Planner

- [x] Full lifecycle: create→plan→execute→complete/fail
- [x] Sub-goals, progress tracking, retrospective notes
- [x] SQLite persistence + load on startup
- [x] Delegation to mesh peers
- [x] `goal_complete_step` and `goal_update_status` tools

### Mesh Networking

- [x] libp2p: TCP+Noise+Yamux, GossipSub, mDNS, Identify, Kademlia
- [x] Task delegation with capability routing
- [x] Memory sync via SyncDelta
- [x] Pending task tracking with oneshot channels
- [x] 3 LLM tools + CLI commands + API endpoints

### Skills

- [x] TOML-based skill definitions with topological executor
- [x] 4 built-in skills: `summarize_url`, `research_topic`, `code_review`, `daily_briefing`
- [x] CLI: `claw skill list|show|run|create|delete`
- [x] Skills exposed as `skill.*` tools to LLM

### Channels

- [x] **Telegram** — long-polling, Markdown+fallback, photo upload, typing indicators, inline keyboard approvals, 409 conflict detection
- [x] **Discord** — Gateway WebSocket, REST API send, rich embeds, button approvals (524 LOC)
- [x] **Slack** — Events API / Socket Mode, OAuth2, Block Kit, threads (510 LOC)
- [x] **WhatsApp** — Business Cloud API, webhook verification, templates, media support (1,514 LOC)
- [x] **Signal** — signal-cli JSON-RPC mode, send/receive, attachment support (324 LOC)
- [x] **WebChat** — bridge adapter for Web UI

### Sub-Agent System

- [x] `sub_agent_spawn` — spawn isolated agent with limited tools and scoped memory
- [x] `sub_agent_wait` — collect result from completed sub-agent task
- [x] `sub_agent_status` — check sub-agent task progress
- [x] Task-scoped memory (sub-agent gets subset of parent context)
- [x] Result aggregation back to parent via goal planner integration

### Scheduled Tasks

- [x] `cron_schedule` — schedule one-shot or recurring tasks via cron expressions
- [x] `cron_list` — list active scheduled tasks
- [x] `cron_cancel` — cancel a scheduled task
- [x] Scheduler engine with auto-resume on restart

### Server

- [x] 22 API routes on Axum (includes screenshots, sub-tasks, scheduled-tasks, events SSE)
- [x] Bearer auth middleware, CORS
- [x] Prometheus metrics (16 counters)
- [x] Static file serving for Web UI
- [x] Screenshot serving endpoint (PNG from `~/.claw/screenshots/`)

### WASM Plugins

- [x] wasmtime with fuel-limited execution (10M fuel)
- [x] Plugin ABI (`claw_malloc` + `claw_invoke`)
- [x] Manifest parsing, BLAKE3 checksums
- [x] `claw plugin create` scaffold generator
- [x] `claw plugin uninstall` removes directory
- [x] Feature-gated behind `wasm` cargo feature

### Docker

- [x] Multi-stage Dockerfile with `rust:1.93-bookworm`
- [x] docker-compose.yml with volume mounts and env vars

### Telegram

- [x] Long-polling with timeouts, exponential backoff, dead-receiver detection
- [x] Send (Markdown + plain text fallback)
- [x] Photo upload via `sendPhoto` multipart (screenshot URLs + absolute disk paths)
- [x] Typing indicators
- [x] Inline keyboard approval prompts + callback queries
- [x] `/start`, `/help`, `/status`, `/new`, `/approve`, `/deny` commands
- [x] 409 Conflict detection (duplicate bot instances) with auto-stop

### Config

- [x] TOML schema with env overrides
- [x] Hot-reload file watcher (notify)
- [x] `claw config set key value` CLI
- [x] 20+ validation checks
- [x] Context window / compaction / timeout config

### CLI

- [x] 17 commands: start, chat, status, version, config, set, plugin, logs, doctor, init, setup, completions, skill, hub, mesh, channels, help
- [x] Shell completions (bash/zsh/fish)
- [x] `claw logs` with color-coding, type filter, JSON output

### Testing & CI

- [x] 182 tests across 13 crates
- [x] Mock LLM provider
- [x] GitHub Actions CI (check, test, clippy, fmt, release builds)

</details>

---

## 🔴 Priority 0 — Performance (from deep audit 2026-02-16)

> These issues were identified during a comprehensive performance review.
> Binary: 8.0 MB, startup <1ms, peak RSS ~9.6 MB. All 182 tests pass, 0 clippy warnings.

### P0.1 ✅ DONE — Blocking `hostname` in async context

Cached hostname in a `LazyLock<String>` in `agent.rs`. No longer blocks a tokio worker thread.

- [x] Cache hostname in a `LazyLock<String>`

### P0.2 ✅ DONE — New `reqwest::Client` per web search

Added `http_client: reqwest::Client` to `SharedAgentState`. `web_search` and `http_fetch` now reuse connections.

- [x] Shared `Client` on `SharedAgentState`

### P0.3 ✅ DONE — `all_tools.clone()` per agent loop iteration

Changed `LlmRequest.tools` from `Vec<Tool>` to `Arc<Vec<Tool>>`. Tools are built once and Arc-cloned (pointer copy) per iteration instead of deep-cloning ~83 tool definitions.

- [x] `Arc<Vec<Tool>>` in `LlmRequest` — updated all 16 construction sites across provider.rs, mock.rs, router_tests.rs, agent_loop.rs, learning.rs, tool_dispatch.rs

### P0.4 ✅ DONE — Global `Mutex<MemoryStore>` concurrency bottleneck

Replaced `Arc<TokioMutex<MemoryStore>>` with `Arc<TokioRwLock<MemoryStore>>`. 55 call sites updated: 29 read locks + 26 write locks. Concurrent read-only operations (search, list, messages) no longer block each other.

- [x] `TokioRwLock` for memory — 55 call sites across agent.rs, agent_loop.rs, tool_dispatch.rs, query.rs, learning.rs, sub_agent.rs

### P0.5 ✅ DONE — Duplicated streaming / non-streaming agent loops (~1000 LOC)

`process_message_shared` now delegates to `process_message_streaming_shared` with a sink channel, collecting `TextDelta` events into the final string. Eliminated ~720 LOC of duplication.

- [x] Non-streaming path creates `mpsc::channel`, calls streaming path, collects text deltas

### P0.6 🟡 Partially resolved — `messages.to_vec()` clones full conversation every iteration

The duplicate non-streaming path was eliminated by P0.5, halving the clone sites. The remaining clones in the streaming loop (2 sites) are inherent to the RwLock read-then-release pattern — the lock must be released before the async LLM call.

- [x] Duplicate clone path eliminated (P0.5)
- [ ] Remaining: `Arc<Vec<Message>>` or COW for the 2 streaming-loop sites (diminishing returns)

### P0.7 ✅ DONE — SSE stream buffer reallocated on every line

Replaced `buffer = buffer[newline_pos + 1..].to_string()` with `buffer.drain(..newline_pos + 1)` in all 3 SSE parsers.

- [x] `buffer.drain()` in anthropic.rs, openai.rs, local.rs

### P0.8 ✅ DONE — SQLite `prepare()` instead of `prepare_cached()`

Replaced 8 `prepare()` call sites with `prepare_cached()` in store.rs and episodic.rs. SQLite now reuses compiled statement plans.

- [x] `prepare_cached()` in 8 call sites across store.rs and episodic.rs

### P0.9 ✅ DONE — N+1 queries in goal loading

Replaced N+1 queries with a single `SELECT g.*, s.* FROM goals LEFT JOIN goal_steps` query. Results grouped in Rust with a `HashMap`. 20 goals now = 1 query instead of 21.

- [x] Single JOIN query with Rust-side grouping in `store.rs`

### P0.10 ✅ DONE — Full sort in vector search replaced with partial sort

When results > top_k, uses `select_nth_unstable_by` to partition the top-k elements in O(n), then sorts only those k elements. Full O(n log n) sort only used when results ≤ top_k.

- [x] `select_nth_unstable_by` partial sort in `semantic.rs`
- [ ] For >10K facts, integrate ANN index (HNSW via `usearch`) — deferred, not needed at current scale

### P0.11 🟡 Moderate — JSON wire format for mesh messages

GossipSub messages serialized as JSON. 2-3× overhead vs binary.

- [ ] Switch to `bincode` or `postcard` for mesh wire format

### P0.12 🟡 Moderate — GossipSub for directed messages (floods all peers)

Directed messages published to topic where **all** peers receive them. Non-target peers filter and discard.

- [ ] Use libp2p `request-response` protocol for point-to-point delivery
- [ ] Reserve GossipSub for broadcasts only

### P0.13 ✅ DONE — WASM pre-linked at load time (`InstancePre`)

`LoadedPlugin` now stores an `InstancePre<()>` instead of a raw `Module`. The `Linker` is created once at plugin load time via `linker.instantiate_pre(&module)`. Each `execute_wasm()` call only needs `Store::new` + `instance_pre.instantiate_async()` — no Linker per call.

- [x] `InstancePre` in `LoadedPlugin`, pre-linked at load time in `host.rs`

### P0.14 🟡 Moderate — `serde_json::json!` macro for LLM request bodies

Double serialization: Rust structs → Value tree → JSON string.

- [ ] Define typed request structs and serialize directly

### P0.15 ✅ DONE — `Vec::remove(0)` in episodic buffer

Replaced `Vec<Episode>` with `VecDeque<Episode>` in episodic.rs. `remove(0)` → `pop_front()`, `push` → `push_back()`.

- [x] `VecDeque` for episodic memory in episodic.rs

### P0.16 🟢 Minor — `Uuid::to_string()` allocations in hot paths

Each call allocates a 36-byte `String`.

- [ ] Use `Uuid` directly as HashMap keys (implements `Hash`)
- [ ] Use `as_bytes()` for SQL BLOB columns

### P0.17 ✅ DONE — EventBus default capacity 4096 → 256

Reduced from 4096 to 256 in `event.rs`. Saves ~30 KB of pre-allocated ring buffer memory per EventBus instance.

- [x] `Self::new(256)` in `event.rs` Default impl

### P0.18 ✅ DONE — WASM AOT compilation cache

After `Module::new()`, serializes the compiled artifact to `.cache/{name}-{hash}.cwasm` (keyed by BLAKE3 hash of WASM bytes). On subsequent loads, `Module::deserialize()` loads the pre-compiled native code. Cache auto-invalidates when WASM content changes.

- [x] `Module::serialize()` / `Module::deserialize()` with content-addressed cache in `host.rs`

---

## 🔴 Priority 1 — Crush the Competition

> These items close every gap with OpenClaw/ZeroClaw/PicoClaw and establish clear leads.

### 1.1 LLM Providers — Match ZeroClaw's 22+ (Currently: 3)

Claw has only OpenAI + Anthropic + Ollama. ZeroClaw supports 22+, PicoClaw supports 7. This is the #1 gap.

- [ ] **OpenRouter provider** — single API key → 200+ models (Claude, GPT, Gemini, Llama, Mixtral, etc.) — _closes the gap overnight_
- [ ] **Gemini provider** — Google AI Studio direct API, generous free tier, multimodal
- [ ] **DeepSeek provider** — DeepSeek-V3/R1 direct API, massive context windows, cheap
- [ ] **Groq provider** — ultra-fast inference (Llama, Mixtral, Whisper transcription)
- [ ] **xAI/Grok provider** — Grok-2/3 API
- [ ] **Mistral provider** — Mistral Large/Codestral direct API
- [ ] **Together AI provider** — open-source model hosting
- [ ] **Fireworks AI provider** — fast open-source inference
- [ ] **Bedrock provider** — AWS Bedrock for enterprise (Claude, Titan, Llama)
- [ ] **Cohere provider** — Command R+ with RAG
- [x] ~~**OpenAI-compatible generic provider**~~ — 🟡 STUB: `OpenAiProvider::with_base_url()` exists, accepts any OpenAI-compatible endpoint. Needs config wiring (`[providers.custom]` section) so users don't need code changes.
- [ ] **Perplexity provider** — search-augmented LLM
- [x] ~~Provider auto-detection from model name prefix~~ — ✅ DONE: `anthropic/`, `openai/`, `ollama/`, `local/` prefixes already routed in `start.rs`. Extend for `google/`, `deepseek/`, `groq/`, etc.
- [ ] Provider health dashboard in Web UI

### 1.2 Voice & Speech — Match OpenClaw (Currently: None)

OpenClaw has Voice Wake + Talk Mode + ElevenLabs. Nobody else in Rust has this.

- [ ] **Speech-to-text (STT)** — Whisper API (OpenAI + Groq + local whisper.cpp via `cpal` audio capture)
- [ ] **Text-to-speech (TTS)** — ElevenLabs + OpenAI TTS + edge-tts (free Microsoft voices)
- [ ] **Voice mode in Web UI** — push-to-talk + continuous listening toggle
- [ ] **Voice mode in Telegram** — auto-transcribe voice messages via Whisper, respond with voice notes
- [ ] **Wake word detection** — local hotword engine (Porcupine or Rust-native)
- [ ] **Talk Mode overlay** — persistent bidirectional voice conversation (like OpenClaw's Talk Mode)
- [ ] Voice activity detection (VAD) for auto-segmenting speech

### 1.3 Onboarding & Setup — ✅ Mostly Done

- [x] ~~**`claw setup` interactive wizard**~~ — ✅ DONE (950 LOC in `setup.rs`): 6-step dialoguer TUI wizard — Model → Channels → Autonomy → Services → Mesh → Server. Includes WhatsApp QR linking, multi-channel selection, auto-install of 8 bundled skills.
- [x] ~~**`claw doctor`**~~ — ✅ DONE (`cmd_doctor` in `mod.rs`): config file validation, denylist checks, API key checks.
- [ ] **`claw channel doctor`** — per-channel health check with actionable fix suggestions (currently `claw channels status` exists but lacks fix suggestions)
- [ ] **First-run tutorial** — guided first conversation with explanation of capabilities
- [ ] **Config migration** — auto-detect OpenClaw/ZeroClaw configs and import

### 1.4 Security Hardening — Beat ZeroClaw (Currently: No encryption, no sandbox)

ZeroClaw has encrypted secrets, Docker sandbox, gateway pairing, tunnel integration. Close every gap.

- [ ] **Secret encryption at rest** — encrypt API keys in `claw.toml` with local key file (AES-256-GCM via `aead` crate)
- [ ] **`claw secrets encrypt`** — CLI to encrypt/decrypt config secrets
- [ ] **Docker sandbox runtime** — `runtime.kind = "docker"` for tool execution in disposable containers (like ZeroClaw)
  - [ ] Configurable image, memory limit, CPU limit, network mode, read-only rootfs
  - [ ] Per-session sandbox isolation (like OpenClaw)
- [ ] **Gateway pairing** — 6-digit one-time code on first connect, exchange for bearer token (like ZeroClaw)  
      _Note: WhatsApp channel already has DM policy with pairing/allowlist/open/disabled modes + PairingRequest system. Generalize to all channels._
- [ ] **Tunnel integration** — built-in support for:
  - [ ] Tailscale Serve/Funnel (auto-configure)
  - [ ] Cloudflare Tunnel
  - [ ] ngrok
  - [ ] Custom tunnel command
- [ ] **Filesystem scoping** — `workspace_only = true` mode restricts all file tools to workspace (like ZeroClaw)
- [ ] **Forbidden paths** — configurable deny list for dangerous paths (`/etc`, `/root`, `~/.ssh`, etc.)
- [ ] **Command allowlist/denylist** — restrict which shell commands the agent can execute  
      _Note: Tool-level allowlist/denylist already exists (`tool_allowlist`/`tool_denylist` in config + `GuardrailEngine`). This is about filtering individual shell commands within `shell_exec`._
- [ ] **Dangerous command detection** — block `rm -rf /`, `dd if=`, fork bombs, etc.  
      _Note: 3 guardrails exist (`RiskLevelGuardrail`, `DestructiveActionGuardrail`, `NetworkExfiltrationGuardrail`) but they operate on tool metadata, not raw shell command strings._
- [ ] **Symlink escape detection** — canonicalize paths, reject escapes from workspace
- [ ] **Audit log cryptographic signing** — HMAC-SHA256 instead of DefaultHasher

### 1.5 Tunnel & Remote Access — Match OpenClaw + ZeroClaw (Currently: None)

OpenClaw has Tailscale Serve/Funnel. ZeroClaw supports 4 tunnel providers. Claw has nothing.

- [ ] **`[tunnel]` config section** — `provider = "tailscale" | "cloudflare" | "ngrok" | "custom" | "none"`
- [ ] **Auto-start tunnel** on `claw start` when configured
- [ ] **Refuse public bind** without tunnel (security default like ZeroClaw)
- [ ] **Remote gateway access** — expose Web UI + API securely over tunnel with auth
- [ ] **`claw tunnel status`** — show tunnel URL, connected clients

### 1.6 Identity & Persona System — Match ZeroClaw (Currently: None)

ZeroClaw supports AIEOS identity + OpenClaw-style markdown. PicoClaw has IDENTITY.md + SOUL.md. Claw has nothing.

- [ ] **Workspace identity files** — `IDENTITY.md`, `SOUL.md`, `USER.md`, `AGENTS.md`, `TOOLS.md` in `~/.claw/workspace/`
- [ ] **AIEOS v1.1 support** — import/export portable AI identity (JSON schema)
- [ ] **Per-session persona override** — switch identity per channel or session
- [ ] **`claw identity create`** — interactive persona builder
- [ ] **`claw identity export`** — export to AIEOS JSON for portability

### 1.7 Heartbeat / Periodic Tasks — 🟡 Partially Done

ZeroClaw has HEARTBEAT.md, PicoClaw has heartbeat with subagent spawning. Claw has cron + heartbeat_cron config + scheduler integration.

- [x] ~~**Configurable heartbeat cron**~~ — ✅ DONE: `heartbeat_cron` in `[autonomy]` config, loaded by `scheduler.rs`, executed by runtime on startup.
- [ ] **HEARTBEAT.md** — periodic task file in workspace, agent reads every N minutes
- [ ] **Heartbeat + subagent integration** — spawn subagents for long-running heartbeat tasks
- [ ] **Heartbeat status in Web UI** — show last run, next run, task history

### 1.8 More Channels — Beat Everyone

OpenClaw leads with 14+ channels. Add the remaining to match and exceed.

- [ ] **iMessage** — BlueBubbles integration (API + webhook) for macOS iMessage bridge
- [ ] **Matrix** — `matrix-sdk` crate, room-based conversations, E2EE
- [ ] **Microsoft Teams** — Bot Framework REST API
- [ ] **Google Chat** — Chat API with service account
- [ ] **LINE** — LINE Messaging API + webhook
- [ ] **QQ** — QQ Official/OpenQQ API
- [ ] **DingTalk/Feishu/Lark** — enterprise IM APIs
- [ ] **Zalo** — Zalo Official Account API
- [ ] **Webhook channel** — generic inbound/outbound webhook adapter (any service)

---

## 🟡 Priority 2 — Establish Clear Leads (Unique Advantages)

> Double down on things only Claw can do. Make these hero features.

### 2.1 Mesh Networking — Nobody Else Has This

Claw is the ONLY project with P2P mesh. Make it the killer feature.

- [ ] Switch to `bincode` or `postcard` for mesh wire format (currently JSON, 2-3× overhead)
- [ ] Use libp2p `request-response` for point-to-point messages (currently GossipSub floods all peers)
- [ ] **Mesh dashboard in Web UI** — live peer map, message flow visualization, capability matrix
- [ ] **Cross-device agent coordination demo** — multi-node task execution with video walkthrough
- [ ] **Mesh auto-discovery showcase** — zero-config LAN discovery with mDNS
- [ ] **Mesh security** — per-peer capability authorization, peer ban list
- [ ] **Mesh relay** — NAT traversal via relay peers for WAN connectivity

### 2.2 Device Control — 83 Tools, Best in Class

Nobody else has native CDP + ADB + simctl in a single binary. Showcase it.

- [ ] **Computer Use mode** — full desktop automation via screenshots + coordinate clicking (like Anthropic's computer use)
- [ ] **Desktop automation** — native macOS (AppleScript/Accessibility) + Linux (xdotool/ydotool) + Windows (UI Automation)
- [ ] **Screen recording** — capture video of agent actions for audit/replay
- [ ] **Multi-browser support** — Firefox (via Marionette), Safari (via WebDriver)
- [ ] **Browser profiles** — persistent sessions, cookie management, authenticated browsing
- [ ] **Device tool gallery in Web UI** — live device status, screenshot preview, action history

### 2.3 Memory — Already Best, Make It Untouchable

Claw's 3-tier memory + self-learning is unique. Extend the lead.

- [ ] **Memory graph visualization** in Web UI — show connections between facts, categories, lessons
- [ ] **Memory export/import** — JSON/SQLite dump for backup and migration
- [x] ~~**Cross-session memory**~~ — ✅ DONE: Semantic memory (facts) is stored globally, not session-scoped. Learned lessons are in `learned_lessons` category accessible to all sessions. Episodic memory is keyed by `session_id` but queryable across sessions.
- [ ] **Forgetting curve** — auto-decay old memories, prioritize frequently recalled facts
- [x] ~~**Memory search API**~~ — ✅ DONE: `/api/v1/memory/search?q=` and `/api/v1/memory/facts` endpoints exist in server.
- [ ] **ANN index (HNSW)** — integrate `usearch` for >10K facts (currently linear scan)
- [ ] **Memory migration from OpenClaw/ZeroClaw** — import their memory formats

### 2.4 WASM Plugin Ecosystem — Nobody Else Has Sandboxed Plugins

The WASM plugin system is unique. Build the ecosystem.

- [x] ~~**ClawHub registry**~~ — ✅ DONE: Full hub server in `hub.rs` with SQLite-backed skill+plugin storage, publish/search/download/delete. `claw hub serve` runs standalone hub. Proxy mode via `/api/v1/hub/*`.
- [x] ~~**`claw plugin search`**~~ — ✅ DONE: `PluginRegistry::search()` in `registry.rs`, CLI command `claw plugin search <query>` in `plugins.rs`.
- [x] ~~**`claw plugin install <name>`**~~ — ✅ DONE: `PluginRegistry::install()` downloads WASM + verifies + installs. CLI command `claw plugin install <name>` in `plugins.rs`.
- [x] ~~**Plugin page in Web UI**~~ — ✅ DONE: Hub page with Skills + Plugins tabs, search, publish modals (with WASM file upload), pull/delete, stats (total/downloads). Full CRUD.
- [ ] **Plugin SDK** — Rust + AssemblyScript + TinyGo templates for writing plugins
- [ ] **10+ community plugins** — GitHub, Jira, Linear, Notion, Todoist, Home Assistant, etc.
- [ ] **Plugin hot-reload** — detect WASM file changes, reload without restart

### 2.5 Self-Learning — Already Unique, Go Further

No competitor has automatic lesson extraction. Push it further.

- [ ] **Learning dashboard in Web UI** — browse lessons, edit, delete, export
- [x] ~~**Lesson sharing across sessions**~~ — ✅ DONE: Lessons in semantic memory under `learned_lessons` category (global, not session-scoped). Also broadcast to mesh peers via `MeshMessage::SyncDelta`.
- [ ] **Lesson confidence scoring** — track how often a lesson was applied successfully
- [ ] **User-correctable lessons** — "that lesson is wrong, here's the correction"
- [ ] **Learning analytics** — show improvement over time (error rate reduction)

---

## 🟢 Priority 3 — Polish & Distribution

> Production-ready quality, wide distribution, comprehensive docs.

### 3.1 Documentation Site — Match OpenClaw (Currently: README only)

OpenClaw has docs.openclaw.ai. Claw only has README + STATUS.md.

- [ ] **mdBook documentation site** — hosted on GitHub Pages
  - [ ] Getting Started guide
  - [ ] Architecture overview with diagrams
  - [ ] Configuration reference (every key explained)
  - [ ] Channel setup guides (per-channel)
  - [ ] Plugin developer guide
  - [ ] API reference (auto-generated from Axum routes)
  - [ ] Mesh networking guide
  - [ ] Device control guide (browser + Android + iOS)
  - [ ] Security & autonomy guide
  - [ ] Troubleshooting / FAQ
- [ ] **`///` doc comments** on all public items (rustdoc)
- [ ] **Architecture decision records (ADRs)** in docs/

### 3.2 Distribution — Be Everywhere

- [x] ~~**Cross-platform release workflow**~~ — ✅ DONE: `.github/workflows/release.yml` already builds 9 targets:
  - [x] `x86_64-unknown-linux-gnu` + `musl` ✅
  - [x] `aarch64-unknown-linux-gnu` + `musl` ✅
  - [x] `x86_64-apple-darwin` ✅
  - [x] `aarch64-apple-darwin` ✅
  - [x] `x86_64-pc-windows-msvc` ✅
  - [x] `armv7-unknown-linux-gnueabihf` (Raspberry Pi) ✅
  - [ ] `riscv64gc-unknown-linux-gnu` (RISC-V — match PicoClaw's IoT story)
  - [x] `aarch64-linux-android` ✅
- [ ] **Homebrew tap** — `brew install props-nothing/tap/claw`
- [ ] **Cargo publish** — `cargo install claw` from crates.io
- [ ] **Docker Hub automated builds** — multi-arch images (amd64 + arm64)
- [ ] **AUR package** — Arch Linux user repository
- [ ] **Nix flake** — declarative install (like OpenClaw's nix-openclaw)
- [ ] **One-line installer** improvements — version pinning, checksum verification, rollback
- [x] ~~**Auto-updater**~~ — ✅ DONE: `claw update` checks GitHub releases, downloads + replaces binary. Background update check runs automatically on `claw start`.

### 3.3 Edge / IoT Deployment — Match PicoClaw

PicoClaw runs on $10 RISC-V boards. Claw should too.

- [ ] **Minimal feature profile** — `--no-default-features` strips mesh + WASM + device for tiny binaries
- [ ] **Size benchmarks** — measure binary size per feature combination
- [ ] **RISC-V CI builds** — verify compilation and basic tests
- [ ] **Resource usage benchmarks** — RSS, startup time, idle CPU
- [ ] **IoT deployment guide** — Raspberry Pi, RISC-V boards, NanoPi, etc.
- [ ] **Benchmark page** — compare Claw vs OpenClaw vs ZeroClaw vs PicoClaw on size/RAM/startup

### 3.4 Web UI — Beat OpenClaw's Control UI

- [ ] **Session management** — scroll to bottom on resume, show session name, delete session button
- [x] ~~**Memory browser**~~ — 🟡 STUB: basic search + fact listing + category display exists in Web UI. Needs: inline edit, bulk delete, export.
- [ ] **Goal tracker** — visual goal progress, Gantt-style timeline
- [ ] **Plugin manager** — install/uninstall/configure plugins from Web UI
- [ ] **Mesh peer map** — live visualization of connected peers and capabilities
- [x] ~~**Cost dashboard**~~ — ✅ DONE: Dashboard shows budget today (progress bar, color-coded), daily limit, total spend, total tool calls. Needs: per-session and per-model breakdown, charts.
- [x] ~~**Settings editor**~~ — 🟡 STUB: Settings page loads config from `/api/v1/config` and displays as read-only JSON. Needs: inline editing + save endpoint + validation.
- [x] ~~**Mobile-responsive design**~~ — ✅ DONE: Two `@media (max-width: 768px)` blocks in `style.css` (collapsible sidebar, grid adjustments, 95vw modals).
- [ ] **PWA support** — installable as home screen app
- [ ] **Dark/light theme toggle**
- [ ] **WebSocket upgrade** — replace SSE with WebSocket for bidirectional streaming

### 3.5 Observability — Enterprise-Ready

- [ ] **OpenTelemetry export** — traces + metrics to Jaeger/Prometheus/Grafana
- [ ] **Per-session cost tracking** — track USD spend per session, surface in Web UI
- [x] ~~**Streaming token counter**~~ — ✅ DONE: `StreamChunk::Usage(Usage)` emitted during streaming. `Usage` struct tracks `input_tokens`, `output_tokens`, `thinking_tokens`, `cache_read_tokens`, `cache_write_tokens`. Parsed from OpenAI + Anthropic stream events.
- [x] ~~**Health check endpoint**~~ — ✅ DONE: `/health` endpoint exists with `HealthResponse` struct. Extend with detailed component status (LLM, memory, channels, mesh).
- [x] ~~**Structured JSON logging**~~ — ✅ DONE: `logging.format = "json" | "pretty" | "compact"` in config. `tracing_subscriber::fmt().json()` initialized when format is `"json"` in `mod.rs`.

### 3.6 Testing — Beat ZeroClaw's 1,017 Tests

ZeroClaw claims 1,017 tests. Claw has 182. Close the gap decisively.

- [ ] `claw-channels` tests — mock server tests for Discord, Slack, WhatsApp, Signal
- [ ] `claw-mesh` tests — peer registration, message routing, capability matching
- [ ] `claw-cli` tests — command parsing, output formatting, completions
- [ ] `claw-device` tests — CDP mock, ADB command construction, iOS simctl parsing
- [ ] Plugin lifecycle integration test — load → list tools → execute → unload
- [ ] End-to-end test with mock LLM — full agent loop with tool calls
- [ ] Approval flow integration test — trigger → prompt → approve → continue
- [ ] Memory compaction stress test — 10K messages → compaction → verify recall
- [ ] Mesh networking integration test — 3 peers, delegate task, collect result
- [ ] Target: **500+ tests** (beat ZeroClaw on density — tests per LOC)

### 3.7 Agent Intelligence

- [x] ~~**Agent specialization**~~ — ✅ DONE: `sub_agent_system_prompt(role)` generates role-specific prompts for: planner, coder/developer, reviewer, tester/qa, researcher. Config supports `system_prompt` + `system_prompt_file`.
- [ ] **Conversation branching** — fork a session into two paths
- [ ] **TTL-based tool result pruning** — auto-expire old outputs from context
- [x] ~~**Parallel tool execution**~~ — ✅ DONE: `is_parallel_safe()` + `JoinSet` concurrent execution in `agent_loop.rs`. Config flag `parallel_tool_calls` (default: true), `max_parallel_tools` (default: 8).
- [ ] **Streaming tool results** — pipe stdout/stderr from shell commands in real-time

### 3.8 Remaining Performance Items

- [ ] `Arc<Vec<Message>>` or COW for streaming-loop message clones (P0.6)
- [ ] Typed request structs for LLM API bodies — eliminate `serde_json::json!` double-serialization (P0.14)
- [ ] `Uuid` as HashMap keys directly, `as_bytes()` for SQL BLOBs (P0.16)

---

## 🏆 Competitive Scorecard (Target State)

| Feature         | OpenClaw          | ZeroClaw         | PicoClaw            | **Claw (Target)**                                  |
| --------------- | ----------------- | ---------------- | ------------------- | -------------------------------------------------- |
| LLM Providers   | Many              | 22+              | 7                   | **25+ (OpenRouter + 12 direct)** ✅                |
| Channels        | 14+               | 6                | 5                   | **15+ (all major + webhook)** ✅                   |
| Memory          | Basic             | Hybrid search    | Basic markdown      | **3-tier + self-learning ✅ (already best)**       |
| Tools           | ~20               | ~15              | ~10                 | **90+ (30 builtin + 45 device + 15 new)** ✅       |
| Mesh Networking | ❌                | ❌               | ❌                  | **✅ libp2p (only us)**                            |
| WASM Plugins    | ❌                | ❌               | ❌                  | **✅ wasmtime (only us)**                          |
| Voice/Speech    | ✅ (best)         | ❌               | ❌                  | **✅ STT + TTS + wake word**                       |
| Autonomy Levels | 1 mode            | 3 levels         | Binary              | **5 levels + budget + approval ✅ (already best)** |
| Security        | Sandbox           | Encrypted+paired | Basic               | **Encrypted + paired + sandboxed + scoped** ✅     |
| Device Control  | Node-based        | ❌               | ❌                  | **✅ CDP + ADB + simctl native (already best)**    |
| Native Apps     | macOS+iOS+Android | ❌               | ❌                  | Web UI + PWA                                       |
| Docs Site       | ✅ (excellent)    | README           | README              | **✅ mdBook site**                                 |
| Tests           | Vitest suite      | 1,017            | Unknown             | **500+**                                           |
| Binary Size     | ~390MB (Node)     | 3.4 MB           | ~8 MB               | **<10 MB** ✅                                      |
| RAM Usage       | >1 GB             | <5 MB            | <10 MB              | **<10 MB** ✅                                      |
| Edge/IoT        | ❌                | Mentioned        | **✅ ($10 RISC-V)** | **✅ (RISC-V + ARM builds)**                       |
| Onboarding      | Wizard            | Wizard           | Wizard              | **✅ Interactive wizard**                          |

---

## Summary — Execution Order

| Phase       | Items                                                                                                            | Effort     | Impact                                |
| ----------- | ---------------------------------------------------------------------------------------------------------------- | ---------- | ------------------------------------- |
| **Week 1**  | OpenRouter + Gemini + DeepSeek providers, secret encryption, OpenAI-compat config wiring                         | 3-4 days   | Closes #1 gap, matches provider count |
| **Week 2**  | Groq + xAI + more providers, gateway pairing (generalize WhatsApp model), filesystem scoping, command allowlists | 3-4 days   | Security parity with ZeroClaw         |
| **Week 3**  | Voice (STT via Whisper + TTS via ElevenLabs/OpenAI), voice in Web UI + Telegram                                  | 5 days     | Matches OpenClaw's killer feature     |
| **Week 4**  | Docker sandbox runtime, tunnel integration (Tailscale + Cloudflare + ngrok)                                      | 4 days     | Enterprise-ready security             |
| **Week 5**  | iMessage + Matrix + Teams channels, webhook channel, identity/persona system                                     | 5 days     | Channel parity with OpenClaw          |
| **Week 6**  | mdBook docs site, Homebrew tap, remaining channel integrations                                                   | 4 days     | Professional distribution             |
| **Week 7**  | HEARTBEAT.md, mesh improvements (binary format, request-response, dashboard)                                     | 4 days     | Amplify unique advantages             |
| **Week 8**  | Test blitz (target 500+), Web UI polish (PWA, theme toggle, settings editor)                                     | 5 days     | Production quality                    |
| **Ongoing** | WASM plugin ecosystem (hot-reload, SDK), desktop automation, learning analytics, IoT benchmarks                  | Continuous | Long-term moat                        |

---

_Last updated: 2026-02-16 — 182 tests, 83 tools (38 builtin + 45 device), 22 API routes, ~38.1k lines (~34.2k Rust + 3.9k Web). 13 of 18 P0 perf items fixed. Already done: cross-platform builds (9 targets), auto-updater, setup wizard, doctor, parallel tools, health check, ClawHub (search+install+publish), cost dashboard, memory browser, streaming tokens, JSON logging, mobile responsive, agent specialization, cross-session memory, lesson sharing, memory search API. **Target: beat all competitors within 8 weeks.**_
