# 🦞 Claw — TODO

> Organized by priority. Updated 2026-02-12 after self-learning system, operator trust, memory deletion/listing tools, and multi-strategy search improvements.

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

### Tools (30 builtin + 45 device = 75 total)

- [x] `shell_exec` — non-interactive shell commands
- [x] `file_read`, `file_write`, `file_list`, `file_edit`, `file_find`, `file_grep`
- [x] `apply_patch` — multi-file search-and-replace
- [x] `process_start`, `process_list`, `process_kill`, `process_output`
- [x] `terminal_open`, `terminal_run`, `terminal_view`, `terminal_input`, `terminal_close` (real PTY)
- [x] `memory_store`, `memory_search`, `memory_delete`, `memory_list`, `memory_delete`, `memory_list`
- [x] `goal_create`, `goal_list`, `goal_complete_step`, `goal_update_status`
- [x] `web_search`, `http_fetch`, `llm_generate`
- [x] `mesh_peers`, `mesh_delegate`, `mesh_status`

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

### Server

- [x] 19 API routes on Axum (includes `/api/v1/screenshots/{file}`)
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

- [x] 15 commands: start, chat, status, version, config, set, plugin, logs, doctor, init, setup, completions, skill, hub, mesh
- [x] Shell completions (bash/zsh/fish)
- [x] `claw logs` with color-coding, type filter, JSON output

### Testing & CI

- [x] 176 tests across 10 crates
- [x] Mock LLM provider
- [x] GitHub Actions CI (check, test, clippy, fmt, release builds)

</details>

---

## 🔴 Priority 1 — Bugs & Polish

### 1.1 Docker Rust Version

- [ ] Update Dockerfile `FROM rust:1.88` → `FROM rust:1.93`
- [ ] Test Docker build end-to-end

### 1.2 Web UI Session Navigation

- [ ] When resuming a session, scroll to bottom of restored messages
- [ ] Show session name in chat header (not just truncated UUID)
- [ ] "Delete session" button / swipe-to-delete on sessions page
- [ ] Clear localStorage `claw_session_id` when session is deleted

### 1.3 Memory Database 0-Byte Issue

- [ ] Investigate why `~/.claw/memory.db` can end up as 0 bytes
- [ ] Add integrity check on startup (PRAGMA integrity_check)
- [ ] Auto-recreate if corrupted

---

## 🟡 Priority 2 — Feature Gaps

### 2.1 Discord Channel 🟡 STUB

**Current**: Struct exists, all methods have `// TODO` comments. No gateway connection.

- [ ] Add `serenity` or `twilight` dependency
- [ ] Implement Discord gateway connection (WebSocket)
- [ ] Handle `MESSAGE_CREATE` events → `ChannelEvent::Message`
- [ ] `send()` — POST to Discord REST API
- [ ] Slash commands: `/claw ask`, `/claw status`
- [ ] Rich embeds for tool results and status
- [ ] Approval flow via Discord buttons

### 2.2 ClawHub — Plugin Registry 🟡 STUB

**Current**: `PluginRegistry` HTTP client points to non-existent `registry.clawhub.com`.

Choose a path:

- [ ] **Option A**: Build ClawHub as a standalone service (upload, search, download, accounts)
- [ ] **Option B**: Use GitHub Releases as registry (download from tagged releases)
- [ ] **Option C**: Remove registry code, keep plugins local-only

### 2.3 Browser Automation Tool ✅ DONE

**Implemented in `claw-device` crate.**

- [x] `browser_navigate`, `browser_click`, `browser_type`, `browser_screenshot`, `browser_upload_file` + 9 more tools
- [x] Headless Chrome via CDP (Chrome DevTools Protocol) over HTTP + WebSocket, 1920×1080 viewport
- [x] Integrated into agent tool dispatch in `claw-runtime`

### 2.4 Android & iOS Device Control ✅ DONE

**Implemented in `claw-device` crate.**

- [x] Android via ADB — 15 tools (screenshot, tap, swipe, type, shell, apps, files)
- [x] iOS via simctl/idb — 14 tools (screenshot, tap, swipe, type, apps, simulators)
- [x] Graceful fallback chains (e.g., simctl → idb)

### 2.5 Sub-Agent Spawning 🔴

**OpenClaw has this. Claw uses mesh delegation as alternative.**

