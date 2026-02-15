// ── Claw Control Centre — SPA ─────────────────────────────────
// Vanilla JS, no build step. Hash-based routing.

const API = ""; // Same origin

// ── API Client ────────────────────────────────────────────────

async function api(path, opts = {}) {
  const res = await fetch(`${API}${path}`, {
    headers: { "Content-Type": "application/json", ...opts.headers },
    ...opts,
  });
  if (!res.ok) throw new Error(`API ${res.status}: ${res.statusText}`);
  return res.json();
}

async function apiStream(path, body) {
  const res = await fetch(`${API}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`API ${res.status}: ${res.statusText}`);
  return res.body.getReader();
}

// ── Utilities ─────────────────────────────────────────────────

function $(sel, ctx = document) {
  return ctx.querySelector(sel);
}
function $$(sel, ctx = document) {
  return [...ctx.querySelectorAll(sel)];
}

function escHtml(str) {
  const div = document.createElement("div");
  div.textContent = str;
  return div.innerHTML;
}

/**
 * Extract a meaningful short summary from a tool result.
 * For shell_exec results, skip "Exit code:" / "STDOUT:" / "STDERR:" boilerplate
 * and return the first line of actual content.
 */
function extractToolSummary(toolName, plain) {
  if (!plain) return "";

  // For shell results, try to find the most meaningful line
  const lines = plain
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  if (lines.length === 0) return "";

  // Skip lines that are just section headers from shell output
  const skipPatterns =
    /^(Exit code:\s*\d+|STDOUT:|STDERR:|Command completed successfully)$/i;
  const meaningful = lines.filter((l) => !skipPatterns.test(l));

  const best = meaningful.length > 0 ? meaningful[0] : lines[0];
  return best.length > 100 ? best.slice(0, 97) + "…" : best;
}

function riskBadge(level) {
  const cls =
    level >= 7
      ? "risk-critical"
      : level >= 5
        ? "risk-high"
        : level >= 3
          ? "risk-medium"
          : "risk-low";
  return `<span class="risk-badge ${cls}">Risk ${level}</span>`;
}

async function resolveApproval(approvalId, action, msgId) {
  const btn = document.querySelector(`#approval-${approvalId} .btn-${action}`);
  const container = document.querySelector(`#approval-${approvalId}`);
  if (!container) return;

  // Disable both buttons immediately
  container.querySelectorAll("button").forEach((b) => {
    b.disabled = true;
    b.style.opacity = "0.5";
  });
  if (btn)
    btn.textContent = action === "approve" ? "⏳ Approving…" : "⏳ Denying…";

  try {
    const resp = await fetch(
      `${API}/api/v1/approvals/${approvalId}/${action}`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
      },
    );
    const data = await resp.json();

    // Update the UI to show resolution
    container.innerHTML = `
            <div class="approval-resolved ${action === "approve" ? "approval-approved" : "approval-denied"}">
                ${action === "approve" ? "✅ Approved" : "❌ Denied"} — ${escHtml(data.status || action)}
            </div>
        `;

    // Mark this specific approval as resolved so streaming updates show it properly
    const msg = chatMessages.find((m) => m.id === msgId);
    if (msg && msg.pendingApprovals) {
      const approval = msg.pendingApprovals.find((a) => a.id === approvalId);
      if (approval) {
        approval.resolved = true;
        approval.resolution = action;
      }
    }
  } catch (e) {
    container.innerHTML = `<div class="approval-resolved approval-denied">⚠️ Error: ${escHtml(e.message)}</div>`;
  }
}

function formatUptime(secs) {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}h ${m}m`;
}

function formatUsd(v) {
  return `$${Number(v).toFixed(4)}`;
}

function formatDate(iso) {
  if (!iso) return "—";
  const d = new Date(iso);
  return d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function riskBadge(level) {
  if (level <= 2)
    return `<span class="badge badge-success">Low (${level})</span>`;
  if (level <= 5)
    return `<span class="badge badge-warning">Med (${level})</span>`;
  return `<span class="badge badge-error">High (${level})</span>`;
}

function autonomyLabel(level) {
  const labels = [
    "Manual",
    "Assisted",
    "Supervised",
    "Autonomous",
    "Full Auto",
  ];
  return labels[level] || `L${level}`;
}

// Simple markdown-like rendering for chat messages
function renderMarkdown(text) {
  if (!text) return "";
  let html = escHtml(text);
  // Code blocks
  html = html.replace(/```(\w*)\n([\s\S]*?)```/g, "<pre><code>$2</code></pre>");
  // Inline code
  html = html.replace(/`([^`]+)`/g, "<code>$1</code>");
  // Bold
  html = html.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  // Italic
  html = html.replace(/\*([^*]+)\*/g, "<em>$1</em>");
  // Screenshot URLs — render as clickable images
  html = html.replace(
    /(\/api\/v1\/screenshots\/[\w._-]+\.png)/g,
    '<div class="screenshot-preview"><img src="$1" alt="Screenshot" class="screenshot-img" onclick="window.open(\'$1\', \'_blank\')" /></div>',
  );
  // Markdown images: ![alt](url)
  html = html.replace(
    /!\[([^\]]*)\]\((\/api\/v1\/screenshots\/[\w._-]+\.png)\)/g,
    '<div class="screenshot-preview"><img src="$2" alt="$1" class="screenshot-img" onclick="window.open(\'$2\', \'_blank\')" /></div>',
  );
  // Newlines
  html = html.replace(/\n/g, "<br>");
  return html;
}

// ── Router ────────────────────────────────────────────────────

const routes = {
  "/": renderDashboard,
  "/chat": renderChat,
  "/sessions": renderSessions,
  "/goals": renderGoals,
  "/agents": renderSubAgents,
  "/scheduler": renderScheduler,
  "/memory": renderMemory,
  "/tools": renderTools,
  "/hub": renderHub,
  "/logs": renderLogs,
  "/settings": renderSettings,
};

function navigate() {
  const hash = location.hash.slice(1) || "/";
  const render = routes[hash] || renderDashboard;

  // Update active nav link
  $$(".nav-link").forEach((link) => {
    const href = link.getAttribute("href").slice(1) || "/";
    link.classList.toggle("active", href === hash);
  });

  // Render the page
  const content = $("#content");
  content.innerHTML = '<div class="loading"><div class="spinner"></div></div>';
  render(content);
}

// ── Status Polling ────────────────────────────────────────────

let cachedStatus = null;

async function fetchStatus() {
  try {
    cachedStatus = await api("/api/v1/status");
    updateConnectionStatus(true);
  } catch {
    updateConnectionStatus(false);
  }
  return cachedStatus;
}

function updateConnectionStatus(online) {
  const dot = $(".status-dot");
  const text = $(".status-text");
  if (dot) {
    dot.className = `status-dot ${online ? "online" : "offline"}`;
  }
  if (text) {
    text.textContent = online ? "Connected" : "Offline";
  }
}

// ── Dashboard Page ────────────────────────────────────────────

async function renderDashboard(el) {
  const status = await fetchStatus();
  if (!status) {
    el.innerHTML = `<div class="page">
            <div class="empty-state">
                <div class="empty-icon">⚠️</div>
                <div class="empty-text">Cannot connect to Claw runtime</div>
            </div>
        </div>`;
    return;
  }

  const budgetPct =
    status.budget.daily_limit_usd > 0
      ? Math.min(
          100,
          (status.budget.spent_usd / status.budget.daily_limit_usd) * 100,
        )
      : 0;
  const budgetColor =
    budgetPct > 80 ? "error" : budgetPct > 50 ? "warning" : "success";

  el.innerHTML = `
    <div class="page">
        <div class="page-header">
            <div>
                <h1 class="page-title">Dashboard</h1>
                <p class="page-subtitle">Claw runtime overview</p>
            </div>
            <span class="badge badge-success">${escHtml(status.status)}</span>
        </div>

        <div class="card-grid">
            <div class="card">
                <div class="card-header">
                    <span class="card-label">Model</span>
                </div>
                <div class="card-value accent" style="font-size:18px; font-family: var(--font-mono);">
                    ${escHtml(status.model)}
                </div>
                <div class="card-detail">Autonomy: ${autonomyLabel(status.autonomy_level)}</div>
            </div>

            <div class="card">
                <div class="card-header">
                    <span class="card-label">Uptime</span>
                </div>
                <div class="card-value success">${formatUptime(status.uptime_secs)}</div>
                <div class="card-detail">v${escHtml(status.version)}</div>
            </div>

            <div class="card">
                <div class="card-header">
                    <span class="card-label">Budget Today</span>
                </div>
                <div class="card-value ${budgetColor}">${formatUsd(status.budget.spent_usd)}</div>
                <div class="card-detail">of ${formatUsd(status.budget.daily_limit_usd)} daily limit</div>
                <div class="progress-bar">
                    <div class="progress-fill ${budgetColor}" style="width: ${budgetPct}%"></div>
                </div>
            </div>

            <div class="card">
                <div class="card-header">
                    <span class="card-label">Sessions</span>
                </div>
                <div class="card-value info">${status.sessions}</div>
                <div class="card-detail">Active sessions</div>
            </div>
        </div>

        <div class="card-grid">
            <div class="card">
                <div class="card-header">
                    <span class="card-label">Total Spend</span>
                </div>
                <div class="card-value" style="font-size: 20px;">${formatUsd(status.budget.total_spend_usd)}</div>
                <div class="card-detail">${status.budget.total_tool_calls} tool calls total</div>
            </div>

            <div class="card">
                <div class="card-header">
                    <span class="card-label">Channels</span>
                </div>
                <div class="card-value" style="font-size: 20px;">${status.channels.length || 0}</div>
                <div class="card-detail">${status.channels.length ? status.channels.map((c) => `<span class="badge badge-info" style="margin-right:4px;">${escHtml(c)}</span>`).join("") : "No channels connected"}</div>
            </div>
        </div>
    </div>`;
}

