# 🦞 Claw — Implementation Status

**Date**: 2026-02-12  
**Rust**: 1.93.0 (aarch64-apple-darwin)  
**Tests**: 176 passing across 10 crates  
**Source**: 26,179 lines of Rust + 3,394 lines of Web UI (JS/CSS/HTML) = ~29.6k total  
**Crates**: 13 workspace crates + binary

---

## Architecture Overview

```
                    ┌───────────────────────────────────┐
                    │         Web UI (SPA)              │
                    │  Dashboard · Chat · Sessions ·    │
                    │  Goals · Memory · Tools · Logs    │
                    └──────────────┬────────────────────┘
                                   │ HTTP/SSE
              ┌────────────────────┼────────────────────┐
              │              Axum Server                 │
              │  18 routes · Auth · CORS · Rate limit   │
              │  Prometheus metrics · Health check       │
              └────────────────────┬────────────────────┘
                                   │
              ┌────────────────────┼────────────────────┐
              │            Agent Runtime                 │
              │  Concurrent task dispatch (tokio::spawn) │
              │  Per-session run locks · Budget · Guard  │
              └───┬────────┬────────┬────────┬──────────┘
                  │        │        │        │
           ┌──────┘   ┌────┘   ┌────┘   ┌────┘
           ▼          ▼        ▼        ▼
    ┌───────────┐ ┌────────┐ ┌──────┐ ┌──────────┐
    │ LLM Router│ │ Memory │ │ Mesh │ │ Channels │
    │ 3 provid. │ │ 3-tier │ │libp2p│ │ TG+Web   │
    │ + retry   │ │+SQLite │ │+mDNS │ │          │
    │ + circuit │ │+embed  │ │+tools│ │          │
    └───────────┘ └────────┘ └──────┘ └──────────┘
```

---

## Summary Matrix

