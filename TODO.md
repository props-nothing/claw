# 🦞 Claw — TODO

> Organized by priority. Updated 2026-02-16 after deep performance audit, accuracy review, and codebase growth verification.

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

## 🔴 Priority 1 — Bugs & Polish

### 1.1 Web UI Session Navigation

- [ ] When resuming a session, scroll to bottom of restored messages
- [ ] Show session name in chat header (not just truncated UUID)
- [ ] "Delete session" button / swipe-to-delete on sessions page
- [ ] Clear localStorage `claw_session_id` when session is deleted

### 1.2 Memory Database 0-Byte Issue

- [ ] Investigate why `~/.claw/memory.db` can end up as 0 bytes
- [ ] Add integrity check on startup (PRAGMA integrity_check)
- [ ] Auto-recreate if corrupted

---

## 🟡 Priority 2 — Feature Gaps

### 2.1 ClawHub — Plugin Registry 🟡 STUB

**Current**: `PluginRegistry` HTTP client points to non-existent `registry.clawhub.com`.

Choose a path:

- [ ] **Option A**: Build ClawHub as a standalone service (upload, search, download, accounts)
- [ ] **Option B**: Use GitHub Releases as registry (download from tagged releases)
- [ ] **Option C**: Remove registry code, keep plugins local-only

### 2.2 Matrix Channel 🔴

- [ ] Add `matrix-sdk` dependency
- [ ] Implement room-based conversations
- [ ] E2EE support

---

## 🟢 Priority 3 — Nice to Have

### 3.1 Observability

- [ ] OpenTelemetry export (traces + metrics to Jaeger/Prometheus/Grafana)
- [ ] Per-session cost tracking in Web UI
- [ ] Streaming token counter in chat UI

### 3.2 Security Hardening

- [ ] Per-session rate limiting (not just per-IP)
- [ ] Configurable rate limits in `claw.toml`
- [ ] Tool sandboxing (chroot / namespaces for `shell_exec`)
- [ ] Audit log cryptographic signing (HMAC instead of DefaultHasher)

### 3.3 Agent Intelligence

- [ ] Agent specialization — per-role system prompts and config
- [ ] TTL-based tool result pruning (auto-expire old outputs)
- [ ] Conversation branching (fork a session)

### 3.4 Distribution

- [ ] Release workflow — cross-compile for 6 platforms, upload to GitHub Releases
- [ ] Homebrew tap: `brew install claw`
- [ ] Cargo publish to crates.io
- [ ] Docker Hub automated image builds

### 3.5 Documentation

- [ ] `///` doc comments on all public items
- [ ] User guide (mdbook or similar)
- [ ] Plugin developer guide
- [ ] API reference (auto-generated)
- [ ] Architecture decision records (ADRs)

### 3.6 Testing Gaps

- [ ] `claw-channels` tests — Discord/Slack/WhatsApp/Signal unit tests
- [ ] `claw-mesh` tests — peer registration, message routing, capability matching
- [ ] `claw-cli` tests — command parsing, output formatting
- [ ] `claw-device` tests — CDP mocking, ADB command construction
- [ ] Plugin lifecycle integration test: load → list tools → execute → unload
- [ ] End-to-end test with real LLM (gated behind env flag)
- [ ] Target: double test count from 182 → 350+

### 3.7 Config & UX

- [ ] Feature-gate `libp2p` behind cargo feature (reduce binary size ~2-3 MB)
- [ ] Feature-gate `wasmtime` behind cargo feature (reduce binary size ~1-2 MB)
- [ ] WebSocket support (optional upgrade from SSE for chat)
- [ ] Config hot-reload notification in Web UI
- [ ] Plugin page in Web UI (list loaded plugins, tools, install/uninstall)

---

## Summary — What's Left