// ── Chat Page ─────────────────────────────────────────────────
//
// Content model: each assistant message stores an ordered `segments` array.
// Segments are rendered top-to-bottom in the order they arrived from the
// stream, so tool calls appear inline where they happened and the final
// summary text ends up at the bottom — matching the real execution flow.

let chatSessionId = localStorage.getItem("claw_session_id") || null;
let chatMessages = [];
let chatStreaming = false;

// Smart auto-scroll: only scroll to bottom if user is already near bottom
function chatAutoScroll() {
  const msgs = $("#chat-messages");
  if (!msgs) return;
  const threshold = 150; // px from bottom
  const atBottom =
    msgs.scrollHeight - msgs.scrollTop - msgs.clientHeight < threshold;
  if (atBottom) msgs.scrollTop = msgs.scrollHeight;
}

async function renderChat(el) {
  // If we have a persisted session but no messages loaded, try to restore from server
  if (chatSessionId && chatMessages.length === 0) {
    try {
      const data = await api(`/api/v1/sessions/${chatSessionId}/messages`);
      const msgs = data.messages || [];
      for (const m of msgs) {
        if (m.role === "system") continue; // skip system messages in UI
        chatMessages.push({
          role: m.role === "tool" ? "assistant" : m.role,
          text: m.content || "",
          id: m.id || `r${Date.now()}${Math.random()}`,
          segments:
            m.role === "tool"
              ? [
                  {
                    type: "text",
                    content: `[Tool result]: ${(m.content || "").slice(0, 200)}`,
                  },
                ]
              : undefined,
        });
      }
    } catch (_) {
      // Server may not have history; start fresh
    }
  }

  el.innerHTML = `
    <div class="page" style="padding: 16px 32px; max-width: 100%; height: 100%;">
        <div class="chat-container">
            <div class="page-header" style="margin-bottom: 8px;">
                <div>
                    <h1 class="page-title">Chat</h1>
                    <p class="page-subtitle" id="chat-session-label">
                        ${chatSessionId ? `Session: ${chatSessionId.slice(0, 8)}…` : "New conversation"}
                    </p>
                </div>
                <button class="btn" onclick="newChatSession()">New Session</button>
            </div>
            <div class="chat-messages" id="chat-messages">
                ${
                  chatMessages.length === 0
                    ? `
                    <div class="empty-state">
                        <div class="empty-icon">🦞</div>
                        <div class="empty-text">Start a conversation with Claw</div>
                    </div>
                `
                    : chatMessages.map(renderChatMessage).join("")
                }
            </div>
            <div class="chat-input-wrap">
                <div class="chat-input-box">
                    <textarea id="chat-input" placeholder="Type a message…" rows="1"
                              onkeydown="handleChatKeydown(event)"
                              oninput="autoResize(this)"></textarea>
                    <button class="send-btn" id="send-btn" onclick="sendChatMessage()">
                        <svg viewBox="0 0 24 24"><path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z"/></svg>
                    </button>
                </div>
            </div>
        </div>
    </div>`;

  const msgs = $("#chat-messages");
  msgs.scrollTop = msgs.scrollHeight;
  $("#chat-input").focus();
}

// Track which tool results the user has manually expanded/collapsed.
// Keys are tool result IDs (e.g. "tr-call_xxx"), values are true (expanded) or false (collapsed).
const _toolExpandState = {};

// Toggle a tool result's expanded/collapsed state
window.toggleToolResult = function (toolId) {
  const el = document.getElementById(`tool-result-${toolId}`);
  if (!el) return;
  el.classList.toggle("expanded");
  const isExpanded = el.classList.contains("expanded");
  _toolExpandState[toolId] = isExpanded;
  const btn = el.previousElementSibling;
  if (btn && btn.classList.contains("tool-header")) {
    const chevron = btn.querySelector(".tool-chevron");
    if (chevron) chevron.textContent = isExpanded ? "▾" : "▸";
  }
};

function renderToolCallSegment(tc, msgId, isStreaming) {
  const hasResult = tc.result != null;
  const isError = hasResult && tc.result.startsWith("❌");
  const resultId = `tr-${tc.id || Math.random().toString(36).slice(2)}`;

  // Determine a short summary for collapsed view
  let summary = "";
  if (hasResult) {
    const plain = tc.result.replace(/^❌ /, "");
    summary = extractToolSummary(tc.name, plain);
  }

  // Check if the result contains a screenshot URL — prefer structured data, fall back to regex
  let screenshotUrl = tc.screenshot_url || null;
  if (!screenshotUrl && hasResult) {
    const m = tc.result.match(/\/api\/v1\/screenshots\/[\w._-]+\.png/);
    if (m) screenshotUrl = m[0];
  }

  const isScreenshot = !!screenshotUrl;
  const expandedByDefault = isScreenshot;

  let resultHtml = "";
  if (hasResult && screenshotUrl) {
    // Render screenshot as an image — auto-expanded so the image shows immediately
    resultHtml = `
      <div class="tool-result-wrap expanded" id="tool-result-${resultId}">
        <div class="screenshot-preview">
          <img src="${screenshotUrl}" alt="Screenshot" class="screenshot-img"
               onclick="window.open('${screenshotUrl}', '_blank')"
               onerror="this.parentElement.innerHTML='<div class=screenshot-error>⚠️ Failed to load screenshot</div>'" />
        </div>
      </div>`;
  } else if (hasResult) {
    // Check if user manually toggled this result
    const userExpanded = _toolExpandState[resultId] === true;
    resultHtml = `
      <div class="tool-result-wrap${userExpanded ? " expanded" : ""}" id="tool-result-${resultId}">
        <div class="tool-result ${isError ? "tool-result-error" : ""}">${escHtml(tc.result)}</div>
      </div>`;
  } else if (isStreaming) {
    resultHtml =
      '<div class="typing-indicator"><span></span><span></span><span></span></div>';
  }

  // Chevron: respect user's manual toggle, otherwise default (▾ for screenshots, ▸ for others)
  const isUserExpanded =
    resultId in _toolExpandState
      ? _toolExpandState[resultId]
      : expandedByDefault;
  const chevron = hasResult ? (isUserExpanded ? "▾" : "▸") : "";

  return `
    <div class="msg-tool-call ${isError ? "tool-error" : hasResult ? "tool-done" : "tool-running"}">
      <div class="tool-header" onclick="toggleToolResult('${resultId}')">
        <span class="tool-chevron">${chevron}</span>
        <span class="tool-icon">${hasResult ? (isError ? "❌" : "✅") : "⚡"}</span>
        <span class="tool-name-label">${escHtml(tc.name)}</span>
        ${summary && !isScreenshot ? `<span class="tool-summary">${escHtml(summary)}</span>` : ""}
        ${isScreenshot ? '<span class="tool-summary">📸 screenshot</span>' : ""}
      </div>
      ${resultHtml}
    </div>`;
}

function renderApprovalSegment(a, msgId) {
  if (a.resolved) {
    return `
      <div class="approval-prompt" id="approval-${a.id}">
        <div class="approval-resolved ${a.resolution === "approve" ? "approval-approved" : "approval-denied"}">
          ${a.resolution === "approve" ? "✅ Approved" : "❌ Denied"} — ${escHtml(a.tool_name)}
        </div>
      </div>`;
  }
  const argsStr =
    typeof a.tool_args === "object"
      ? JSON.stringify(a.tool_args, null, 2)
      : String(a.tool_args);
  return `
    <div class="approval-prompt" id="approval-${a.id}">
      <div class="approval-header">
        <span class="approval-icon">⚠️</span>
        <span class="approval-title">Approval Required</span>
        ${riskBadge(a.risk_level)}
      </div>
      <div class="approval-detail">
        <strong>${escHtml(a.tool_name)}</strong> wants to execute:
        <pre class="approval-args">${escHtml(argsStr)}</pre>
        <div class="approval-reason">${escHtml(a.reason)}</div>
      </div>
      <div class="approval-actions">
        <button class="btn btn-approve" onclick="resolveApproval('${a.id}', 'approve', '${msgId}')">✅ Approve</button>
        <button class="btn btn-deny" onclick="resolveApproval('${a.id}', 'deny', '${msgId}')">❌ Deny</button>
      </div>
    </div>`;
}

