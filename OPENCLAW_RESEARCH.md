# OpenClaw Research Analysis

> Structured comparison of [OpenClaw](https://github.com/openclaw/openclaw) features vs Claw.
> Focus areas: agent loop continuation, tool richness, and context management.

---

## 1. Tools Provided

### OpenClaw

OpenClaw has a **rich, categorized tool system** with ~25+ built-in tools organized into groups:

| Group | Tools |
|-------|-------|
| `group:fs` | `read`, `write`, `edit`, `apply_patch` |
| `group:runtime` | `exec`, `bash`, `process` |
| `group:sessions` | `sessions_list`, `sessions_history`, `sessions_send`, `sessions_spawn`, `session_status` |
| `group:memory` | `memory_search`, `memory_get` |
| `group:web` | `web_search`, `web_fetch` |
| `group:ui` | `browser`, `canvas` |
| `group:automation` | `cron`, `gateway` |
| `group:messaging` | `message` |
| `group:nodes` | `nodes` |

Additional tools: `grep`, `find`, `ls`, `image`, `agents_list`

Each tool has **action-based subcommands** (e.g., `browser` supports `status/start/stop/tabs/open/focus/close/snapshot/screenshot/navigate/act/pdf/upload/dialog`).

**Tool profiles** control which tools are available: `minimal`, `coding`, `messaging`, `full`.

**Tool policy system**: Allow/deny lists at global, per-agent, and sandbox levels. Deny always takes precedence. Groups can be used as shorthand in policies.

### Claw

Claw has **13 built-in tools**, all flat (no sub-actions):

- `shell_exec`, `file_read`, `file_write`, `file_list`
- `http_fetch`, `web_search`
- `memory_store`, `memory_search`
- `goal_create`, `goal_list`
- `mesh_peers`, `mesh_delegate`, `mesh_status`
- `llm_generate` (via tool)

Additionally supports plugin-provided tools via `claw-plugin`.

### Gap Analysis

| Feature | OpenClaw | Claw |
|---------|----------|------|
| File editing (precise) | `edit` tool with search/replace | ❌ Only full `file_write` |
| Multi-file patching | `apply_patch` (structured patches) | ❌ |
| Browser control | Full CDP browser automation (724 lines) | ❌ |
| Background processes | `process` tool (list/poll/log/write/kill) | ❌ |
| Canvas/UI rendering | `canvas` tool (present/eval/snapshot/A2UI) | ❌ |
| Cron/scheduling | `cron` tool (add/update/remove/run) | ❌ |
| Sub-agent spawning | `sessions_spawn` tool | ❌ |
| Device nodes | `nodes` tool (camera/screen/location/run) | ❌ |
| Tool policy system | Per-agent allow/deny with group shorthands | ❌ |
| Plugin tools | External extensions | ✅ `claw-plugin` crate |
| Mesh delegation | ❌ | ✅ `mesh_delegate` |
| Goal tracking | ❌ (uses skills/memory) | ✅ `goal_create`/`goal_list` |

**Priority gaps**: `edit` (precise file editing), `process` (background exec management), browser automation, and the tool policy system.

---

## 2. Agent Loop Architecture

### OpenClaw

OpenClaw's agent loop is **delegated to `pi-agent-core`** ([@mariozechner/pi-agent-core](https://www.npmjs.com/package/@mariozechner/pi-agent-core)), wrapped with OpenClaw-specific lifecycle management:

1. **RPC entry** (`agent` method): Validates params, resolves session, returns `{ runId, acceptedAt }` immediately (non-blocking).
2. **`agentCommand`**: Resolves model + thinking level, loads skills, calls `runEmbeddedPiAgent`.
3. **`runEmbeddedPiAgent`**: Serializes runs via **per-session + global queues**, builds pi session, subscribes to events, enforces timeout, returns payloads + usage.
4. **`subscribeEmbeddedPiSession`**: Bridges pi-agent-core events to streams (tool events, assistant events, lifecycle events).
5. **`agent.wait`**: Uses `waitForAgentJob` — event-based completion detection.

**Key behaviors:**
- **No explicit max_iterations** — the core loop runs until the model stops calling tools.
- **Timeout-based termination**: Default 600s (`agents.defaults.timeoutSeconds`), enforced via abort timer.
- **Early termination**: AbortSignal, gateway disconnect, RPC timeout.
- **Serialized execution**: Per-session queuing prevents concurrent runs on the same session.

### Claw

Claw has a **self-contained agent loop** in `agent.rs`:

```
loop {
    iteration += 1;
    if iteration > max_iterations { break; }
    
    budget.check()?;
    llm_response = llm.complete(request);
    budget.record_spend(cost);
    
    if !has_tool_calls {
        match stop_reason {
            MaxTokens => inject_continuation_prompt; continue;
            _ => {
                if is_lazy_stop(text) { inject_nudge; continue; }
                break;  // genuinely done
            }
        }
    }
    
    for tool_call in tool_calls {
        budget.record_tool_call()?;
        guardrail_check(tool_call);
        execute_tool(tool_call);
    }
}
```

**Key behaviors:**
- **Explicit `max_iterations`** (configurable).
- **Budget-based termination**: Cost tracking + tool call counting.
- **`MaxTokens` continuation**: Injects continuation prompt when model is cut off.
- **Lazy stop detection**: Re-prompts model if it stops prematurely without completing the task.
- **Guardrail checks** on every tool call (autonomy level, approval gates).

### Gap Analysis

| Feature | OpenClaw | Claw |
|---------|----------|------|
| Max iteration cap | ❌ (relies on timeout) | ✅ `max_iterations` config |
| Timeout enforcement | ✅ 600s default | ❌ |
| MaxTokens continuation | Via pi-agent-core (implicit) | ✅ Explicit continuation prompt |
| Lazy stop detection | ❌ | ✅ `is_lazy_stop()` heuristic |
| Budget/cost tracking | Via usage reporting | ✅ `BudgetTracker` with cost limits |
| Run serialization | ✅ Per-session + global queues | ❌ |
| Non-blocking RPC | ✅ Returns immediately | ❌ Synchronous processing |
| Guardrail gating | Via tool policy | ✅ Per-call autonomy/approval |

**Priority**: Claw's loop is actually solid. Add a **timeout** and **run serialization** (prevent concurrent runs on same session). The lazy stop detection is a Claw advantage.

---

## 3. Context Window Management & Compaction ⭐

### OpenClaw

This is OpenClaw's most sophisticated subsystem. It uses a **two-layer approach**:

#### Layer 1: Session Compaction (persistent)
**File**: `src/agents/compaction.ts` + `src/agents/pi-extensions/compaction-safeguard.ts`

- **Trigger**: (1) Model returns context overflow error → compact → retry. (2) After successful turn when `contextTokens > contextWindow - reserveTokens`.
- **Process**: `summarizeInStages` with adaptive chunk ratios based on message sizes.
- **Pre-compaction memory flush**: Silent turn to write durable notes to disk before compaction.
- **Dropped-messages summarization**: Summarizes what was lost.
- **Tool failure collection**: Tracks and preserves tool error context.
- **File operations tracking**: Preserves which files were modified.
- **Manual**: `/compact` command with optional instructions.

#### Layer 2: Context Pruning (in-memory, non-persistent)
**File**: `src/agents/pi-extensions/context-pruning/pruner.ts`

- `pruneContextMessages`: Trims old tool results in-memory before LLM calls.
- **Cache-TTL mode**: Tool results have a TTL, pruned after expiry.
- Does NOT modify the session file — only affects what's sent to the model.

#### Context Window Guard
**File**: `src/agents/context-window-guard.ts`

- Guards against small context windows.
- Resolves effective context window from model config → agent config → defaults.
- Ensures minimum reserve tokens for the model's response.

#### Token Estimation
- `estimateMessagesTokens`: Estimates token count for a message array.
- `splitMessagesByTokenShare`: Splits messages to fit within a target token budget.
- `pruneHistoryForContextShare`: Removes oldest messages to fit context share.

### Claw

Claw has **no context management**:
- Working memory is an in-memory message list with no size limits.
- No compaction, summarization, or pruning.
- No token counting or context window awareness.
- If context overflows, the LLM API call simply fails.

### Gap Analysis

| Feature | OpenClaw | Claw |
|---------|----------|------|
| Token counting | ✅ `estimateMessagesTokens` | ❌ |
| Context window awareness | ✅ Per-model resolution | ❌ |
| Auto-compaction on overflow | ✅ Summarize → retry | ❌ |
| Threshold compaction | ✅ Proactive before overflow | ❌ |
| In-memory pruning | ✅ Tool result TTL | ❌ |
| Pre-compaction memory flush | ✅ Saves notes to disk | ❌ |
| Manual compact command | ✅ `/compact` | ❌ |
| Context window guard | ✅ Min reserve tokens | ❌ |

**This is the #1 gap.** Without context management, Claw cannot handle long conversations or complex multi-step tasks. Implementation priority:
1. Add token estimation (can use tiktoken-rs or simple char-based heuristic like OpenClaw's `4 chars ≈ 1 token`)
2. Add context window config per model
3. Add message pruning (drop old tool results first)
4. Add overflow recovery (catch API error → summarize → retry)
5. Add proactive compaction (before hitting the limit)

---

## 4. Sub-Agent Spawning

### OpenClaw

**File**: `src/agents/tools/sessions-spawn-tool.ts` + `src/agents/subagent-registry.ts`

Full sub-agent system:
- **`sessions_spawn` tool**: Non-blocking, returns `{ status, runId, childSessionKey }` immediately.
- **Isolated sessions**: Each sub-agent gets `agent:<agentId>:subagent:<uuid>`.
- **Cross-agent spawning**: Allowlist (`agents.list[].subagents.allowAgents`).
- **No nesting**: Sub-agents cannot spawn sub-agents.
- **Concurrency limits**: `DEFAULT_AGENT_MAX_CONCURRENT = 4`, `DEFAULT_SUBAGENT_MAX_CONCURRENT = 8`.
- **Auto-archive**: After configurable minutes (default 60).
- **Model override**: Sub-agent can use a different model.
- **Completion announce**: Posts summary to requester chat when done.
- **Agent-to-agent messaging**: `sessions_send` with ping-pong turns (max 5).
- **Abort management**: `stopSubagentsForRequester`.
- **CLI commands**: `/subagents list|stop|info`.

### Claw

- ❌ No sub-agent system.
- Mesh delegation (`mesh_delegate`) is conceptually similar but operates across physical devices, not within the same runtime.

### Gap Analysis

Sub-agents are useful for parallelizing complex tasks. Low priority unless targeting IDE-like coding workflows. Mesh delegation could be extended to serve a similar purpose for local sub-agents.

---

## 5. Tool Result Truncation

### OpenClaw

**File**: `src/agents/pi-embedded-runner/tool-result-truncation.ts`

Sophisticated truncation system:
- `MAX_TOOL_RESULT_CONTEXT_SHARE = 0.3` — No single tool result can use more than 30% of context window.
- `HARD_MAX_TOOL_RESULT_CHARS = 400,000` — Safety net.
- `MIN_KEEP_CHARS = 2,000` — Always keeps the beginning of the result.
- `calculateMaxToolResultChars`: Scales with context window (`4 chars ≈ 1 token` heuristic).
- `truncateToolResultText`: Appends a warning about truncation.
- `truncateOversizedToolResultsInSession`: Rewrites session file.
- `truncateOversizedToolResultsInMessages`: In-memory truncation before LLM call.
- **Overflow recovery flow**: Compaction first → fall back to tool-result truncation → retry.

### Claw

- ❌ No tool result truncation.
- Large `shell_exec` or `file_read` outputs go into context unmodified.
- This directly contributes to context overflow issues.

### Gap Analysis

**High priority.** Easy to implement:
1. Add a `max_tool_result_chars` config (e.g., 50,000 chars).
2. Truncate with `"... [truncated, showing first N chars of M total]"` suffix.
3. Scale the limit based on context window size.

---

## 6. Sandbox / Isolation

### OpenClaw

**Full Docker-based sandbox system**:

**Modes**: `"off"` | `"non-main"` | `"all"`
**Scope**: `"session"` | `"agent"` | `"shared"` (controls container granularity)
**Workspace access**: `"none"` | `"ro"` | `"rw"`

Docker hardening:
- `readOnlyRoot: true`
- `capDrop: ["ALL"]`
- `network: "none"` (default — no egress)
- `--security-opt no-new-privileges`
- Configurable: `pidsLimit`, `memory`, `cpus`, `ulimits`, `seccompProfile`, `apparmorProfile`
- Custom bind mounts, setup commands, DNS config
- Per-agent sandbox overrides
- Auto-prune: idle > 24h OR age > 7d

**Tool policy per sandbox**: Allow/deny lists, group shorthands, deny-takes-precedence.

**Sandbox browser**: Separate container for browser automation inside sandbox, with CDP port, VNC, noVNC.

### Claw

- ❌ No sandbox system.
- Tools execute directly on the host.
- Guardrail system provides some safety (approval gates, risk levels) but no isolation.

### Gap Analysis

OpenClaw's sandbox is comprehensive but also complex. For Claw, consider:
1. **Phase 1**: Docker-based exec sandbox (route `shell_exec` through a container).
2. **Phase 2**: Workspace mount modes (ro/rw/none).
3. **Phase 3**: Per-session containers.

---

## 7. Model Providers

### OpenClaw

Extensive multi-provider support with a **pi-ai catalog** (built-in, no config needed):

**Built-in providers** (just need API key):
- Anthropic (`claude-opus-4-6`, `claude-sonnet-4-5`, etc.)
- OpenAI (`gpt-5.2`, `gpt-5.1-codex`, etc.)
- OpenAI Codex
- Google Gemini (`gemini-3-pro-preview`, etc.)
- Google Vertex, Antigravity, Gemini CLI
- Z.AI (GLM)
- xAI (Grok)
- Groq, Cerebras, Mistral
- GitHub Copilot
- Vercel AI Gateway
- OpenCode Zen

**Custom providers** (via `models.providers`):
- Moonshot AI (Kimi), MiniMax, Synthetic, Together AI, Venice
- Ollama (auto-detected at `localhost:11434`)
- LiteLLM, Cloudflare AI Gateway, Qianfan
- Any OpenAI/Anthropic-compatible proxy (LM Studio, vLLM, etc.)

**API protocols**: `anthropic-messages`, `openai-responses`, `openai-completions`, `openai-codex-responses`, `google-generative-ai`

**Features**: Model aliases, forward-compat fallbacks, per-model context window, cost tracking, model allowlists.

**Embedding providers**: OpenAI, Gemini, Voyage, local (GGUF via node-llama-cpp).

### Claw

The `claw-llm` crate supports:
- Anthropic (`anthropic.rs`)
- OpenAI (`openai.rs`)
- Local models (`local.rs`)
- Mock provider (`mock.rs`)
- Model router with circuit breaker, exponential backoff, fallback
- Embedding support (`embedding.rs`)

### Gap Analysis

| Feature | OpenClaw | Claw |
|---------|----------|------|
| Built-in provider catalog | ✅ 15+ providers | 3 providers |
| Auto-discovery (Ollama) | ✅ | ❌ |
| Custom/proxy providers | ✅ Config-driven | ❌ |
| Multiple API protocols | ✅ 5 protocols | 2 protocols |
| Circuit breaker | ❌ (via pi-ai) | ✅ Full implementation |
| Fallback routing | ✅ Primary + fallbacks | ✅ Fallback model config |
| Forward-compat aliases | ✅ Model ID normalization | ❌ |

Claw's router is well-engineered (circuit breaker is a real advantage). Adding more providers (Google, Ollama) would close the practical gap.

---

## 8. Canvas / Nodes System

### OpenClaw

**Unique to OpenClaw** — ties into its iOS/macOS/Android companion apps:

**Canvas tool** (`canvas-tool.ts`):
- Actions: `present`, `hide`, `navigate`, `eval`, `snapshot`, `a2ui_push`, `a2ui_reset`
- Renders web content on paired devices via WebView
- JavaScript evaluation, UI snapshots, A2UI rendering
- Architecture: Canvas Host (HTTP) → Node Bridge (TCP) → Node App (device)

**Nodes tool** (`nodes-tool.ts`, 491 lines):
- Device management: `status`, `describe`, `pending`, `approve`, `reject`
- Notifications: `notify` (macOS system notifications)
- Camera: `camera_snap`, `camera_list`, `camera_clip`
- Screen: `screen_record`
- Location: `location_get`
- Remote execution: `run` (macOS `system.run`)
- Generic: `invoke` (any node command)

**Pairing system**: Devices pair with the gateway, can be approved/rejected. Tokens, permissions per node.

### Claw

- ❌ No canvas or device node system.
- The `claw-mesh` crate provides peer-to-peer networking between Claw instances, which is a different but related concept.

### Gap Analysis

Canvas/Nodes is OpenClaw's differentiator for physical-world integration. Not directly relevant for Claw unless targeting IoT/device scenarios. Claw's mesh system could evolve to support similar device-bridging if needed.

---

## 9. Long-Running Tasks

### OpenClaw

Multiple mechanisms:

1. **Sub-agents** (`sessions_spawn`): Non-blocking background agents with completion announcements.
2. **Cron jobs** (`cron` tool): Scheduled recurring tasks with full cron expressions.
3. **Process management** (`process` tool): Background shell commands with poll/log/kill.
4. **OpenProse loops**: `loop until **condition** (max: N)`, `repeat N`, `parallel for` — bounded iteration with safety limits.
5. **Heartbeat system**: System events that wake the agent on schedule.
6. **Process registry** (`bash-process-registry.ts`): Tracks running/finished sessions with sweeper.

### Claw

- `shell_exec` is synchronous only (with timeout).
- Goal system (`goal_create`/`goal_list`) tracks multi-step plans but doesn't execute them autonomously.
- No cron, no background processes, no heartbeat.

### Gap Analysis

| Feature | OpenClaw | Claw |
|---------|----------|------|
| Background shell commands | ✅ `process` tool | ❌ |
| Cron/scheduled tasks | ✅ `cron` tool | ❌ |
| Sub-agent spawning | ✅ `sessions_spawn` | ❌ |
| Heartbeat/wake system | ✅ System events | ❌ |
| Goal tracking | ❌ | ✅ `goal_create` |
| Bounded loops (DSL) | ✅ OpenProse | ❌ |

**Priority**: Add `process` tool equivalent for background exec management. This is a practical gap for any real-world agent usage (e.g., starting a dev server while editing files).

---

## Summary: Biggest Gaps (Ranked)

### 🔴 Critical (blocks real-world usage)

1. **Context window management** — Without token counting, compaction, or pruning, Claw will fail on any non-trivial multi-step task. The model will hit context limits and the API call will error.

2. **Tool result truncation** — A single large `file_read` or `shell_exec` output can blow the context. Simple truncation with configurable limits is essential.

3. **Timeout enforcement** — The agent loop needs a wall-clock timeout to prevent infinite loops or stuck LLM calls.

### 🟡 Important (limits capability)

4. **Precise file editing** — `edit` tool (search/replace) instead of full file rewrites. Critical for coding tasks.

5. **Background process management** — `process` tool for managing long-running commands (dev servers, builds, watchers).

6. **Tool policy system** — Per-agent/per-session allow/deny lists for tools. Important for multi-user or sandboxed scenarios.

### 🟢 Nice to Have (differentiators)

7. **Sub-agent spawning** — Parallel task execution within the runtime.
8. **Docker sandbox** — Tool execution isolation.
9. **Cron/scheduling** — Recurring tasks and reminders.
10. **Browser automation** — Web browsing and interaction.
11. **More model providers** — Google, Ollama, OpenRouter, etc.

### Claw Advantages Over OpenClaw

| Feature | Notes |
|---------|-------|
| **Circuit breaker** | Full implementation with states, OpenClaw relies on pi-ai |
| **Lazy stop detection** | Re-prompts model when it stops prematurely |
| **Guardrail system** | Per-call autonomy levels + approval gates |
| **Budget tracking** | Cost limits + tool call counting |
| **Goal system** | Built-in multi-step goal planning |
| **Mesh networking** | P2P device delegation |
| **Injection defense** | Prompt injection detection |
| **Rust performance** | Single binary, low memory footprint |

---

## Recommended Implementation Order

```
Phase 1 (Context Safety):
  ├── Token estimation (char-based heuristic: 4 chars ≈ 1 token)
  ├── Tool result truncation (max chars per result)
  ├── Context window config per model
  ├── Message pruning (drop old tool results first)
  └── Overflow recovery (catch API error → trim → retry)

Phase 2 (Agent Loop Hardening):
  ├── Wall-clock timeout for agent loop
  ├── Run serialization (prevent concurrent runs per session)
  └── Streaming continuation on MaxTokens

Phase 3 (Tool Richness):
  ├── edit tool (search/replace file editing)
  ├── process tool (background exec management)
  ├── grep/find tools (file search)
  └── Tool result caching

Phase 4 (Advanced):
  ├── Context compaction (summarize old messages via LLM)
  ├── Sub-agent spawning
  ├── Tool policy system (allow/deny per agent)
  ├── More model providers (Ollama, Google)
  └── Docker sandbox for exec
```