| Component                 | Status          | Details                                                                                                                                                                                                                                                                                                              |
| ------------------------- | --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Agent Loop**            | ✅ Done         | 4,796 lines. Receive→Recall→Think→Guard→Act→Remember→Respond. Auto-continuation on max_tokens, lazy stop detection, budget enforcement, wall-clock timeout, model fallback, per-session run locks. **Operator trust prompt** for credential handling. **Self-learning** with automatic lesson extraction (error→correction→success pattern detection). **Multi-strategy RECALL** (vector + keyword + extracted-keyword search with dedup). |
| **LLM Providers**         | ✅ Done         | OpenAI (complete+stream), Anthropic (complete+stream+thinking), Ollama (complete+stream). Router with failover, retry (3x exponential backoff), circuit breaker (5 failures → 60s cool-off). 2,422 lines.                                                                                                            |
| **30 Built-in Tools**     | ✅ Done         | `shell_exec`, `file_read/write/list/edit/find/grep`, `apply_patch`, `process_start/list/kill/output`, `terminal_open/run/view/input/close` (real PTY), `memory_store/search/delete/list`, `goal_create/list/complete_step/update_status`, `web_search`, `http_fetch`, `mesh_peers/delegate/status`, `llm_generate`. 1,711 lines. |
| **Web UI**                | ✅ Done         | Dark-themed SPA (vanilla JS, no build step). 8 pages: Dashboard, Chat (SSE streaming with tool calls + approval prompts), Sessions (clickable resume), Goals, Memory, Tools, Logs, Settings. 3,290 lines.                                                                                                            |
| **Session Management**    | ✅ Done         | Per-session tracking with message count, channel/target routing, `get_or_insert` for resume, `record_message` for counting, `set_name` auto-labeling, `run_lock` for serialization, SQLite persistence (60s flush), cleanup of empty sessions on startup, restore on startup.                                        |
| **Memory System**         | ✅ Done         | 3-tier: Working (per-session, auto-compaction via LLM), Episodic (keyword search, SQLite persist + load on startup), Semantic (fact store with vector + word-level scored search, SQLite persist + load). **Memory deletion** (per-fact and per-category). **Memory listing** (browse all stored facts). **Learned lessons** auto-extracted and recalled. Session messages persisted. 1,578 lines. |
| **Embeddings**            | ✅ Done         | OpenAI `text-embedding-3-small` + Ollama embedding providers. Used in memory recall (vector search) and fact storage.                                                                                                                                                                                                |
| **Autonomy & Guardrails** | ✅ Done         | 5 levels (L0–L4), 3 guardrail rules (risk level, destructive action, network exfiltration), allow/deny lists, budget tracker (daily USD + per-loop tool calls). 1,274 lines.                                                                                                                                         |
| **Goal Planner**          | ✅ Done         | Full lifecycle: create→plan→execute→complete. Sub-goals, progress tracking, delegation to mesh peers. SQLite persistence + load on startup. LLM tools for step completion and status updates.                                                                                                                        |
| **Approval Flow**         | ✅ Done         | End-to-end: API endpoints (`/approve`, `/deny`), Web UI inline buttons, Telegram inline keyboards + callback queries, CLI prompts, text commands. Timeout auto-deny.                                                                                                                                                 |
| **Skills System**         | ✅ Done         | TOML-based skill definitions with parameters, steps, variable binding, conditions. Topological executor. 4 built-in skills. CLI commands. Skills exposed as `skill.*` tools to LLM. 1,182 lines.                                                                                                                     |
| **Telegram**              | ✅ Done         | Long-polling with timeouts + exponential backoff + 409 conflict detection, send (Markdown+fallback), photo upload (multipart), typing indicators, inline keyboard approvals, `/start /help /status /new /approve /deny` commands. 1,028 lines.                                                                       |
| **Mesh Networking**       | ✅ Done         | libp2p with TCP+Noise+Yamux, GossipSub, mDNS, Identify, Kademlia. Task delegation, capability routing, memory sync (SyncDelta), peer discovery. 3 LLM tools. CLI + API. 948 lines.                                                                                                                                   |
| **Server**                | ✅ Done         | Axum with 18 routes (chat, stream, sessions, goals, tools, facts, memory search, config, audit, approvals, mesh status/peers/send, health, metrics). Bearer auth, CORS, per-IP rate limiting (token bucket). Prometheus metrics (16 counters). 1,946 lines.                                                          |
| **WASM Plugins**          | ✅ Done         | wasmtime with fuel-limited execution (10M fuel). Plugin ABI (`claw_malloc` + `claw_invoke`), manifest parsing, BLAKE3 checksums, scaffold generator. Feature-gated behind `wasm`. 844 lines.                                                                                                                         |
| **Config**                | ✅ Done         | TOML schema with env overrides. Hot-reload file watcher (notify). `claw config set` CLI. 20+ validation checks. Context window auto-detect per model. 1,005 lines.                                                                                                                                                   |
| **CLI**                   | ✅ Done         | 15 commands: start, chat, status, version, config, set, plugin, logs, doctor, init, setup, completions, skill, hub, mesh. Shell completions (bash/zsh/fish). 1,968 lines.                                                                                                                                            |
| **Testing**               | ✅ Done         | 176 tests: claw-autonomy (31), claw-config (13), claw-core (19), claw-llm (14), claw-memory (25), claw-plugin (12), claw-runtime (19), claw-server (26), claw-skills (17). Mock LLM provider.                                                                                                                        |
| **CI/CD**                 | ✅ Done         | GitHub Actions: check, test, clippy, fmt, cross-platform release builds.                                                                                                                                                                                                                                             |
| **Docker**                | 🟡 Needs update | Multi-stage Dockerfile + docker-compose.yml. References `rust:1.88` (needs updating to 1.93).                                                                                                                                                                                                                        |
| **Discord**               | 🟡 Stub         | Struct exists, all methods are TODOs. No gateway connection.                                                                                                                                                                                                                                                         |
| **ClawHub Registry**      | 🟡 Stub         | HTTP client code exists, points to non-existent `registry.clawhub.com`.                                                                                                                                                                                                                                              |
| **Slack/WhatsApp/Matrix** | 🔴 Missing      | No code.                                                                                                                                                                                                                                                                                                             |

---

## Per-Crate Breakdown