function renderChatMessage(msg) {
  if (msg.role === "user") {
    return `
      <div class="chat-msg user">
        <div class="msg-avatar">👤</div>
        <div class="msg-bubble">${renderMarkdown(msg.text)}</div>
      </div>`;
  }

  // Segment-based rendering: render in order of arrival
  let content = "";
  if (msg.segments && msg.segments.length > 0) {
    for (const seg of msg.segments) {
      if (seg.type === "text") {
        content += `<div class="msg-text-block">${renderMarkdown(seg.content)}</div>`;
      } else if (seg.type === "tool_call") {
        content += renderToolCallSegment(seg, msg.id, false);
      } else if (seg.type === "approval") {
        content += renderApprovalSegment(seg, msg.id);
      }
    }
  } else {
    // Legacy / fallback: old format with text + toolCalls
    if (msg.text) content += renderMarkdown(msg.text);
    if (msg.toolCalls && msg.toolCalls.length > 0) {
      content += msg.toolCalls
        .map((tc) => renderToolCallSegment(tc, msg.id, false))
        .join("");
    }
    if (msg.pendingApprovals) {
      content += msg.pendingApprovals
        .map((a) => renderApprovalSegment(a, msg.id))
        .join("");
    }
  }

  return `
    <div class="chat-msg assistant">
      <div class="msg-avatar">🦞</div>
      <div class="msg-bubble" id="msg-${msg.id || ""}">${content}</div>
    </div>`;
}

window.newChatSession = function () {
  chatSessionId = null;
  chatMessages = [];
  chatStreaming = false;
  // Clear tracked expand states for the old session
  for (const key in _toolExpandState) delete _toolExpandState[key];
  localStorage.removeItem("claw_session_id");
  const hash = location.hash.slice(1);
  if (hash === "/chat") renderChat($("#content"));
};

window.resumeSession = function (sessionId) {
  chatSessionId = sessionId;
  chatMessages = [];
  chatStreaming = false;
  localStorage.setItem("claw_session_id", sessionId);
  location.hash = "#/chat";
};

window.handleChatKeydown = function (e) {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    sendChatMessage();
  }
};

window.autoResize = function (el) {
  el.style.height = "auto";
  el.style.height = Math.min(el.scrollHeight, 120) + "px";
};

window.sendChatMessage = async function () {
  if (chatStreaming) return;
  const input = $("#chat-input");
  const text = input.value.trim();
  if (!text) return;

  input.value = "";
  input.style.height = "auto";

  // Add user message
  chatMessages.push({ role: "user", text, id: `u${Date.now()}` });

  // Add placeholder assistant message with segment-based content model
  const assistantMsg = {
    role: "assistant",
    text: "", // accumulated raw text (kept for compatibility)
    segments: [], // ordered: text / tool_call / approval segments
    toolCalls: [], // lookup table for tool results by id
    pendingApprovals: [],
    id: `a${Date.now()}`,
    _currentTextSeg: null, // pointer to the current text segment being streamed
  };
  chatMessages.push(assistantMsg);

  // Re-render messages
  const msgsEl = $("#chat-messages");
  msgsEl.innerHTML = chatMessages.map(renderChatMessage).join("");
  msgsEl.scrollTop = msgsEl.scrollHeight;

  chatStreaming = true;
  $("#send-btn").disabled = true;

  try {
    const res = await fetch(`${API}/api/v1/chat/stream`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ message: text, session_id: chatSessionId }),
    });

    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop(); // Keep incomplete line

      for (const line of lines) {
        if (!line.startsWith("data: ")) continue;
        const data = line.slice(6).trim();
        if (!data) continue;

        try {
          const event = JSON.parse(data);
          handleStreamEvent(event, assistantMsg);
        } catch {}
      }
    }
  } catch (err) {
    assistantMsg.segments.push({
      type: "text",
      content: `\n\n⚠️ Error: ${err.message}`,
    });
    assistantMsg._currentTextSeg = null;
  }

  chatStreaming = false;
  const sendBtn = $("#send-btn");
  if (sendBtn) sendBtn.disabled = false;

  // Final re-render
  const msgsEl2 = $("#chat-messages");
  if (msgsEl2) {
    msgsEl2.innerHTML = chatMessages.map(renderChatMessage).join("");
    chatAutoScroll();
  }
};

function handleStreamEvent(event, assistantMsg) {
  switch (event.type) {
    case "session":
      chatSessionId = event.session_id;
      localStorage.setItem("claw_session_id", chatSessionId);
      const label = $("#chat-session-label");
      if (label) label.textContent = `Session: ${chatSessionId.slice(0, 8)}…`;
      break;

    case "text": {
      // Append to current text segment, or start a new one
      if (assistantMsg._currentTextSeg) {
        assistantMsg._currentTextSeg.content += event.content;
      } else {
        const seg = { type: "text", content: event.content };
        assistantMsg.segments.push(seg);
        assistantMsg._currentTextSeg = seg;
      }
      assistantMsg.text += event.content; // keep raw text for compat
      updateStreamingBubble(assistantMsg);
      break;
    }

    case "thinking":
      // Could show thinking indicator
      break;

    case "tool_call": {
      // A tool call breaks the current text segment
      assistantMsg._currentTextSeg = null;
      const tcSeg = {
        type: "tool_call",
        name: event.name,
        id: event.id,
        result: null,
      };
      assistantMsg.segments.push(tcSeg);
      assistantMsg.toolCalls.push(tcSeg); // same object ref for result lookup

      // Try appending just the new tool card instead of full re-render
      const bubble = $(`#msg-${assistantMsg.id}`);
      if (bubble) {
        // Remove streaming cursor before appending
        const cursor = bubble.querySelector(".streaming-cursor");
        if (cursor) cursor.remove();
        // Append new tool card + fresh cursor
        const fragment = document.createElement("div");
        fragment.innerHTML =
          renderToolCallSegment(tcSeg, assistantMsg.id, true) +
          '<span class="streaming-cursor">▊</span>';
        while (fragment.firstChild) bubble.appendChild(fragment.firstChild);
        chatAutoScroll();
      } else {
        updateStreamingBubble(assistantMsg);
      }
      break;
    }

    case "tool_result": {
      const tc = assistantMsg.toolCalls.find((t) => t.id === event.id);
      if (tc) {
        tc.result = event.is_error ? `❌ ${event.content}` : event.content;
        // Extract screenshot URL from structured data (preferred) or regex fallback
        if (event.data && event.data.screenshot_url) {
          tc.screenshot_url = event.data.screenshot_url;
        }
        // Try targeted update of just this tool card to avoid disrupting user interaction
        const resultId = `tr-${tc.id}`;
        const existingCard = document.getElementById(`tool-result-${resultId}`);
        if (existingCard) {
          // Replace just the tool call card in-place
          const toolCallEl = existingCard.closest(".msg-tool-call");
          if (toolCallEl) {
            const newHtml = renderToolCallSegment(tc, assistantMsg.id, true);
            toolCallEl.outerHTML = newHtml;
            chatAutoScroll();
            break;
          }
        }
      }
      updateStreamingBubble(assistantMsg);
      break;
    }

    case "approval_required": {
      assistantMsg._currentTextSeg = null;
      const approval = {
        type: "approval",
        id: event.id,
        tool_name: event.tool_name,
        tool_args: event.tool_args,
        reason: event.reason,
        risk_level: event.risk_level,
        resolved: false,
        resolution: null,
      };
      assistantMsg.segments.push(approval);
      if (!assistantMsg.pendingApprovals) assistantMsg.pendingApprovals = [];
      assistantMsg.pendingApprovals.push(approval); // same ref
      updateStreamingBubble(assistantMsg);
      break;
    }

    case "usage":
      // Could display token count
      break;

    case "done":
    case "error":
      if (event.message) {
        assistantMsg._currentTextSeg = null;
        assistantMsg.segments.push({
          type: "text",
          content: `\n\n⚠️ ${event.message}`,
        });
      }
      break;
  }
}