- [ ] `sub_agent_spawn` tool — spawn isolated agent with limited tools
- [ ] Task-scoped memory (sub-agent gets subset of parent context)
- [ ] Result aggregation back to parent

---

## 🟢 Priority 3 — Nice to Have

### 3.1 More Channel Adapters

- [ ] **Slack** — Events API / Socket Mode, OAuth2, Block Kit, threads
- [ ] **WhatsApp** — Business API, webhook verification, templates
- [ ] **Matrix** — `matrix-sdk`, room-based conversations, E2EE

### 3.2 Observability

- [ ] OpenTelemetry export (traces + metrics to Jaeger/Prometheus/Grafana)
- [ ] Per-session cost tracking in Web UI
- [ ] Streaming token counter in chat UI

### 3.3 Security Hardening

- [ ] Per-session rate limiting (not just per-IP)
- [ ] Configurable rate limits in `claw.toml`
- [ ] Tool sandboxing (chroot / namespaces for `shell_exec`)
- [ ] Audit log cryptographic signing (HMAC instead of DefaultHasher)

### 3.4 Agent Intelligence

- [x] ~~Skill learning — agent defines new skills from successful interactions~~ (implemented as self-learning lesson extraction)
- [ ] Agent specialization — per-role system prompts and config
- [ ] TTL-based tool result pruning (auto-expire old outputs)
- [ ] Conversation branching (fork a session)

### 3.5 Distribution

- [ ] Release workflow — cross-compile for 6 platforms, upload to GitHub Releases
- [ ] Homebrew tap: `brew install claw`
- [ ] Cargo publish to crates.io
- [ ] Docker Hub automated image builds

### 3.6 Documentation

- [ ] `///` doc comments on all public items
- [ ] User guide (mdbook or similar)
- [ ] Plugin developer guide
- [ ] API reference (auto-generated)
- [ ] Architecture decision records (ADRs)

### 3.7 Testing Gaps

- [ ] `claw-channels` tests — Telegram URL construction, WebChat plumbing
- [ ] `claw-mesh` tests — peer registration, message routing, capability matching
- [ ] `claw-cli` tests — command parsing, output formatting
- [ ] Plugin lifecycle integration test: load → list tools → execute → unload
- [ ] End-to-end test with real LLM (gated behind env flag)

### 3.8 Config & UX

- [ ] Feature-gate `libp2p` behind cargo feature (reduce binary size)
- [ ] WebSocket support (optional upgrade from SSE for chat)
- [ ] Config hot-reload notification in Web UI
- [ ] Plugin page in Web UI (list loaded plugins, tools, install/uninstall)

---

## Summary — What's Left

| Priority | Item                      | Effort      | Impact                   |
| -------- | ------------------------- | ----------- | ------------------------ |
| 🔴 P1    | Docker Rust version fix   | 30 min      | Blocks Docker users      |
| 🔴 P1    | Memory DB integrity check | 1 hour      | Prevents data loss       |
| 🔴 P1    | Session UI polish         | 2 hours     | Better UX                |
| ✅ P2    | Browser automation        | **DONE**    | 45 device tools          |
| ✅ P2    | Android & iOS control     | **DONE**    | Full device control      |
| ✅ P2    | Self-learning system      | **DONE**    | Automatic lesson memory  |
| ✅ P2    | Memory delete + list      | **DONE**    | Full memory CRUD         |
| ✅ P2    | Multi-strategy search     | **DONE**    | Reliable memory recall   |
| ✅ P2    | Operator trust prompt     | **DONE**    | Credential handling      |
| 🟡 P2    | Discord channel           | 1-2 days    | Medium — niche audience  |
| 🟡 P2    | ClawHub registry          | 3-5 days    | Low — local plugins work |
| 🟡 P2    | Sub-agent spawning        | 1-2 days    | Low — mesh covers this   |
| 🟢 P3    | Slack/WhatsApp/Matrix     | 1 week each | Low                      |
| 🟢 P3    | OpenTelemetry             | 2-3 days    | Low                      |
| 🟢 P3    | Documentation             | Ongoing     | Medium                   |
| 🟢 P3    | More tests                | Ongoing     | Medium                   |
| 🟢 P3    | Distribution              | 2-3 days    | Medium                   |

---

_Last updated: 2026-02-12 — 176 tests, 75 tools (30 builtin + 45 device), 19 API routes, ~29.6k lines_