| Crate           | Lines      | Tests   | Purpose                                                                             |
| --------------- | ---------- | ------- | ----------------------------------------------------------------------------------- |
| `claw-core`     | 747        | 19      | Error types, events, messages, tools, EventBus                                      |
| `claw-config`   | 1,026      | 13      | TOML config, env overrides, validation, hot-reload                                  |
| `claw-llm`      | 2,422      | 14      | OpenAI/Anthropic/Ollama providers, router, retry, circuit breaker, embeddings, mock |
| `claw-memory`   | 1,578      | 25      | Working memory, episodic, semantic (word-level search + delete), SQLite store        |
| `claw-autonomy` | 1,274      | 31      | Levels, guardrails, budget, planner, approval                                       |
| `claw-runtime`  | 7,558      | 19      | Agent loop, session manager, 30 built-in tools, self-learning, operator trust       |
| `claw-server`   | 2,030      | 26      | Axum HTTP server, 18 routes, rate limiting, metrics                                 |
| `claw-channels` | 1,093      | 0       | Telegram (working), Discord (stub), WebChat (bridge)                                |
| `claw-mesh`     | 948        | 0       | libp2p mesh networking, peer discovery, task delegation                             |
| `claw-plugin`   | 844        | 12      | WASM plugin host, manifest, registry client                                         |
| `claw-skills`   | 1,182      | 17      | Skill definitions, executor, registry                                               |
| `claw-cli`      | 2,007      | 0       | 15 CLI commands                                                                     |
| `claw-device`   | 3,458      | 0       | Browser (CDP), Android (ADB), iOS (simctl/idb) — 45 device tools                    |
| **Total**       | **26,179** | **176** |                                                                                     |

---

## Tool Inventory (30 tools)

### File Operations

| Tool          | Description                               |
| ------------- | ----------------------------------------- |
| `file_read`   | Read file contents                        |
| `file_write`  | Write/create files                        |
| `file_list`   | List directory contents                   |
| `file_edit`   | Search-and-replace within files           |
| `file_find`   | Recursive glob/regex file search          |
| `file_grep`   | Regex content search across files         |
| `apply_patch` | Multi-file search-and-replace in one call |

### Shell & Process

| Tool             | Description                                       |
| ---------------- | ------------------------------------------------- |
| `shell_exec`     | Run non-interactive shell commands (timeout 120s) |
| `process_start`  | Spawn detached background processes               |
| `process_list`   | List running processes                            |
| `process_kill`   | Kill process by PID                               |
| `process_output` | Get output from a started process                 |

### PTY Terminals

| Tool             | Description                            |
| ---------------- | -------------------------------------- |
| `terminal_open`  | Open persistent PTY terminal session   |
| `terminal_run`   | Run command in terminal, return output |
| `terminal_view`  | View current terminal output           |
| `terminal_input` | Send input/keystrokes to terminal      |
| `terminal_close` | Close terminal session                 |

### Memory & Goals

| Tool                 | Description                                           |
| -------------------- | ----------------------------------------------------- |
| `memory_store`       | Store a fact (key-value with embedding)               |
| `memory_search`      | Search episodic + semantic memory (multi-strategy)    |
| `memory_delete`      | Delete a fact or entire category from memory          |
| `memory_list`        | List all stored facts, optionally filtered by category|
| `goal_create`        | Create a new goal with steps                          |
| `goal_list`          | List active goals                                     |
| `goal_complete_step` | Mark a goal step as complete                          |
| `goal_update_status` | Update goal status                                    |

### Network & AI

| Tool           | Description                         |
| -------------- | ----------------------------------- |
| `web_search`   | Web search via DuckDuckGo           |
| `http_fetch`   | HTTP GET/POST requests              |
| `llm_generate` | Generate text with a specific model |

### Mesh

| Tool            | Description                         |
| --------------- | ----------------------------------- |
| `mesh_peers`    | Discover connected peers            |
| `mesh_delegate` | Delegate task to peer by capability |
| `mesh_status`   | Show mesh network info              |

---

## API Endpoints (18 routes)

| Method | Path                             | Description                                      |
| ------ | -------------------------------- | ------------------------------------------------ |
| POST   | `/api/v1/chat`                   | Send message, get response                       |
| POST   | `/api/v1/chat/stream`            | SSE streaming chat                               |
| GET    | `/api/v1/status`                 | Runtime status (uptime, model, budget, sessions) |
| GET    | `/api/v1/sessions`               | List sessions (filtered: message_count > 0)      |
| GET    | `/api/v1/sessions/{id}/messages` | Get session message history                      |
| GET    | `/api/v1/goals`                  | Active goals with steps                          |
| GET    | `/api/v1/tools`                  | All available tools                              |
| GET    | `/api/v1/memory/facts`           | Stored facts                                     |
| GET    | `/api/v1/memory/search?q=`       | Search episodic + semantic memory                |
| GET    | `/api/v1/config`                 | Runtime configuration                            |
| GET    | `/api/v1/audit`                  | Audit log entries                                |
| POST   | `/api/v1/approvals/{id}/approve` | Approve pending action                           |
| POST   | `/api/v1/approvals/{id}/deny`    | Deny pending action                              |
| GET    | `/api/v1/mesh/status`            | Mesh network status                              |
| GET    | `/api/v1/mesh/peers`             | Connected mesh peers                             |
| POST   | `/api/v1/mesh/send`              | Send message to mesh peer                        |
| GET    | `/health`                        | Health check                                     |
| GET    | `/metrics`                       | Prometheus metrics (16 counters)                 |