function updateStreamingBubble(msg) {
  const bubble = $(`#msg-${msg.id}`);
  if (!bubble) return;

  // If the last segment is a text delta and it's the only thing that changed,
  // try a targeted update of just the last text block to avoid nuking expanded tool states.
  const lastSeg = msg.segments[msg.segments.length - 1];
  if (lastSeg && lastSeg.type === "text" && lastSeg === msg._currentTextSeg) {
    // Find existing text blocks — if the count matches segments, just update the last one
    const textBlocks = bubble.querySelectorAll(".msg-text-block");
    const textSegCount = msg.segments.filter((s) => s.type === "text").length;
    if (textBlocks.length === textSegCount && textBlocks.length > 0) {
      const lastBlock = textBlocks[textBlocks.length - 1];
      lastBlock.innerHTML = renderMarkdown(lastSeg.content);
      chatAutoScroll();
      return;
    }
  }

  // Full re-render — preserve tool expand states via _toolExpandState
  let content = "";
  for (const seg of msg.segments) {
    if (seg.type === "text") {
      content += `<div class="msg-text-block">${renderMarkdown(seg.content)}</div>`;
    } else if (seg.type === "tool_call") {
      content += renderToolCallSegment(seg, msg.id, true);
    } else if (seg.type === "approval") {
      content += renderApprovalSegment(seg, msg.id);
    }
  }

  // Add cursor while streaming
  if (chatStreaming) {
    content += '<span class="streaming-cursor">▊</span>';
  }

  bubble.innerHTML = content;
  chatAutoScroll();
}

// ── Sessions Page ─────────────────────────────────────────────

async function renderSessions(el) {
  try {
    const data = await api("/api/v1/sessions");
    const sessions = data.sessions || [];

    el.innerHTML = `
        <div class="page">
            <div class="page-header">
                <div>
                    <h1 class="page-title">Sessions</h1>
                    <p class="page-subtitle">${sessions.length} session${sessions.length !== 1 ? "s" : ""}</p>
                </div>
            </div>
            ${
              sessions.length === 0
                ? `
                <div class="empty-state">
                    <div class="empty-icon">💬</div>
                    <div class="empty-text">No sessions yet</div>
                </div>
            `
                : `
                <div class="table-wrap">
                    <table>
                        <thead>
                            <tr>
                                <th>Session</th>
                                <th>Status</th>
                                <th>Messages</th>
                                <th>Created</th>
                            </tr>
                        </thead>
                        <tbody>
                            ${sessions
                              .map(
                                (s) => `
                                <tr class="clickable-row" onclick="resumeSession('${escHtml(s.id)}')" title="Click to resume this session">
                                    <td class="mono">${s.name ? escHtml(s.name) : escHtml(s.id).slice(0, 12) + "…"}</td>
                                    <td>${
                                      s.active
                                        ? '<span class="badge badge-success">Active</span>'
                                        : '<span class="badge badge-warning">Idle</span>'
                                    }</td>
                                    <td>${s.message_count}</td>
                                    <td>${formatDate(s.created_at)}</td>
                                </tr>
                            `,
                              )
                              .join("")}
                        </tbody>
                    </table>
                </div>
            `
            }
        </div>`;
  } catch (err) {
    el.innerHTML = `<div class="page"><div class="empty-state"><div class="empty-icon">⚠️</div><div class="empty-text">${escHtml(err.message)}</div></div></div>`;
  }
}

// ── Goals Page ────────────────────────────────────────────────

async function renderGoals(el) {
  try {
    const data = await api("/api/v1/goals");
    const goals = data.goals || [];
    const active = goals.filter((g) => g.status === "Active");
    const completed = goals.filter((g) => g.status === "Completed");
    const other = goals.filter(
      (g) => g.status !== "Active" && g.status !== "Completed",
    );

    function goalStatusBadge(status) {
      switch (status) {
        case "Completed":
          return '<span class="badge badge-success">✅ Completed</span>';
        case "Failed":
          return '<span class="badge badge-danger">❌ Failed</span>';
        case "Cancelled":
          return '<span class="badge badge-danger">❌ Cancelled</span>';
        case "Paused":
          return '<span class="badge" style="background:var(--surface-hover);color:var(--text-secondary);">⏸ Paused</span>';
        default:
          return '<span class="badge badge-accent">Active</span>';
      }
    }

    function renderGoalCard(g) {
      const pct = Math.round(g.progress * 100);
      const color = pct >= 100 ? "success" : pct > 50 ? "accent" : "";
      const title = g.title || g.description || "(untitled)";
      const isInactive =
        g.status === "Completed" ||
        g.status === "Cancelled" ||
        g.status === "Failed";
      return `
                <div class="goal-card${isInactive ? " goal-inactive" : ""}">
                    <div class="goal-header">
                        <span class="goal-title">${escHtml(title)}</span>
                        <div style="display:flex; gap:8px; align-items:center;">
                            ${goalStatusBadge(g.status)}
                            <span class="badge badge-accent">P${g.priority}</span>
                            <span style="font-size:13px; color:var(--text-secondary);">${pct}%</span>
                        </div>
                    </div>
                    <div class="progress-bar">
                        <div class="progress-fill ${color}" style="width: ${pct}%"></div>
                    </div>
                    ${
                      g.steps && g.steps.length > 0
                        ? `
                        <ul class="goal-steps">
                            ${g.steps
                              .map((s) => {
                                const icon =
                                  s.status === "Completed"
                                    ? "✅"
                                    : s.status === "InProgress"
                                      ? "🔄"
                                      : s.status === "Failed"
                                        ? "❌"
                                        : "⬜";
                                return `<li class="goal-step"><span class="step-icon">${icon}</span> ${escHtml(s.description)}</li>`;
                              })
                              .join("")}
                        </ul>
                    `
                        : ""
                    }
                </div>`;
    }

    el.innerHTML = `
        <div class="page">
            <div class="page-header">
                <div>
                    <h1 class="page-title">Goals</h1>
                    <p class="page-subtitle">${goals.length} goal${goals.length !== 1 ? "s" : ""} · ${active.length} active · ${completed.length} completed</p>
                </div>
            </div>
            ${
              goals.length === 0
                ? `
                <div class="empty-state">
                    <div class="empty-icon">🎯</div>
                    <div class="empty-text">No goals yet — ask Claw to create one in chat</div>
                </div>
            `
                : `
                ${active.length > 0 ? `<h3 style="margin:16px 0 8px;color:var(--text-secondary);font-size:13px;text-transform:uppercase;letter-spacing:1px;">Active</h3>` + active.map(renderGoalCard).join("") : ""}
                ${completed.length > 0 ? `<h3 style="margin:16px 0 8px;color:var(--text-secondary);font-size:13px;text-transform:uppercase;letter-spacing:1px;">Completed</h3>` + completed.map(renderGoalCard).join("") : ""}
                ${other.length > 0 ? `<h3 style="margin:16px 0 8px;color:var(--text-secondary);font-size:13px;text-transform:uppercase;letter-spacing:1px;">Other</h3>` + other.map(renderGoalCard).join("") : ""}
                `
            }
        </div>`;
  } catch (err) {
    el.innerHTML = `<div class="page"><div class="empty-state"><div class="empty-icon">⚠️</div><div class="empty-text">${escHtml(err.message)}</div></div></div>`;
  }
}

// ── Memory Page ───────────────────────────────────────────────

async function renderMemory(el) {
  try {
    const data = await api("/api/v1/memory/facts");
    const facts = data.facts || [];

    el.innerHTML = `
        <div class="page">
            <div class="page-header">
                <div>
                    <h1 class="page-title">Memory</h1>
                    <p class="page-subtitle">${facts.length} stored fact${facts.length !== 1 ? "s" : ""}</p>
                </div>
            </div>

            <div class="search-box">
                <input class="search-input" id="memory-search" type="text"
                       placeholder="Search memory…" onkeydown="if(event.key==='Enter') searchMemory()">
                <button class="btn btn-primary" onclick="searchMemory()">Search</button>
            </div>

            <div id="memory-search-results"></div>

            ${
              facts.length === 0
                ? `
                <div class="empty-state">
                    <div class="empty-icon">🧠</div>
                    <div class="empty-text">No facts stored yet — ask Claw to remember something</div>
                </div>
            `
                : `
                <div class="table-wrap">
                    <table>
                        <thead>
                            <tr>
                                <th>Category</th>
                                <th>Key</th>
                                <th>Value</th>
                                <th>Confidence</th>
                            </tr>
                        </thead>
                        <tbody>
                            ${facts
                              .map(
                                (f) => `
                                <tr>
                                    <td><span class="badge badge-info">${escHtml(f.category)}</span></td>
                                    <td class="mono">${escHtml(f.key)}</td>
                                    <td>${escHtml(f.value)}</td>
                                    <td>${Math.round(f.confidence * 100)}%</td>
                                </tr>
                            `,
                              )
                              .join("")}
                        </tbody>
                    </table>
                </div>
            `
            }
        </div>`;
  } catch (err) {
    el.innerHTML = `<div class="page"><div class="empty-state"><div class="empty-icon">⚠️</div><div class="empty-text">${escHtml(err.message)}</div></div></div>`;
  }
}