| Priority | Item                              | Effort    | Impact                         |
| -------- | --------------------------------- | --------- | ------------------------------ |
| ✅ P0.1  | Cache hostname (LazyLock)         | ~~5 min~~ | **DONE** — unblocks tokio      |
| ✅ P0.2  | Shared reqwest::Client            | ~~5 min~~ | **DONE** — saves ~100ms/search |
| ✅ P0.3  | `Arc<Vec<Tool>>` for tools        | ~~30 min~~| **DONE** — no more deep clone  |
| ✅ P0.4  | RwLock for MemoryStore            | ~~2-3 hrs~~| **DONE** — concurrent reads   |
| ✅ P0.5  | Unify streaming/non-streaming     | ~~4-6 hrs~~| **DONE** — -720 LOC           |
| 🟡 P0.6  | Arc messages in loop              | —         | Partially resolved by P0.5     |
| ✅ P0.7  | SSE buffer.drain()               | ~~5 min~~ | **DONE** — no realloc          |
| ✅ P0.8  | `prepare_cached()` in SQLite      | ~~15 min~~| **DONE** — 2-3× query speedup |
| ✅ P0.9  | N+1 goal queries → JOIN           | ~~20 min~~| **DONE** — 21 queries → 1     |
| ✅ P0.10 | Partial sort in vector search     | ~~10 min~~| **DONE** — O(n) partition      |
| ✅ P0.13 | WASM InstancePre (pre-link)       | ~~30 min~~| **DONE** — no Linker per call  |
| ✅ P0.15 | `VecDeque` for episodes           | ~~5 min~~ | **DONE** — O(n) → O(1)         |
| ✅ P0.17 | EventBus capacity 4096→256        | ~~2 min~~ | **DONE** — saves ~30 KB        |
| ✅ P0.18 | WASM AOT compilation cache        | ~~30 min~~| **DONE** — skip recompile      |
| 🔴 P1    | Memory DB integrity check         | 1 hour    | Prevents data loss             |
| 🔴 P1    | Session UI polish                 | 2 hours   | Better UX                      |
| 🟡 P2    | ClawHub registry                  | 3-5 days  | Low — local plugins work       |
| 🟡 P2    | Matrix channel                    | 2-3 days  | Medium — niche audience        |
| 🟢 P3    | OpenTelemetry                     | 2-3 days  | Medium                         |
| 🟢 P3    | Feature-gate libp2p + wasmtime    | 1 day     | ~3-5 MB binary size reduction  |
| 🟢 P3    | Documentation                     | Ongoing   | Medium                         |
| 🟢 P3    | Double test coverage (181 → 350+) | Ongoing   | Medium                         |
| 🟢 P3    | Distribution (brew, crates.io)    | 2-3 days  | Medium                         |

---

## Corrections from 2026-02-16 accuracy review

The previous TODO.md (dated 2026-02-12) had several stale entries. Changes made:

| Item | Old (wrong) | New (correct) |
|---|---|---|
| Test count | 176 | **182** |
| Total Rust LOC | 26,179 | **~34,900** |
| Web UI LOC | 3,394 | **3,906** |
| Builtin tools | 30 | **38** (added sub_agent ×3, cron ×3, channel_send_file, llm_generate) |
| Total tools | 75 | **83** (38 builtin + 45 device) |
| CLI commands | 15 | **17** (added channels, help) |
| API routes | 18-19 (inconsistent) | **22** (added sub-tasks, scheduled-tasks, events SSE, screenshots) |
| Docker Rust version | "needs updating from 1.88" | **Already updated to 1.93** ✅ |
| Discord | "Stub — all methods TODO" | **Fully implemented** (524 LOC, gateway + REST) ✅ |
| Slack | "No code" | **Fully implemented** (510 LOC, Events API + Socket Mode) ✅ |
| WhatsApp | "No code" | **Fully implemented** (1,514 LOC, Business Cloud API) ✅ |
| Signal | Not mentioned | **Fully implemented** (324 LOC, signal-cli JSON-RPC) ✅ |
| Sub-agent spawning | "TODO" | **Fully implemented** (943 LOC, 3 tools) ✅ |
| Scheduled tasks | Not mentioned | **Fully implemented** (440 LOC, 3 tools) ✅ |
| claw-runtime LOC | 7,558 | **10,842** |
| claw-channels LOC | 1,093 | **4,539** |
| claw-cli LOC | 2,007 | **3,276** |

---

_Last updated: 2026-02-16 — 181 tests (1 ignored), 83 tools (38 builtin + 45 device), 22 API routes, ~38.1k lines (~34.2k Rust + 3.9k Web). 13 of 18 P0 perf items fixed (P0.6 partial, P0.11/P0.12/P0.14/P0.16 deferred)._