---

## SQLite Tables (8)

| Table              | Purpose                                                    |
| ------------------ | ---------------------------------------------------------- |
| `episodes`         | Episodic memory — conversation summaries, outcomes, tags   |
| `episode_messages` | Messages associated with episodes                          |
| `facts`            | Semantic memory — key-value facts with optional embeddings |
| `goals`            | Goal definitions with status, priority, delegation         |
| `goal_steps`       | Individual steps within goals                              |
| `audit_log`        | Timestamped audit trail with checksum                      |
| `sessions`         | Session metadata (name, channel, target, message_count)    |
| `session_messages` | Working memory persistence (JSON blob per session)         |

---

## Web UI Pages (8)

| Page      | Route        | Features                                                                                            |
| --------- | ------------ | --------------------------------------------------------------------------------------------------- |
| Dashboard | `#/`         | Uptime, model, autonomy level, budget bar, session count, channels                                  |
| Chat      | `#/chat`     | SSE streaming, tool call segments (collapsible), approval buttons, session resume from localStorage |
| Sessions  | `#/sessions` | Clickable rows to resume, name/ID display, message count, created date                              |
| Goals     | `#/goals`    | Progress bars, step lists, priority badges                                                          |
| Memory    | `#/memory`   | Facts table, search (episodic + semantic)                                                           |
| Tools     | `#/tools`    | Card grid with risk badges, mutating indicators                                                     |
| Logs      | `#/logs`     | Audit log with search, type filter, auto-refresh, color-coded badges                                |
| Settings  | `#/settings` | Read-only formatted config view                                                                     |

---

## What Works End-to-End

Running `claw start` with a valid OpenAI or Anthropic key:

1. Config loads from `~/.claw/claw.toml` with env overrides
2. SQLite opens, loads persisted facts, episodes, goals, and sessions
3. Empty sessions cleaned up from previous runs
4. LLM provider registers with retry + circuit breaker
5. Channels start (Telegram long-polling if configured)
6. Mesh networking starts (libp2p with mDNS if enabled)
7. HTTP server on port 3700 with auth, CORS, rate limiting
8. Web UI served from `~/.claw/web/`
9. Concurrent message processing via `tokio::spawn` — nothing blocks
10. Agent loop: embed query → recall from memory → build system prompt → LLM call → guardrail check → tool execution → remember → respond
11. Sessions: create, track message count, auto-label, persist to SQLite, resume from Web UI
12. Tools: 30 built-in tools including PTY terminals, file ops, process management, memory CRUD, mesh delegation
13. Context management: auto-detect window per model, LLM-powered compaction, overflow recovery
14. Budget + guardrails enforced every iteration
15. Episodic memory recorded after each conversation turn
16. Background: session persistence (60s), peer announcement (30s), config hot-reload

---

## Claw vs OpenClaw Comparison

| Feature             | Claw                          | OpenClaw             |
| ------------------- | ----------------------------- | -------------------- |
| Language            | Rust (single binary)          | TypeScript (Node.js) |
| Mesh networking     | ✅ libp2p multi-agent         | ❌                   |
| Circuit breaker     | ✅ Auto-failover              | ❌                   |
| Budget tracking     | ✅ Daily USD + tool limits    | ❌                   |
| Guardrail engine    | ✅ 3 rules + allow/deny       | Partial              |
| Goal planner        | ✅ Multi-step + delegation    | ❌                   |
| Skills system       | ✅ TOML workflows             | ❌                   |
| WASM plugins        | ✅ Fuel-limited sandbox       | Node.js (no sandbox) |
| Autonomy levels     | ✅ L0–L4                      | ❌                   |
| Approval flow       | ✅ API + Web + Telegram + CLI | Partial              |
| Context compaction  | ✅ LLM-powered                | ✅ LLM-powered       |
| Tool count          | 75 (30 + 45 device)           | ~25                  |
| Browser automation  | ✅ CDP + file upload          | ✅ CDP               |
| Sub-agent spawning  | Via mesh delegation           | ✅ Built-in          |
| Community/Ecosystem | New                           | 184k stars           |

---

_Last updated: 2026-02-12 — 176 tests, 75 tools (30 builtin + 45 device), 18 API routes, 8 SQLite tables, 8 web pages, ~29.6k lines_