window.searchMemory = async function () {
  const input = $("#memory-search");
  const q = input.value.trim();
  if (!q) return;

  const resultsEl = $("#memory-search-results");
  resultsEl.innerHTML =
    '<div class="loading"><div class="spinner"></div></div>';

  try {
    const data = await api(`/api/v1/memory/search?q=${encodeURIComponent(q)}`);
    const results = data.results || [];

    if (results.length === 0) {
      resultsEl.innerHTML = `<div style="padding:16px 0; color:var(--text-muted);">No results for "${escHtml(q)}"</div>`;
    } else {
      resultsEl.innerHTML = `
            <div style="margin-bottom: 16px;">
                <div style="font-size:13px; color:var(--text-secondary); margin-bottom:8px;">${results.length} result${results.length !== 1 ? "s" : ""} for "${escHtml(q)}"</div>
                <div class="table-wrap">
                    <table>
                        <thead><tr><th>Type</th><th>Content</th></tr></thead>
                        <tbody>
                            ${results
                              .map((r) => {
                                if (r.type === "fact") {
                                  return `<tr>
                                        <td><span class="badge badge-info">Fact</span></td>
                                        <td><strong>${escHtml(r.key)}</strong>: ${escHtml(r.value)} <span style="color:var(--text-muted)">(${escHtml(r.category)})</span></td>
                                    </tr>`;
                                } else {
                                  return `<tr>
                                        <td><span class="badge badge-accent">Episode</span></td>
                                        <td>${escHtml(r.summary)} ${r.outcome ? `→ ${escHtml(r.outcome)}` : ""}</td>
                                    </tr>`;
                                }
                              })
                              .join("")}
                        </tbody>
                    </table>
                </div>
            </div>`;
    }
  } catch (err) {
    resultsEl.innerHTML = `<div style="color:var(--error);">Error: ${escHtml(err.message)}</div>`;
  }
};

// ── Tools Page ────────────────────────────────────────────────

async function renderTools(el) {
  try {
    const data = await api("/api/v1/tools");
    const tools = data.tools || [];

    el.innerHTML = `
        <div class="page">
            <div class="page-header">
                <div>
                    <h1 class="page-title">Tools</h1>
                    <p class="page-subtitle">${tools.length} available tool${tools.length !== 1 ? "s" : ""}</p>
                </div>
            </div>
            <div class="card-grid">
                ${tools
                  .map(
                    (t) => `
                    <div class="tool-card">
                        <div class="tool-name">${escHtml(t.name)}</div>
                        <div class="tool-desc">${escHtml(t.description)}</div>
                        <div class="tool-meta">
                            ${riskBadge(t.risk_level)}
                            ${t.is_mutating ? '<span class="badge badge-warning">Mutating</span>' : '<span class="badge badge-success">Read-only</span>'}
                            ${(t.capabilities || []).map((c) => `<span class="badge badge-info">${escHtml(c)}</span>`).join("")}
                        </div>
                    </div>
                `,
                  )
                  .join("")}
            </div>
        </div>`;
  } catch (err) {
    el.innerHTML = `<div class="page"><div class="empty-state"><div class="empty-icon">⚠️</div><div class="empty-text">${escHtml(err.message)}</div></div></div>`;
  }
}

// ── Skills Hub Page ───────────────────────────────────────────

async function renderHub(el) {
  // Render skeleton immediately
  el.innerHTML = `
    <div class="page">
      <div class="page-header">
        <div>
          <h1 class="page-title">Skills Hub</h1>
          <p class="page-subtitle">Discover, publish, and install reusable skills from the central hub</p>
        </div>
        <div class="page-actions">
          <span class="hub-connection-badge" id="hub-connection">⏳ Connecting…</span>
          <button class="btn btn-primary" id="hub-publish-btn">+ Publish Skill</button>
        </div>
      </div>

      <!-- Not-connected banner (hidden by default) -->
      <div class="hub-no-connection" id="hub-no-connection" style="display:none">
        <div class="empty-state">
          <div class="empty-icon">🌐</div>
          <div class="empty-text">No Skills Hub connected</div>
          <p style="color:var(--text-secondary);font-size:13px;margin-top:8px">
            Set <code>services.hub_url</code> in your <code>claw.toml</code> to connect to a hub,<br>
            or run <code>claw hub serve</code> to host your own.
          </p>
        </div>
      </div>

      <!-- Hub content (hidden until connected) -->
      <div id="hub-content" style="display:none">
        <!-- Stats bar -->
        <div class="hub-stats-bar" id="hub-stats-bar">
          <div class="stat-card"><div class="stat-label">Total Skills</div><div class="stat-value" id="hub-stat-total">—</div></div>
          <div class="stat-card"><div class="stat-label">Total Downloads</div><div class="stat-value" id="hub-stat-downloads">—</div></div>
          <div class="stat-card"><div class="stat-label">Top Tags</div><div class="stat-value" id="hub-stat-tags" style="font-size:13px;color:var(--text-secondary)">—</div></div>
        </div>

        <!-- Search & Filters -->
        <div class="hub-search-bar">
          <input type="text" id="hub-search" class="search-input" placeholder="Search skills by name, description, or tag…" />
          <select id="hub-sort" class="input" style="width:auto">
            <option value="updated">Recently Updated</option>
            <option value="downloads">Most Downloaded</option>
            <option value="name">Alphabetical</option>
          </select>
          <button class="btn btn-primary" id="hub-search-btn">Search</button>
        </div>

        <!-- Tag filter chips -->
        <div class="hub-tags" id="hub-tags"></div>

      <!-- Skills grid -->
      <div class="card-grid" id="hub-skills-grid">
        <div class="loading"><div class="spinner"></div></div>
      </div>
      </div><!-- /hub-content -->

      <!-- Publish modal -->
      <div class="modal-overlay" id="hub-publish-modal" style="display:none">
        <div class="modal">
          <div class="modal-header">
            <h2>Publish Skill</h2>
            <button class="modal-close" id="hub-modal-close">&times;</button>
          </div>
          <div class="modal-body">
            <p style="margin-bottom:12px;color:var(--text-secondary);font-size:13px">
              Paste the full TOML content of your skill definition below.
              The hub will parse it, validate the schema, and publish it.
            </p>
            <textarea id="hub-toml-input" class="hub-toml-textarea" rows="18" placeholder="# Paste your skill .toml here&#10;[skill]&#10;name = &quot;my_skill&quot;&#10;description = &quot;…&quot;&#10;version = &quot;1.0.0&quot;&#10;&#10;[[steps]]&#10;…"></textarea>
            <div id="hub-publish-error" class="hub-publish-error" style="display:none"></div>
          </div>
          <div class="modal-footer">
            <button class="btn" id="hub-modal-cancel">Cancel</button>
            <button class="btn btn-primary" id="hub-modal-submit">Publish</button>
          </div>
        </div>
      </div>

      <!-- Detail modal -->
      <div class="modal-overlay" id="hub-detail-modal" style="display:none">
        <div class="modal modal-lg">
          <div class="modal-header">
            <h2 id="hub-detail-title">Skill Detail</h2>
            <button class="modal-close" id="hub-detail-close">&times;</button>
          </div>
          <div class="modal-body" id="hub-detail-body"></div>
          <div class="modal-footer">
            <button class="btn" id="hub-detail-cancel">Close</button>
            <button class="btn btn-primary" id="hub-detail-pull">Pull Skill</button>
            <button class="btn btn-danger" id="hub-detail-delete" style="margin-left:auto">Delete</button>
          </div>
        </div>
      </div>
    </div>`;

  // ── Wire up event handlers ──
  const searchInput = $("#hub-search");
  const sortSel = $("#hub-sort");
  const searchBtn = $("#hub-search-btn");
  const publishBtn = $("#hub-publish-btn");
  const publishModal = $("#hub-publish-modal");
  const detailModal = $("#hub-detail-modal");

  let currentTag = null;

  async function loadStats() {
    try {
      const stats = await api("/api/v1/hub/stats");
      $("#hub-stat-total").textContent = stats.total_skills;
      $("#hub-stat-downloads").textContent = stats.total_downloads;
      const tagsEl = $("#hub-stat-tags");
      if (stats.top_tags && stats.top_tags.length > 0) {
        tagsEl.innerHTML = stats.top_tags
          .slice(0, 8)
          .map(
            (t) =>
              `<span class="badge badge-accent" style="cursor:pointer;margin:2px" data-tag="${escHtml(t.tag)}">${escHtml(t.tag)} (${t.count})</span>`,
          )
          .join(" ");
      } else {
        tagsEl.textContent = "No tags yet";
      }
      // Render tag chips bar
      const tagsBar = $("#hub-tags");
      if (stats.top_tags && stats.top_tags.length > 0) {
        tagsBar.innerHTML = stats.top_tags
          .map(
            (t) =>
              `<span class="hub-tag-chip ${currentTag === t.tag ? "active" : ""}" data-tag="${escHtml(t.tag)}">${escHtml(t.tag)}</span>`,
          )
          .join("");
      }
    } catch {
      // Stats are optional, ignore errors
    }
  }

  async function loadSkills() {
    const grid = $("#hub-skills-grid");
    grid.innerHTML = '<div class="loading"><div class="spinner"></div></div>';

    try {
      const q = searchInput.value.trim();
      const sort = sortSel.value;
      let url;

      if (q || currentTag) {
        const params = new URLSearchParams();
        if (q) params.set("q", q);
        if (currentTag) params.set("tag", currentTag);
        params.set("sort", sort);
        url = `/api/v1/hub/skills/search?${params}`;
      } else {
        url = `/api/v1/hub/skills?sort=${sort}&limit=100`;
      }

      const data = await api(url);
      const skills = data.skills || [];

      if (skills.length === 0) {
        grid.innerHTML = `
          <div class="empty-state" style="grid-column:1/-1">
            <div class="empty-icon">📦</div>
            <div class="empty-text">No skills found${q ? " matching your search" : ""}. Publish your first skill!</div>
          </div>`;
        return;
      }

      grid.innerHTML = skills
        .map(
          (s) => `
        <div class="hub-skill-card" data-skill="${escHtml(s.name)}">
          <div class="hub-skill-header">
            <div class="hub-skill-name">${escHtml(s.name)}</div>
            <div class="hub-skill-version">${escHtml(s.version)}</div>
          </div>
          <div class="hub-skill-desc">${escHtml(s.description)}</div>
          <div class="hub-skill-tags">
            ${s.tags.map((t) => `<span class="badge badge-info">${escHtml(t)}</span>`).join("")}
          </div>
          <div class="hub-skill-footer">
            <span class="hub-skill-meta">
              ${riskBadge(s.risk_level)}
              <span class="badge badge-accent">${s.steps_count} step${s.steps_count !== 1 ? "s" : ""}</span>
            </span>
            <span class="hub-skill-downloads">⬇ ${s.downloads}</span>
          </div>
          ${s.author ? `<div class="hub-skill-author">by ${escHtml(s.author)}</div>` : ""}
        </div>
      `,
        )
        .join("");

      // Click handler for skill cards
      grid.querySelectorAll(".hub-skill-card").forEach((card) => {
        card.addEventListener("click", () =>
          showSkillDetail(card.dataset.skill),
        );
      });
    } catch (err) {
      grid.innerHTML = `<div class="empty-state" style="grid-column:1/-1"><div class="empty-icon">⚠️</div><div class="empty-text">${escHtml(err.message)}</div></div>`;
    }
  }

  async function showSkillDetail(name) {
    try {
      const skill = await api(`/api/v1/hub/skills/${encodeURIComponent(name)}`);
      $("#hub-detail-title").textContent = skill.name;
      $("#hub-detail-body").innerHTML = `
        <div class="hub-detail-grid">
          <div class="hub-detail-info">
            <div class="hub-detail-row"><strong>Description:</strong> ${escHtml(skill.description)}</div>
            <div class="hub-detail-row"><strong>Version:</strong> ${escHtml(skill.version)}</div>
            <div class="hub-detail-row"><strong>Author:</strong> ${escHtml(skill.author || "—")}</div>
            <div class="hub-detail-row"><strong>Risk Level:</strong> ${riskBadge(skill.risk_level)}</div>
            <div class="hub-detail-row"><strong>Steps:</strong> ${skill.steps_count}</div>
            <div class="hub-detail-row"><strong>Downloads:</strong> ${skill.downloads}</div>
            <div class="hub-detail-row"><strong>Published:</strong> ${formatDate(skill.published_at)}</div>
            <div class="hub-detail-row"><strong>Updated:</strong> ${formatDate(skill.updated_at)}</div>
            ${skill.tags.length > 0 ? `<div class="hub-detail-row"><strong>Tags:</strong> ${skill.tags.map((t) => `<span class="badge badge-info">${escHtml(t)}</span>`).join(" ")}</div>` : ""}
            ${
              skill.parameters.length > 0
                ? `
              <div class="hub-detail-row"><strong>Parameters:</strong></div>
              <div class="hub-params-list">
                ${skill.parameters
                  .map(
                    (p) => `
                  <div class="hub-param">
                    <code>${escHtml(p.name)}</code>
                    ${p.required ? '<span class="badge badge-error">required</span>' : '<span class="badge badge-success">optional</span>'}
                    <span class="hub-param-desc">${escHtml(p.description)}</span>
                  </div>
                `,
                  )
                  .join("")}
              </div>
            `
                : ""
            }
          </div>
          <div class="hub-detail-toml">
            <div class="hub-detail-toml-header">
              <strong>Skill Definition (TOML)</strong>
              <button class="btn" id="hub-copy-toml" style="font-size:11px;padding:2px 8px">Copy</button>
            </div>
            <pre class="hub-toml-pre"><code>${escHtml(skill.toml_content)}</code></pre>
          </div>
        </div>`;

      // Copy button
      const copyBtn = $("#hub-copy-toml");
      if (copyBtn) {
        copyBtn.addEventListener("click", () => {
          navigator.clipboard.writeText(skill.toml_content).then(() => {
            copyBtn.textContent = "Copied!";
            setTimeout(() => (copyBtn.textContent = "Copy"), 1500);
          });
        });
      }

      // Pull button
      const pullBtn = $("#hub-detail-pull");
      pullBtn.onclick = async () => {
        pullBtn.disabled = true;
        pullBtn.textContent = "Pulling…";
        try {
          const result = await api(
            `/api/v1/hub/skills/${encodeURIComponent(name)}/pull`,
            { method: "POST" },
          );
          pullBtn.textContent = `✓ Pulled v${result.version}`;
          pullBtn.classList.add("btn-success");
          loadStats();
          loadSkills();
        } catch (err) {
          pullBtn.textContent = "Pull Failed";
          pullBtn.classList.add("btn-danger");
        }
      };

      // Delete button
      const deleteBtn = $("#hub-detail-delete");
      deleteBtn.onclick = async () => {
        if (!confirm(`Delete skill "${name}" from the hub?`)) return;
        deleteBtn.disabled = true;
        try {
          await api(`/api/v1/hub/skills/${encodeURIComponent(name)}`, {
            method: "DELETE",
          });
          detailModal.style.display = "none";
          loadStats();
          loadSkills();
        } catch (err) {
          alert("Delete failed: " + err.message);
          deleteBtn.disabled = false;
        }
      };

      detailModal.style.display = "flex";
    } catch (err) {
      alert("Failed to load skill: " + err.message);
    }
  }

  // Search / sort handlers
  searchBtn.addEventListener("click", loadSkills);
  searchInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") loadSkills();
  });
  sortSel.addEventListener("change", loadSkills);

  // Tag click handlers (in stats and tag bar)
  el.addEventListener("click", (e) => {
    const tagEl = e.target.closest("[data-tag]");
    if (tagEl && !tagEl.closest(".modal-overlay")) {
      const tag = tagEl.dataset.tag;
      currentTag = currentTag === tag ? null : tag;
      // Update active states
      el.querySelectorAll(".hub-tag-chip").forEach((c) => {
        c.classList.toggle("active", c.dataset.tag === currentTag);
      });
      loadSkills();
    }
  });

  // Publish modal
  publishBtn.addEventListener("click", () => {
    publishModal.style.display = "flex";
    $("#hub-toml-input").value = "";
    $("#hub-publish-error").style.display = "none";
  });
  $("#hub-modal-close").addEventListener(
    "click",
    () => (publishModal.style.display = "none"),
  );
  $("#hub-modal-cancel").addEventListener(
    "click",
    () => (publishModal.style.display = "none"),
  );
  publishModal.addEventListener("click", (e) => {
    if (e.target === publishModal) publishModal.style.display = "none";
  });

  $("#hub-modal-submit").addEventListener("click", async () => {
    const toml = $("#hub-toml-input").value.trim();
    const errEl = $("#hub-publish-error");
    if (!toml) {
      errEl.textContent = "Please paste the TOML content of your skill.";
      errEl.style.display = "block";
      return;
    }
    const submitBtn = $("#hub-modal-submit");
    submitBtn.disabled = true;
    submitBtn.textContent = "Publishing…";
    errEl.style.display = "none";

    try {
      const result = await api("/api/v1/hub/skills", {
        method: "POST",
        body: JSON.stringify({ toml_content: toml }),
      });
      publishModal.style.display = "none";
      submitBtn.disabled = false;
      submitBtn.textContent = "Publish";
      loadStats();
      loadSkills();
    } catch (err) {
      errEl.textContent = "Publish failed: " + err.message;
      errEl.style.display = "block";
      submitBtn.disabled = false;
      submitBtn.textContent = "Publish";
    }
  });

  // Detail modal close
  $("#hub-detail-close").addEventListener(
    "click",
    () => (detailModal.style.display = "none"),
  );
  $("#hub-detail-cancel").addEventListener(
    "click",
    () => (detailModal.style.display = "none"),
  );
  detailModal.addEventListener("click", (e) => {
    if (e.target === detailModal) detailModal.style.display = "none";
  });

  // Initial load — check hub connectivity first
  async function checkHubConnection() {
    const badge = $("#hub-connection");
    const noConn = $("#hub-no-connection");
    const content = $("#hub-content");
    try {
      await api("/api/v1/hub/stats");
      badge.textContent = "🟢 Connected";
      badge.classList.add("connected");
      noConn.style.display = "none";
      content.style.display = "";
      loadStats();
      loadSkills();
    } catch {
      badge.textContent = "🔴 Not connected";
      badge.classList.add("disconnected");
      noConn.style.display = "";
      content.style.display = "none";
    }
  }
  checkHubConnection();
}

// ── Logs / Audit Page ─────────────────────────────────────────

let logsAutoRefresh = null;

async function renderLogs(el) {
  // Clear any previous auto-refresh
  if (logsAutoRefresh) {
    clearInterval(logsAutoRefresh);
    logsAutoRefresh = null;
  }

  el.innerHTML = `
    <div class="page">
      <div class="page-header">
        <div>
          <h1 class="page-title">Audit Log</h1>
          <p class="page-subtitle">Security and system events</p>
        </div>
        <div class="page-actions">
          <select id="log-limit" class="input" style="width:auto">
            <option value="50">Last 50</option>
            <option value="100" selected>Last 100</option>
            <option value="250">Last 250</option>
            <option value="500">Last 500</option>
          </select>
          <label style="display:flex;align-items:center;gap:6px;font-size:0.85rem;color:var(--text-secondary)">
            <input type="checkbox" id="log-auto-refresh" /> Auto-refresh
          </label>
          <button class="btn btn-primary" id="log-refresh-btn">Refresh</button>
        </div>
      </div>
      <div id="log-filter-bar" style="display:flex;gap:8px;margin-bottom:16px;flex-wrap:wrap">
        <input type="text" id="log-search" class="input" placeholder="Filter by keyword…" style="flex:1;min-width:200px" />
        <select id="log-type-filter" class="input" style="width:auto">
          <option value="">All types</option>
        </select>
      </div>
      <div id="log-table-wrap"></div>
      <div id="log-count" style="margin-top:8px;font-size:0.8rem;color:var(--text-secondary)"></div>
    </div>`;

  const limitSel = $("#log-limit");
  const searchInput = $("#log-search");
  const typeFilter = $("#log-type-filter");
  const autoCheck = $("#log-auto-refresh");

  async function loadLogs() {
    const limit = limitSel.value;
    try {
      const data = await api(`/api/v1/audit?limit=${limit}`);
      const entries = data.audit_log || [];
      renderLogTable(entries);
    } catch (err) {
      $("#log-table-wrap").innerHTML =
        `<div class="empty-state"><div class="empty-icon">⚠️</div><div class="empty-text">${escHtml(err.message)}</div></div>`;
    }
  }

  function renderLogTable(entries) {
    // Populate type filter options
    const types = [
      ...new Set(entries.map((e) => e.event_type).filter(Boolean)),
    ];
    const curType = typeFilter.value;
    typeFilter.innerHTML =
      '<option value="">All types</option>' +
      types
        .map(
          (t) =>
            `<option value="${escHtml(t)}"${t === curType ? " selected" : ""}>${escHtml(t)}</option>`,
        )
        .join("");

    // Apply filters
    const search = searchInput.value.toLowerCase();
    const typeVal = typeFilter.value;
    let filtered = entries;
    if (typeVal) {
      filtered = filtered.filter((e) => e.event_type === typeVal);
    }
    if (search) {
      filtered = filtered.filter(
        (e) =>
          (e.action || "").toLowerCase().includes(search) ||
          (e.event_type || "").toLowerCase().includes(search) ||
          (e.details || "").toLowerCase().includes(search) ||
          (e.timestamp || "").toLowerCase().includes(search),
      );
    }

    $("#log-count").textContent =
      `Showing ${filtered.length} of ${entries.length} entries`;

    if (filtered.length === 0) {
      $("#log-table-wrap").innerHTML =
        '<div class="empty-state"><div class="empty-icon">📋</div><div class="empty-text">No audit log entries</div></div>';
      return;
    }

    const rows = filtered
      .map((e) => {
        const ts = e.timestamp ? new Date(e.timestamp).toLocaleString() : "—";
        const badge = eventTypeBadge(e.event_type);
        const details = e.details
          ? `<span class="log-details">${escHtml(truncate(e.details, 120))}</span>`
          : '<span class="text-secondary">—</span>';
        return `<tr>
        <td class="log-ts">${escHtml(ts)}</td>
        <td>${badge}</td>
        <td>${escHtml(e.action || "—")}</td>
        <td>${details}</td>
      </tr>`;
      })
      .join("");

    $("#log-table-wrap").innerHTML = `
      <div class="table-responsive">
        <table class="table">
          <thead><tr><th style="width:170px">Timestamp</th><th style="width:130px">Type</th><th style="width:200px">Action</th><th>Details</th></tr></thead>
          <tbody>${rows}</tbody>
        </table>
      </div>`;
  }

  function eventTypeBadge(type) {
    const colors = {
      tool_execution: "var(--accent)",
      tool_denied: "#e74c3c",
      message: "#3498db",
      approval: "#f39c12",
      session: "#2ecc71",
      goal: "#9b59b6",
      injection: "#e74c3c",
      budget: "#e67e22",
      config: "#1abc9c",
    };
    const t = (type || "unknown").toLowerCase();
    const color =
      Object.entries(colors).find(([k]) => t.includes(k))?.[1] ||
      "var(--text-secondary)";
    return `<span class="badge" style="background:${color}20;color:${color};border:1px solid ${color}40">${escHtml(type || "unknown")}</span>`;
  }

  function truncate(s, n) {
    return s.length > n ? s.slice(0, n) + "…" : s;
  }

  // Event listeners
  $("#log-refresh-btn").addEventListener("click", loadLogs);
  limitSel.addEventListener("change", loadLogs);
  searchInput.addEventListener("input", loadLogs);
  typeFilter.addEventListener("change", loadLogs);
  autoCheck.addEventListener("change", () => {
    if (autoCheck.checked) {
      logsAutoRefresh = setInterval(loadLogs, 5000);
    } else {
      clearInterval(logsAutoRefresh);
      logsAutoRefresh = null;
    }
  });

  // Initial load
  await loadLogs();
}

// ── Sub-Agents Page ───────────────────────────────────────────

async function renderSubAgents(el) {
  try {
    const data = await api("/api/v1/sub-tasks");
    const tasks = data.sub_tasks || [];
    const running = data.running || 0;
    const completed = data.completed || 0;
    const failed = data.failed || 0;

    function statusBadge(status) {
      switch (status) {
        case "completed":
          return '<span class="badge badge-success">Completed</span>';
        case "running":
          return '<span class="badge badge-warning">Running</span>';
        case "failed":
          return '<span class="badge badge-error">Failed</span>';
        case "pending":
          return '<span class="badge badge-info">Pending</span>';
        case "waiting_for_deps":
          return '<span class="badge badge-accent">Waiting</span>';
        default:
          return `<span class="badge">${escHtml(status)}</span>`;
      }
    }

    function formatElapsed(secs) {
      if (secs == null) return "—";
      if (secs < 60) return `${secs.toFixed(1)}s`;
      return `${Math.floor(secs / 60)}m ${Math.round(secs % 60)}s`;
    }

    el.innerHTML = `
        <div class="page">
            <div class="page-header">
                <div>
                    <h1 class="page-title">Sub-Agents</h1>
                    <p class="page-subtitle">${tasks.length} task${tasks.length !== 1 ? "s" : ""} — ${running} running, ${completed} completed, ${failed} failed</p>
                </div>
                <button class="btn btn-primary" onclick="refreshSubAgents()">↻ Refresh</button>
            </div>

            ${
              tasks.length === 0
                ? `
                <div class="empty-state">
                    <div class="empty-icon">🤖</div>
                    <div class="empty-text">No sub-agent tasks — delegate work in chat with "spawn agents to…"</div>
                </div>
            `
                : `
                <div class="card-grid">
                    ${tasks
                      .map((t) => {
                        const depList =
                          t.depends_on && t.depends_on.length > 0
                            ? `<div class="agent-deps"><span class="deps-label">Depends on:</span> ${t.depends_on.map((d) => `<span class="badge">${escHtml(d).slice(0, 8)}…</span>`).join(" ")}</div>`
                            : "";
                        const resultPreview = t.result
                          ? `<div class="agent-result"><span class="result-label">Result:</span> <span class="result-text">${escHtml(t.result.length > 200 ? t.result.slice(0, 200) + "…" : t.result)}</span></div>`
                          : "";
                        const errorMsg = t.error
                          ? `<div class="agent-error">${escHtml(t.error)}</div>`
                          : "";

                        return `
                    <div class="card agent-card agent-status-${escHtml(t.status)}">
                        <div class="agent-card-header">
                            <span class="agent-role">${escHtml(t.role || "agent")}</span>
                            ${statusBadge(t.status)}
                        </div>
                        <div class="agent-task-desc">${escHtml(t.task_description)}</div>
                        <div class="agent-meta">
                            <span class="agent-id" title="${escHtml(t.task_id)}">ID: ${escHtml(t.task_id).slice(0, 8)}…</span>
                            <span class="agent-elapsed">⏱ ${formatElapsed(t.elapsed_secs)}</span>
                        </div>
                        ${depList}
                        ${resultPreview}
                        ${errorMsg}
                    </div>`;
                      })
                      .join("")}
                </div>
            `
            }
        </div>`;
  } catch (err) {
    el.innerHTML = `<div class="page"><div class="empty-state"><div class="empty-icon">⚠️</div><div class="empty-text">${escHtml(err.message)}</div></div></div>`;
  }
}

window.refreshSubAgents = function () {
  const content = $("#content");
  content.innerHTML = '<div class="loading"><div class="spinner"></div></div>';
  renderSubAgents(content);
};

// ── Scheduler Page ────────────────────────────────────────────

async function renderScheduler(el) {
  try {
    const data = await api("/api/v1/scheduled-tasks");
    const tasks = data.scheduled_tasks || [];
    const activeCount = data.active || 0;

    function kindBadge(kind) {
      if (!kind) return '<span class="badge">Unknown</span>';
      if (kind.type === "cron") {
        return `<span class="badge badge-accent" title="Cron expression: ${escHtml(kind.expression || "")}">⏰ Cron</span>`;
      }
      if (kind.type === "one_shot") {
        return `<span class="badge badge-info" title="Fires at: ${kind.fire_at || ""}">🎯 One-Shot</span>`;
      }
      return `<span class="badge">${escHtml(kind.type)}</span>`;
    }

    function scheduleDetail(kind) {
      if (!kind) return "—";
      if (kind.type === "cron")
        return `<code>${escHtml(kind.expression || "")}</code>`;
      if (kind.type === "one_shot") return formatDate(kind.fire_at);
      return "—";
    }

    el.innerHTML = `
        <div class="page">
            <div class="page-header">
                <div>
                    <h1 class="page-title">Scheduler</h1>
                    <p class="page-subtitle">${tasks.length} task${tasks.length !== 1 ? "s" : ""} — ${activeCount} active</p>
                </div>
                <button class="btn btn-primary" onclick="refreshScheduler()">↻ Refresh</button>
            </div>

            ${
              tasks.length === 0
                ? `
                <div class="empty-state">
                    <div class="empty-icon">⏰</div>
                    <div class="empty-text">No scheduled tasks — ask Claw to "remind me every…" or "run X in 5 minutes"</div>
                </div>
            `
                : `
                <div class="table-wrap">
                    <table>
                        <thead>
                            <tr>
                                <th>Label</th>
                                <th>Type</th>
                                <th>Schedule</th>
                                <th>Status</th>
                                <th>Fires</th>
                                <th>Last Fired</th>
                                <th>Created</th>
                            </tr>
                        </thead>
                        <tbody>
                            ${tasks
                              .map(
                                (t) => `
                                <tr>
                                    <td>
                                        <div class="sched-label">${escHtml(t.label)}</div>
                                        ${t.description ? `<div class="sched-desc">${escHtml(t.description.length > 80 ? t.description.slice(0, 80) + "…" : t.description)}</div>` : ""}
                                    </td>
                                    <td>${kindBadge(t.kind)}</td>
                                    <td class="mono">${scheduleDetail(t.kind)}</td>
                                    <td>${
                                      t.active
                                        ? '<span class="badge badge-success">Active</span>'
                                        : '<span class="badge badge-error">Inactive</span>'
                                    }</td>
                                    <td class="mono">${t.fire_count ?? 0}</td>
                                    <td>${formatDate(t.last_fired)}</td>
                                    <td>${formatDate(t.created_at)}</td>
                                </tr>
                            `,
                              )
                              .join("")}
                        </tbody>
                    </table>
                </div>
            `
            }
        </div>`;
  } catch (err) {
    el.innerHTML = `<div class="page"><div class="empty-state"><div class="empty-icon">⚠️</div><div class="empty-text">${escHtml(err.message)}</div></div></div>`;
  }
}

window.refreshScheduler = function () {
  const content = $("#content");
  content.innerHTML = '<div class="loading"><div class="spinner"></div></div>';
  renderScheduler(content);
};

// ── Settings Page ─────────────────────────────────────────────

async function renderSettings(el) {
  try {
    const config = await api("/api/v1/config");

    el.innerHTML = `
        <div class="page">
            <div class="page-header">
                <div>
                    <h1 class="page-title">Settings</h1>
                    <p class="page-subtitle">Current runtime configuration (read-only)</p>
                </div>
            </div>
            <div class="config-block">
                <pre>${syntaxHighlight(JSON.stringify(config, null, 2))}</pre>
            </div>
        </div>`;
  } catch (err) {
    el.innerHTML = `<div class="page"><div class="empty-state"><div class="empty-icon">⚠️</div><div class="empty-text">${escHtml(err.message)}</div></div></div>`;
  }
}

function syntaxHighlight(json) {
  return json.replace(
    /("(\\u[\da-fA-F]{4}|\\[^u]|[^\\"])*"(\s*:)?|\b(true|false|null)\b|-?\d+(?:\.\d*)?(?:[eE][+-]?\d+)?)/g,
    (match) => {
      let cls = "number";
      if (/^"/.test(match)) {
        cls = /:$/.test(match) ? "key" : "string";
      } else if (/true|false/.test(match)) {
        cls = "boolean";
      }
      return `<span class="${cls}">${match}</span>`;
    },
  );
}

// ── Streaming Cursor CSS ──────────────────────────────────────
const style = document.createElement("style");
style.textContent = `
.streaming-cursor {
    display: inline-block;
    animation: blink 1s step-end infinite;
    color: var(--accent);
    font-weight: 300;
    margin-left: 1px;
}
@keyframes blink { 50% { opacity: 0; } }
`;
document.head.appendChild(style);

// ── Server-push notifications (cron results, etc.) ───────────

let _notificationSource = null;

function connectNotifications() {
  if (_notificationSource) {
    _notificationSource.close();
  }
  _notificationSource = new EventSource(`${API}/api/v1/events`);

  _notificationSource.onmessage = function (e) {
    if (!e.data || e.data.startsWith(":")) return; // skip keepalive comments
    try {
      const notification = JSON.parse(e.data);
      handleNotification(notification);
    } catch (_) {}
  };

  _notificationSource.onerror = function () {
    // Reconnect after a delay (EventSource auto-reconnects, but add safety)
    setTimeout(() => {
      if (
        _notificationSource &&
        _notificationSource.readyState === EventSource.CLOSED
      ) {
        connectNotifications();
      }
    }, 5000);
  };
}

function handleNotification(notification) {
  if (notification.type === "cron_result") {
    const label = notification.label || "Scheduled task";
    const text = notification.text || "";
    const sessionId = notification.session_id || "";

    // Add as a system notification in the chat messages
    chatMessages.push({
      role: "assistant",
      text: `⏰ **${label}**\n\n${text}`,
      id: `cron-${Date.now()}`,
      segments: [{ type: "text", content: `⏰ **${label}**\n\n${text}` }],
      isCronNotification: true,
      cronSessionId: sessionId,
    });

    // Re-render chat if we're on the chat page
    const msgsEl = $("#chat-messages");
    if (msgsEl && !chatStreaming) {
      msgsEl.innerHTML = chatMessages.map(renderChatMessage).join("");
      chatAutoScroll();
    }
  }
}

// ── Init ──────────────────────────────────────────────────────

window.addEventListener("hashchange", navigate);
window.addEventListener("DOMContentLoaded", () => {
  navigate();
  // Periodic status check
  setInterval(fetchStatus, 15000);
  // Connect to server-push notifications
  connectNotifications();
});
