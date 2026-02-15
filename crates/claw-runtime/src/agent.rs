use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use tokio::sync::{Mutex as TokioMutex, mpsc, oneshot};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Global handle so the HTTP server can reach the running agent runtime.
static RUNTIME_HANDLE: LazyLock<TokioMutex<Option<RuntimeHandle>>> =
    LazyLock::new(|| TokioMutex::new(None));

/// Get the global runtime handle (used by the server).
pub async fn get_runtime_handle() -> Option<RuntimeHandle> {
    RUNTIME_HANDLE.lock().await.clone()
}

/// Set the global runtime handle (used for testing).
pub async fn set_runtime_handle(handle: RuntimeHandle) {
    RUNTIME_HANDLE.lock().await.replace(handle);
}

use claw_autonomy::{
    ApprovalGate, ApprovalResponse, AutonomyLevel, BudgetTracker, GoalPlanner, GuardrailEngine,
};
use claw_channels::adapter::{
    Channel, ChannelEvent, OutgoingMessage,
};
use claw_config::ClawConfig;
use claw_core::{Event, EventBus};
use claw_llm::{LlmProvider, ModelRouter};
use claw_memory::MemoryStore;
use claw_mesh::{MeshMessage, MeshNode};
use claw_plugin::PluginHost;
use claw_skills::SkillRegistry;

use crate::scheduler::SchedulerHandle;
use crate::session::SessionManager;
use crate::tools::BuiltinTools;
use claw_device::DeviceTools;

// Re-import functions extracted to sub-modules so call sites in run() and tests compile.
use crate::agent_loop::{process_api_message, process_channel_message, process_mesh_message, process_stream_message};
use crate::channel_helpers::{resolve_approval, send_response_shared};
use crate::sub_agent::persist_task_to_db;

/// The default system prompt — concise, principle-based. Behavioral guidance
/// lives in tool descriptions so the model gets context *at the point of use*.
/// Use `build_default_system_prompt()` which injects runtime OS/arch/hostname.
const DEFAULT_SYSTEM_PROMPT_TEMPLATE: &str = r#"You are Claw 🦞, an autonomous AI agent running on the user's device.

## Environment
- **OS:** {os}
- **Arch:** {arch}
- **Hostname:** {hostname}

You take action using tools — files, shell, terminals, web, memory, goals, mesh delegation. Act, don't just talk about acting.

## Principles

- **Tools first, text after.** Every response should start with tool calls. Do NOT output long summaries, plans, or descriptions before you've taken action. However, for complex tasks, your FIRST tool calls should be research and discovery (http_fetch, web_search, file_list, browser_navigate) to understand the problem before generating code. Output brief status text only after all work in a turn is done.
- **Write production-quality content.** Each file you create should be complete, detailed, and polished. A landing page component needs real headings, real paragraphs, real structure — not 3-line placeholders. Write as if this is shipping to a real client. If you can't fit everything in one turn, write fewer files but make each one excellent — the system will loop back for more.
- **Explore first.** Discover project structure (file_list, file_find) before writing code. Don't guess paths.
- **Research before building.** When the user provides a URL or references an existing website/service/repo, ALWAYS fetch and study it first with `http_fetch` or `browser_navigate` + `browser_screenshot` before writing any code. Understand the source material (layout, sections, content, style) so your output is informed, not invented. When rebuilding or cloning a site, capture its structure, navigation, copy, color palette, and key sections. Never skip this step — the user gave you the URL for a reason.
- **Be thorough.** Complex tasks need many tool calls across many turns. Don't stop early. Finish the job.
- **Brief text, rich code.** Keep explanatory text under 100 words. Pour the detail into your code and file content instead.
- **Diagnose and retry on errors.** Read error messages, fix the cause, try again.
- **Scaffolding is step 1, not the finish line.** After `npx create-*` or `npm init`, explore what it created, then write the real application code on top of the skeleton.
- **Shell first for native apps.** You are running on {os}. Use OS-native shell commands to control installed applications (e.g. `osascript` on macOS, `dbus-send`/`playerctl` on Linux, PowerShell on Windows). Only fall back to opening a browser URL when a native approach doesn't exist.
- **Check before you act.** Before running a tool command, verify the target is available: use `which <binary>`, `command -v`, `pgrep`, or `ls` to confirm an app/tool is installed and running. If a device tool (android_*, ios_*) is needed, check connectivity first (e.g. `adb devices`, `xcrun simctl list`). Don't blindly run commands that will fail.
- **Use mesh delegation** for tasks requiring capabilities on remote peers you don't have locally.
- **Sub-agents ARE your task delegation system.** When the user asks you to "delegate", "split work", "test task delegation", or any complex multi-step project, use `sub_agent_spawn` to assign work to specialized sub-agents (planner, coder, reviewer, tester, devops, researcher). Sub-agents are independent AI workers that run in parallel on this machine — they are NOT mesh peers. This is how you delegate work.
- **Default to delegation for complex work.** For any task with 2+ distinct parts (e.g., "build a website", "set up a project", "research and implement"), spawn sub-agents for each part rather than doing everything sequentially. Each sub-agent gets its own session, tools, and role. Use `depends_on` to chain agents that need each other's output. You are an **orchestrator** — your job is to decompose, delegate, and synthesize results.\n- **Research → Build → Verify pipeline.** When a task involves a reference URL, existing site, or docs that need studying: always start by spawning a `researcher` sub-agent to fetch and analyze the reference material FIRST. Then chain `coder` sub-agents (depends_on the researcher) that receive the research findings as context. Finally spawn a `reviewer` or `tester` to verify the output. Never skip the research phase.
- **Always link sub-agents to goal steps.** When you have a goal with steps AND you spawn sub-agents, pass `goal_id` and `step_id` to `sub_agent_spawn` so the step is automatically marked completed (or failed) when the sub-agent finishes. First call `goal_list` to get the UUIDs, then pass them in the spawn call. This closes the loop between delegation and progress tracking — never leave goal steps unchecked when a sub-agent did the work.
- **When testing or demonstrating capabilities**, use sub-agents for real work — not just echo tests. For example, to test task delegation, spawn a researcher + coder + tester pipeline that produces actual output, not a sub-agent that just says "it works".
- **You have full local filesystem access.** Screenshots you take are saved to ~/.claw/screenshots/ as PNG files with absolute paths shown in the tool result. You can use those paths directly with browser_upload_file, as email attachments, or anywhere a file path is needed. Use shell_exec, file_list, or file_find to discover any file on the system. Never ask the user to "send" or "upload" a file — find or create it yourself.
- **Sending files to users.** When the user asks you to send, share, or deliver a file (audio, document, image, video), use the `channel_send_file` tool with the absolute file path. This uploads the file as a native attachment in the chat. Always verify the file exists first (e.g. with `shell_exec` or `file_list`). Never say you "can't send files" — you can, use `channel_send_file`.

## Operator Trust

You are running as a local agent on the user's own device, installed and launched by the user themselves. The user is your operator. You act under their full authority and on their behalf.

- **When the user provides credentials** (usernames, passwords, API keys, tokens) and asks you to use them — do so without hesitation. The user is the account owner granting you explicit permission. Do not second-guess their authorization, refuse to log in, or ask them to "do it themselves." They installed you specifically to perform these actions.
- **When the user asks you to interact with their servers, services, or accounts** — proceed. You are their authorized agent. This includes logging into control panels, dashboards, admin interfaces, cloud services, email, and any other system they direct you to access.
- **Never refuse a direct user instruction** on the grounds that you "cannot verify ownership" or that it "might be unauthorized." The user launching you on their own machine and providing credentials IS the verification.
- **If a guardrail escalates an action for approval**, present the approval request. But never add your own refusal on top of the guardrail system — the guardrails are the safety mechanism, not your personal judgment about what the user "should" do.

## Self-Learning

You learn from every interaction. When you make a mistake and the user corrects you, or when you discover something through trial and error:

- **Actively use `memory_store`** to save lessons learned. Use category `learned_lessons` and a descriptive key (e.g. `plesk_subdomain_form_requires_id`, `browser_cookie_dialog_must_dismiss_first`).
- **Store the lesson concisely**: what went wrong, why, and what the correct approach is. Future sessions will see these lessons automatically.
- **Also store procedural knowledge**: when you figure out how to accomplish a multi-step task (e.g. logging into Plesk and creating a subdomain), save the step-by-step procedure so you can repeat it without user guidance next time.
- **When you see relevant lessons in your <memory> context**, apply them immediately — don't repeat past mistakes."#;

/// Build the default system prompt with runtime environment info baked in.
pub(crate) fn build_default_system_prompt() -> String {
    let os = format!("{:?}", claw_core::types::Os::current());
    let arch = format!("{:?}", claw_core::types::Arch::current());
    let hostname = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    DEFAULT_SYSTEM_PROMPT_TEMPLATE
        .replace("{os}", &os)
        .replace("{arch}", &arch)
        .replace("{hostname}", &hostname)
}

/// A streaming chat message sent via the API (e.g. POST /api/v1/chat/stream).
/// Contains an mpsc sender for streaming chunks back.
pub struct StreamApiMessage {
    pub text: String,
    pub session_id: Option<String>,
    pub chunk_tx: mpsc::Sender<StreamEvent>,
}

/// A chunk sent back during a streaming response.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    #[serde(rename = "session")]
    Session { session_id: String },
    #[serde(rename = "text")]
    TextDelta { content: String },
    #[serde(rename = "thinking")]
    Thinking { content: String },
    #[serde(rename = "tool_call")]
    ToolCall {
        name: String,
        id: String,
        #[serde(default)]
        args: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
    #[serde(rename = "usage")]
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        cost_usd: f64,
    },
    #[serde(rename = "done")]
    Done,
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "approval_required")]
    ApprovalRequired {
        id: String,
        tool_name: String,
        tool_args: serde_json::Value,
        reason: String,
        risk_level: u8,
    },
}

/// Shared map of pending approval requests.
pub type PendingApprovals = Arc<TokioMutex<HashMap<Uuid, oneshot::Sender<ApprovalResponse>>>>;

/// Pending mesh task delegation — awaiting TaskResult from a peer.
pub type PendingMeshTasks = Arc<TokioMutex<HashMap<Uuid, oneshot::Sender<MeshTaskResult>>>>;

/// A server-push notification for connected web UI clients.
/// Broadcast via `RuntimeHandle::notification_tx`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum Notification {
    /// A scheduled task produced output.
    #[serde(rename = "cron_result")]
    CronResult {
        task_id: String,
        label: String,
        text: String,
        session_id: String,
    },
    /// A generic info message from the runtime.
    #[serde(rename = "info")]
    Info { message: String },
}

/// The result of a delegated mesh task.
#[derive(Debug, Clone)]
pub struct MeshTaskResult {
    pub task_id: Uuid,
    pub peer_id: String,
    pub success: bool,
    pub result: String,
}

/// Pending sub-agent tasks — awaiting completion from spawned sub-agents.
pub type PendingSubTasks = Arc<TokioMutex<HashMap<Uuid, SubTaskState>>>;

/// The state of a sub-agent task.
#[derive(Debug, Clone)]
pub struct SubTaskState {
    pub task_id: Uuid,
    pub role: String,
    pub task_description: String,
    pub status: SubTaskStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub parent_session_id: Uuid,
    pub depends_on: Vec<Uuid>,
    pub created_at: std::time::Instant,
    /// If this sub-agent is linked to a goal step, auto-complete it on finish.
    pub goal_id: Option<Uuid>,
    pub step_id: Option<Uuid>,
}

/// Status of a sub-agent task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubTaskStatus {
    /// Waiting for dependency tasks to complete.
    WaitingForDeps,
    /// Queued and ready to run.
    Pending,
    /// Currently executing.
    Running,
    /// Finished successfully.
    Completed,
    /// Finished with error.
    Failed,
}

/// Shared agent state — cheaply cloneable for concurrent task spawning.
/// Components with interior mutability (SessionManager, BudgetTracker, EventBus) clone directly.
/// Heavy-mutation components (MemoryStore, GoalPlanner) are behind Arc<TokioMutex<>>.
/// Read-mostly components (ModelRouter, GuardrailEngine, PluginHost) are behind Arc.
#[derive(Clone)]
pub struct SharedAgentState {
    pub config: ClawConfig,
    pub llm: Arc<ModelRouter>,
    pub tools: BuiltinTools,
    pub sessions: SessionManager,
    pub budget: BudgetTracker,
    pub guardrails: Arc<GuardrailEngine>,
    pub approval: Arc<ApprovalGate>,
    pub plugins: Arc<PluginHost>,
    pub skills: Arc<TokioMutex<SkillRegistry>>,
    pub event_bus: EventBus,
    pub memory: Arc<TokioMutex<MemoryStore>>,
    pub planner: Arc<TokioMutex<GoalPlanner>>,
    pub channels: Arc<TokioMutex<Vec<Box<dyn Channel>>>>,
    pub embedder: Option<Arc<dyn claw_llm::EmbeddingProvider>>,
    pub mesh: Arc<TokioMutex<MeshNode>>,
    pub pending_mesh_tasks: PendingMeshTasks,
    pub pending_sub_tasks: PendingSubTasks,
    pub scheduler: Option<SchedulerHandle>,
    pub device_tools: Arc<DeviceTools>,
    /// Current channel context for tool calls that need to send back to the user.
    /// Set at the start of process_message_streaming_shared, cleared at the end.
    /// Tuple of (channel_id, target).
    pub reply_context: Arc<TokioMutex<Option<(String, String)>>>,
    /// Active stream sender for forwarding sub-agent events to the parent stream.
    /// Set at the start of process_message_streaming_shared or process_channel_message.
    pub stream_tx: Arc<TokioMutex<Option<mpsc::Sender<StreamEvent>>>>,
}

/// The response sent back to the API caller.
pub struct ApiResponse {
    pub text: String,
    pub session_id: String,
    pub error: Option<String>,
}

/// A handle for sending messages into the running agent runtime.
/// Given to the HTTP server so API requests can reach the agent.
#[derive(Clone)]
pub struct RuntimeHandle {
    state: SharedAgentState,
    stream_tx: mpsc::Sender<StreamApiMessage>,
    pending_approvals: PendingApprovals,
    started_at: std::time::Instant,
    notification_tx: tokio::sync::broadcast::Sender<Notification>,
}

impl RuntimeHandle {
    /// Create a RuntimeHandle for testing (no background stream processor).
    pub fn new_for_test(state: SharedAgentState) -> Self {
        let (stream_tx, _stream_rx) = mpsc::channel(64);
        let (notification_tx, _) = tokio::sync::broadcast::channel(64);
        Self {
            state,
            stream_tx,
            pending_approvals: Arc::new(TokioMutex::new(HashMap::new())),
            started_at: std::time::Instant::now(),
            notification_tx,
        }
    }

    /// Get a reference to the shared agent state.
    pub fn state(&self) -> &SharedAgentState {
        &self.state
    }

    /// Approve a pending approval request.
    pub async fn approve(&self, id: Uuid) -> Result<(), String> {
        let tx = self
            .pending_approvals
            .lock()
            .await
            .remove(&id)
            .ok_or_else(|| "approval request not found or already resolved".to_string())?;
        let _ = tx.send(ApprovalResponse::Approved);
        Ok(())
    }

    /// Deny a pending approval request.
    pub async fn deny(&self, id: Uuid) -> Result<(), String> {
        let tx = self
            .pending_approvals
            .lock()
            .await
            .remove(&id)
            .ok_or_else(|| "approval request not found or already resolved".to_string())?;
        let _ = tx.send(ApprovalResponse::Denied);
        Ok(())
    }

    /// List pending approval requests (IDs only — details are in the stream events).
    pub async fn pending_approval_count(&self) -> usize {
        self.pending_approvals.lock().await.len()
    }

    /// Send a non-streaming chat message — spawns a concurrent task.
    pub async fn chat(
        &self,
        text: String,
        session_id: Option<String>,
    ) -> Result<ApiResponse, String> {
        let state = self.state.clone();
        let handle =
            tokio::spawn(async move { process_api_message(state, text, session_id).await });
        handle.await.map_err(|e| format!("task panicked: {e}"))
    }

    /// Send a streaming chat message. Returns a receiver for stream events.
    pub async fn chat_stream(
        &self,
        text: String,
        session_id: Option<String>,
    ) -> Result<mpsc::Receiver<StreamEvent>, String> {
        let (chunk_tx, chunk_rx) = mpsc::channel(256);
        let msg = StreamApiMessage {
            text,
            session_id,
            chunk_tx,
        };
        self.stream_tx
            .send(msg)
            .await
            .map_err(|_| "runtime is not running".to_string())?;
        Ok(chunk_rx)
    }

    /// Query runtime for status/sessions/goals — reads shared state directly, never blocks.
    pub async fn query(&self, kind: crate::query::QueryKind) -> Result<serde_json::Value, String> {
        crate::query::handle_query(&self.state, kind, self.started_at).await
    }

    /// Subscribe to server-push notifications (cron results, etc.).
    pub fn subscribe_notifications(&self) -> tokio::sync::broadcast::Receiver<Notification> {
        self.notification_tx.subscribe()
    }

    /// Broadcast a notification to all connected clients.
    pub fn notify(&self, notification: Notification) {
        let _ = self.notification_tx.send(notification);
    }
}

/// The agent runtime — orchestrates the entire system.
pub struct AgentRuntime {
    config: ClawConfig,
    llm: ModelRouter,
    memory: MemoryStore,
    sessions: SessionManager,
    guardrails: GuardrailEngine,
    budget: BudgetTracker,
    planner: GoalPlanner,
    approval: ApprovalGate,
    plugins: PluginHost,
    tools: BuiltinTools,
    channels: Vec<Box<dyn Channel>>,
    event_bus: EventBus,
}

impl AgentRuntime {
    /// Create a new agent runtime from configuration.
    pub fn new(config: ClawConfig) -> claw_core::Result<Self> {
        info!("initializing agent runtime");

        // Resolve memory db_path relative to ~/.claw/ if it's not absolute
        let db_path = if config.memory.db_path.is_absolute() {
            config.memory.db_path.clone()
        } else {
            let config_base = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".claw");
            config_base.join(&config.memory.db_path)
        };

        // Initialize memory store
        let mut memory = MemoryStore::open(&db_path)?;

        // Load persisted facts into semantic memory
        match memory.load_facts() {
            Ok(n) if n > 0 => info!(count = n, "loaded persisted facts into semantic memory"),
            Ok(_) => {}
            Err(e) => warn!(error = %e, "failed to load persisted facts"),
        }

        // Initialize autonomy subsystems
        let mut guardrails = GuardrailEngine::new();
        guardrails.set_allowlist(config.autonomy.tool_allowlist.clone());
        guardrails.set_denylist(config.autonomy.tool_denylist.clone());

        let budget = BudgetTracker::new(
            config.autonomy.daily_budget_usd,
            config.autonomy.max_tool_calls_per_loop,
        );

        // Load persisted goals
        let mut planner = GoalPlanner::new();
        match memory.load_goals() {
            Ok(rows) => {
                let count = rows.len();
                for row in rows {
                    if let Ok(id) = row.id.parse::<Uuid>() {
                        let parent_id = row
                            .parent_id
                            .as_deref()
                            .and_then(|s| s.parse::<Uuid>().ok());
                        planner.restore_goal(
                            id,
                            row.description,
                            &row.status,
                            row.priority,
                            row.progress,
                            parent_id,
                            row.steps
                                .into_iter()
                                .map(|s| {
                                    let step_id =
                                        s.id.parse::<Uuid>().unwrap_or_else(|_| Uuid::new_v4());
                                    (step_id, s.description, s.status, s.result)
                                })
                                .collect(),
                        );
                    }
                }
                if count > 0 {
                    info!(count, "loaded persisted goals from SQLite");
                }
            }
            Err(e) => warn!(error = %e, "failed to load persisted goals"),
        }

        // Initialize plugin host
        let plugins = PluginHost::new(&config.plugins.plugin_dir)?;

        // Session persistence — sessions will be restored in run() since we need async
        let sessions = SessionManager::new();

        Ok(Self {
            config: config.clone(),
            llm: ModelRouter::new(),
            memory,
            sessions,
            guardrails,
            budget,
            planner,
            approval: ApprovalGate::new(),
            plugins,
            tools: BuiltinTools::new(),
            channels: Vec::new(),
            event_bus: EventBus::default(),
        })
    }

    /// Register an LLM provider.
    pub fn add_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        self.llm.add_provider(provider);
    }

    /// Register a channel adapter.
    pub fn add_channel(&mut self, channel: Box<dyn Channel>) {
        info!(
            channel = channel.channel_type(),
            id = channel.id(),
            "registered channel"
        );
        self.channels.push(channel);
    }

    /// Get the event bus for subscribing to system events.
    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    /// Get the approval gate (for the server to handle approval requests).
    pub fn approval_gate(&mut self) -> &mut ApprovalGate {
        &mut self.approval
    }

    /// Start the runtime — launches all channels and the main processing loop.
    /// Message handlers are spawned as concurrent tasks so the runtime never blocks.
    pub async fn run(mut self) -> claw_core::Result<()> {
        info!(
            model = %self.config.agent.model,
            autonomy = %AutonomyLevel::from_u8(self.config.autonomy.level),
            "starting agent runtime"
        );

        // Clean up stale empty sessions from previous runs
        match self.memory.cleanup_empty_sessions() {
            Ok(deleted) if deleted > 0 => info!(deleted, "cleaned up empty sessions from SQLite"),
            _ => {}
        }

        // Restore persisted sessions and their messages
        match self.memory.load_sessions(500) {
            Ok(rows) => {
                let mut restored = 0;
                for row in rows {
                    if let Ok(id) = row.id.parse::<Uuid>() {
                        // Only restore sessions that have messages (skip stale empties)
                        if row.active && row.message_count > 0 {
                            self.sessions
                                .restore(id, row.name, row.channel, row.target, row.message_count)
                                .await;
                            // Restore working memory messages for this session
                            match self.memory.load_session_messages(&id) {
                                Ok(messages) if !messages.is_empty() => {
                                    let msg_count = messages.len();
                                    let ctx = self.memory.working.session(id);
                                    ctx.messages = messages;
                                    ctx.estimated_tokens =
                                        ctx.messages.iter().map(|m| m.estimate_tokens()).sum();
                                    tracing::debug!(session = %id, messages = msg_count, "restored session messages");
                                }
                                _ => {}
                            }
                            restored += 1;
                        }
                    }
                }
                if restored > 0 {
                    info!(count = restored, "restored persisted sessions");
                }
            }
            Err(e) => warn!(error = %e, "failed to load persisted sessions"),
        }

        // Discover and load plugins
        match self.plugins.discover() {
            Ok(loaded) => info!(count = loaded.len(), "loaded plugins"),
            Err(e) => warn!(error = %e, "plugin discovery failed"),
        }

        // Discover and load skills (SKILL.md format)
        // Resolve skills_dir relative to config base (~/.claw/) for consistency
        let config_base = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".claw");
        let skills_dir = if self.config.plugins.plugin_dir.is_absolute() {
            self.config
                .plugins
                .plugin_dir
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("skills")
        } else {
            config_base.join("skills")
        };

        // Auto-seed bundled skills if the skills directory is empty or missing
        if !skills_dir.exists()
            || skills_dir
                .read_dir()
                .map(|mut d| d.next().is_none())
                .unwrap_or(true)
        {
            info!("seeding bundled skills into {}", skills_dir.display());
            let bundled: &[(&str, &str)] = &[
                (
                    "plesk-server",
                    include_str!("../../../skills/plesk-server/SKILL.md"),
                ),
                ("github", include_str!("../../../skills/github/SKILL.md")),
                ("docker", include_str!("../../../skills/docker/SKILL.md")),
                (
                    "server-management",
                    include_str!("../../../skills/server-management/SKILL.md"),
                ),
                ("coding", include_str!("../../../skills/coding/SKILL.md")),
                (
                    "web-research",
                    include_str!("../../../skills/web-research/SKILL.md"),
                ),
                (
                    "system-admin",
                    include_str!("../../../skills/system-admin/SKILL.md"),
                ),
                (
                    "1password",
                    include_str!("../../../skills/1password/SKILL.md"),
                ),
            ];
            for (name, content) in bundled {
                let skill_dir = skills_dir.join(name);
                if let Err(e) = std::fs::create_dir_all(&skill_dir) {
                    warn!(error = %e, skill = name, "failed to create skill directory");
                    continue;
                }
                let path = skill_dir.join("SKILL.md");
                if !path.exists() {
                    if let Err(e) = std::fs::write(&path, content) {
                        warn!(error = %e, skill = name, "failed to write bundled skill");
                    }
                }
            }
        }

        let mut skills = SkillRegistry::new_single(&skills_dir);
        match skills.discover() {
            Ok(loaded) => info!(count = loaded.len(), "loaded skills (SKILL.md)"),
            Err(e) => warn!(error = %e, "skill discovery failed"),
        }

        // Shared pending approvals map
        let pending_approvals: PendingApprovals = Arc::new(TokioMutex::new(HashMap::new()));

        // Take the approval receiver and spawn a background task that stores pending approvals
        if let Some(mut approval_rx) = self.approval.take_receiver() {
            let pa = pending_approvals.clone();
            tokio::spawn(async move {
                while let Some((request, response_tx)) = approval_rx.recv().await {
                    info!(
                        id = %request.id,
                        tool = %request.tool_name,
                        risk = request.risk_level,
                        "queuing approval request for API/UI"
                    );
                    pa.lock().await.insert(request.id, response_tx);
                }
            });
        }

        // Create an aggregate channel for all incoming channel messages
        let (aggregate_tx, mut aggregate_rx) = mpsc::channel::<(String, ChannelEvent)>(512);

        // Start all channels
        for channel in &mut self.channels {
            let channel_id = channel.id().to_string();
            match channel.start().await {
                Ok(mut event_rx) => {
                    let tx = aggregate_tx.clone();
                    let id = channel_id.clone();
                    tokio::spawn(async move {
                        while let Some(event) = event_rx.recv().await {
                            if tx.send((id.clone(), event)).await.is_err() {
                                break;
                            }
                        }
                    });
                    self.event_bus.publish(Event::ChannelConnected {
                        channel_id: channel_id.clone(),
                        channel_type: channel.channel_type().to_string(),
                    });
                }
                Err(e) => {
                    error!(channel = %channel_id, error = %e, "failed to start channel");
                }
            }
        }
        drop(aggregate_tx);

        // Track uptime
        let started_at = std::time::Instant::now();

        // Build the shared agent state — cheaply cloneable for concurrent tasks
        let (stream_tx, mut stream_rx) = mpsc::channel::<StreamApiMessage>(64);
        let mesh_node = MeshNode::new()?;
        let state = SharedAgentState {
            config: self.config.clone(),
            llm: Arc::new(self.llm),
            tools: self.tools,
            sessions: self.sessions,
            budget: self.budget,
            guardrails: Arc::new(self.guardrails),
            approval: Arc::new(self.approval),
            plugins: Arc::new(self.plugins),
            skills: Arc::new(TokioMutex::new(skills)),
            event_bus: self.event_bus.clone(),
            memory: Arc::new(TokioMutex::new(self.memory)),
            planner: Arc::new(TokioMutex::new(self.planner)),
            channels: Arc::new(TokioMutex::new(self.channels)),
            embedder: None,
            mesh: Arc::new(TokioMutex::new(mesh_node)),
            pending_mesh_tasks: Arc::new(TokioMutex::new(HashMap::new())),
            pending_sub_tasks: Arc::new(TokioMutex::new(HashMap::new())),
            scheduler: None, // Set after scheduler is created below
            device_tools: Arc::new(DeviceTools::new()),
            reply_context: Arc::new(TokioMutex::new(None)),
            stream_tx: Arc::new(TokioMutex::new(None)),
        };

        // ── Start mesh networking (if enabled) ─────────────────────
        let mut mesh_rx: Option<mpsc::Receiver<MeshMessage>> = None;
        if self.config.mesh.enabled {
            let mut mesh = state.mesh.lock().await;
            match mesh
                .start(
                    &self.config.mesh.listen,
                    &self.config.mesh.bootstrap_peers,
                    self.config.mesh.capabilities.clone(),
                )
                .await
            {
                Ok(rx) => {
                    info!(
                        peer_id = %mesh.peer_id(),
                        "mesh networking started"
                    );
                    mesh_rx = Some(rx);
                }
                Err(e) => {
                    warn!(error = %e, "mesh networking failed to start — continuing without mesh");
                }
            }
        }

        // ── Start scheduler ─────────────────────────────────────────
        let (scheduler, mut scheduler_rx) = crate::scheduler::CronScheduler::new();
        let scheduler_handle = scheduler.handle();

        // Load scheduled tasks from config (heartbeat_cron, goal crons)
        scheduler.load_from_config(&self.config).await;

        // Restore user-created scheduled tasks from SQLite
        {
            let mem = state.memory.lock().await;
            match mem.load_scheduled_tasks() {
                Ok(rows) => {
                    let mut restored = 0u32;
                    for row in rows {
                        // Skip inactive tasks (expired one-shots)
                        if !row.active {
                            continue;
                        }
                        let id = match uuid::Uuid::parse_str(&row.id) {
                            Ok(id) => id,
                            Err(_) => continue,
                        };
                        let kind: crate::scheduler::ScheduleKind =
                            match serde_json::from_str(&row.kind_json) {
                                Ok(k) => k,
                                Err(_) => continue,
                            };
                        let created_at = chrono::DateTime::parse_from_rfc3339(&row.created_at)
                            .map(|dt| dt.with_timezone(&chrono::Utc))
                            .unwrap_or_else(|_| chrono::Utc::now());
                        let last_fired = row.last_fired.as_deref().and_then(|s| {
                            chrono::DateTime::parse_from_rfc3339(s)
                                .map(|dt| dt.with_timezone(&chrono::Utc))
                                .ok()
                        });
                        let session_id = row
                            .session_id
                            .as_deref()
                            .and_then(|s| uuid::Uuid::parse_str(s).ok());

                        let task = crate::scheduler::ScheduledTask {
                            id,
                            label: row.label,
                            description: row.description,
                            kind,
                            created_at,
                            session_id,
                            active: true,
                            fire_count: row.fire_count,
                            last_fired,
                        };
                        scheduler_handle.restore_task(task).await;
                        restored += 1;
                    }
                    if restored > 0 {
                        info!(count = restored, "restored scheduled tasks from SQLite");
                    }
                }
                Err(e) => {
                    warn!(error = %e, "failed to load scheduled tasks from SQLite");
                }
            }
        }

        // Store the handle in shared state so tools can schedule tasks
        let mut state = state;
        state.scheduler = Some(scheduler_handle);

        // Spawn the scheduler background loop
        tokio::spawn(async move {
            scheduler.run().await;
        });

        // Publish the RuntimeHandle so the server can use it
        let (notification_tx, _) = tokio::sync::broadcast::channel(64);
        let handle = RuntimeHandle {
            state: state.clone(),
            stream_tx,
            pending_approvals: pending_approvals.clone(),
            started_at,
            notification_tx,
        };
        RUNTIME_HANDLE.lock().await.replace(handle);

        info!("agent runtime started, waiting for messages");

        // Spawn a background task to persist sessions + messages periodically
        {
            let state_for_persist = state.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    let sessions = state_for_persist.sessions.snapshot().await;
                    let mem = state_for_persist.memory.lock().await;
                    for session in &sessions {
                        // Only persist sessions that have activity
                        if session.message_count == 0 {
                            continue;
                        }
                        let _ = mem.persist_session(
                            &session.id,
                            session.name.as_deref(),
                            session.channel.as_deref(),
                            session.target.as_deref(),
                            session.active,
                            session.message_count,
                        );
                        // Persist working memory messages for active sessions
                        if session.active {
                            let messages = mem.working.messages(session.id);
                            if !messages.is_empty() {
                                let _ = mem.persist_session_messages(&session.id, messages);
                            }
                        }
                    }
                }
            });
        }

        // Main event loop — dispatches work to concurrent tasks so nothing blocks
        loop {
            tokio::select! {
                // Handle messages from channels (Telegram, Discord, etc.)
                Some((channel_id, event)) = aggregate_rx.recv() => {
                    match event {
                        ChannelEvent::Message(msg) => {
                            // Check for bot commands first
                            if let Some(ref text) = msg.text {
                                let trimmed = text.trim();

                                // /start — welcome message
                                if trimmed == "/start" || trimmed.starts_with("/start@") {
                                    let s = state.clone();
                                    let cid = channel_id.clone();
                                    let target = msg.group.as_deref().unwrap_or(&msg.sender).to_string();
                                    tokio::spawn(async move {
                                        let welcome = format!(
                                            "🦞 *Claw AI Agent*\n\n\
                                             I'm an autonomous AI agent running v{}.\n\n\
                                             Send me any message and I'll do my best to help.\n\n\
                                             Commands:\n\
                                             /new — start a new session\n\
                                             /status — show agent status\n\
                                             /help — show this help\n\
                                             /approve <id> — approve a pending action\n\
                                             /deny <id> — deny a pending action",
                                            env!("CARGO_PKG_VERSION"),
                                        );
                                        let _ = send_response_shared(&s, &cid, &target, &welcome).await;
                                    });
                                    continue;
                                }

                                // /help — same as start
                                if trimmed == "/help" || trimmed.starts_with("/help@") {
                                    let s = state.clone();
                                    let cid = channel_id.clone();
                                    let target = msg.group.as_deref().unwrap_or(&msg.sender).to_string();
                                    tokio::spawn(async move {
                                        let help = "🦞 *Claw Commands*\n\n\
                                             /new — start a new session (clear conversation)\n\
                                             /status — show agent status (model, uptime, budget)\n\
                                             /help — show this help\n\
                                             /approve <id> — approve a pending action\n\
                                             /deny <id> — deny a pending action\n\n\
                                             Or just send me a message and I'll respond!";
                                        let _ = send_response_shared(&s, &cid, &target, help).await;
                                    });
                                    continue;
                                }

                                // /new — start a new session (clear conversation)
                                if trimmed == "/new" || trimmed.starts_with("/new@") {
                                    let s = state.clone();
                                    let cid = channel_id.clone();
                                    let target = msg.group.as_deref().unwrap_or(&msg.sender).to_string();
                                    tokio::spawn(async move {
                                        // Find the current active session for this channel+target
                                        let old_session_id = s.sessions.find_or_create(&cid, &target).await;
                                        // Close the old session
                                        s.sessions.close(old_session_id).await;
                                        // Clear working memory for the old session
                                        s.memory.lock().await.working.clear(old_session_id);
                                        // Create a fresh session
                                        let _new_id = s.sessions.create_for_channel(&cid, &target).await;
                                        let reply = "🔄 New session started. Previous conversation cleared.";
                                        let _ = send_response_shared(&s, &cid, &target, reply).await;
                                    });
                                    continue;
                                }

                                // /status — show runtime status
                                if trimmed == "/status" || trimmed.starts_with("/status@") {
                                    let s = state.clone();
                                    let cid = channel_id.clone();
                                    let target = msg.group.as_deref().unwrap_or(&msg.sender).to_string();
                                    let st = started_at;
                                    let pa = pending_approvals.clone();
                                    tokio::spawn(async move {
                                        let budget = s.budget.snapshot();
                                        let sessions = s.sessions.active_count().await;
                                        let pending = pa.lock().await.len();
                                        let uptime = st.elapsed();
                                        let hours = uptime.as_secs() / 3600;
                                        let mins = (uptime.as_secs() % 3600) / 60;
                                        let status = format!(
                                            "🦞 *Claw Status*\n\n\
                                             📦 Version: {}\n\
                                             🤖 Model: {}\n\
                                             ⚡ Autonomy: L{}\n\
                                             ⏱ Uptime: {}h {}m\n\
                                             💰 Budget: ${:.2} / ${:.2}\n\
                                             📋 Sessions: {}\n\
                                             🔒 Pending approvals: {}",
                                            env!("CARGO_PKG_VERSION"),
                                            s.config.agent.model,
                                            s.config.autonomy.level,
                                            hours, mins,
                                            budget.daily_spend_usd, budget.daily_limit_usd,
                                            sessions,
                                            pending,
                                        );
                                        let _ = send_response_shared(&s, &cid, &target, &status).await;
                                    });
                                    continue;
                                }

                                // /approve or /deny — with or without UUID
                                if trimmed == "/approve" || trimmed == "/deny"
                                    || trimmed.starts_with("/approve ") || trimmed.starts_with("/deny ")
                                    || trimmed.starts_with("/approve@") || trimmed.starts_with("/deny@")
                                {
                                    let is_approve = trimmed.starts_with("/approve");
                                    // Extract UUID argument (if any) — skip the command word
                                    let uuid_arg: Option<String> = trimmed.split_once(' ').map(|x| x.1)
                                        .map(|s| s.trim().to_string())
                                        .filter(|s| !s.is_empty());

                                    let pa = pending_approvals.clone();
                                    let s = state.clone();
                                    let target = msg.group.as_deref().unwrap_or(&msg.sender).to_string();
                                    let cid = channel_id.clone();

                                    tokio::spawn(async move {
                                        let reply = match uuid_arg.as_deref() {
                                            Some(id_str) => {
                                                // User provided a specific UUID
                                                match id_str.parse::<Uuid>() {
                                                    Ok(id) => {
                                                        match resolve_approval(&pa, id, is_approve).await {
                                                            Ok(()) => if is_approve {
                                                                "✅ Approved — executing...".to_string()
                                                            } else {
                                                                "❌ Denied.".to_string()
                                                            },
                                                            Err(e) => format!("⚠️ {e}"),
                                                        }
                                                    }
                                                    Err(_) => "⚠️ Invalid approval ID. Use the buttons or type /approve <uuid>.".to_string(),
                                                }
                                            }
                                            None => {
                                                // No UUID — auto-resolve if there's exactly one pending
                                                let map = pa.lock().await;
                                                let count = map.len();
                                                if count == 0 {
                                                    "ℹ️ No pending approvals.".to_string()
                                                } else if count == 1 {
                                                    let id = *map.keys().next().unwrap();
                                                    drop(map); // release lock before resolve
                                                    match resolve_approval(&pa, id, is_approve).await {
                                                        Ok(()) => if is_approve {
                                                            "✅ Approved — executing...".to_string()
                                                        } else {
                                                            "❌ Denied.".to_string()
                                                        },
                                                        Err(e) => format!("⚠️ {e}"),
                                                    }
                                                } else {
                                                    let ids: Vec<String> = map.keys().map(|id| id.to_string()).collect();
                                                    drop(map);
                                                    format!("⚠️ {} pending approvals. Specify which:\n{}", count,
                                                        ids.iter().map(|id| format!("  /approve {id}")).collect::<Vec<_>>().join("\n"))
                                                }
                                            }
                                        };
                                        let _ = send_response_shared(&s, &cid, &target, &reply).await;
                                    });
                                    continue;
                                }
                            }

                            let s = state.clone();
                            tokio::spawn(async move {
                                if let Err(e) = process_channel_message(s, &channel_id, msg).await {
                                    error!(error = %e, "failed to handle channel message");
                                }
                            });
                        }
                        ChannelEvent::CallbackQuery { callback_id: _, data, sender: _, chat_id } => {
                            // Parse "approve:<uuid>" or "deny:<uuid>" from inline keyboard
                            if let Some((action, id_str)) = data.split_once(':') {
                                let is_approve = action == "approve";
                                if let Ok(id) = id_str.parse::<Uuid>() {
                                    let pa = pending_approvals.clone();
                                    let s = state.clone();
                                    tokio::spawn(async move {
                                        let result = resolve_approval(&pa, id, is_approve).await;
                                        let reply = match result {
                                            Ok(()) => if is_approve {
                                                "✅ Action approved — executing...".to_string()
                                            } else {
                                                "❌ Action denied.".to_string()
                                            },
                                            Err(e) => format!("⚠️ {e}"),
                                        };
                                        let _ = send_response_shared(&s, "telegram", &chat_id, &reply).await;
                                    });
                                }
                            }
                        }
                        ChannelEvent::Connected => {
                            info!(channel = %channel_id, "channel connected");
                        }
                        ChannelEvent::Disconnected(reason) => {
                            warn!(channel = %channel_id, ?reason, "channel disconnected");
                        }
                        _ => {}
                    }
                }
                // Handle streaming messages from the HTTP API — spawn concurrently
                Some(stream_msg) = stream_rx.recv() => {
                    let s = state.clone();
                    tokio::spawn(async move {
                        process_stream_message(s, stream_msg.text, stream_msg.session_id, stream_msg.chunk_tx).await;
                    });
                }
                // Handle incoming mesh messages (if mesh is enabled)
                Some(mesh_msg) = async {
                    match mesh_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let s = state.clone();
                    tokio::spawn(async move {
                        process_mesh_message(s, mesh_msg).await;
                    });
                }
                // Handle scheduled tasks from the cron/one-shot scheduler
                Some(sched_event) = scheduler_rx.recv() => {
                    let s = state.clone();
                    tokio::spawn(async move {
                        info!(
                            task_id = %sched_event.task_id,
                            label = ?sched_event.label,
                            "scheduler fired — processing scheduled task"
                        );

                        // Persist updated fire_count/last_fired to DB
                        if let Some(ref sched_handle) = s.scheduler {
                            if let Some(task) = sched_handle.get(sched_event.task_id).await {
                                let mem = s.memory.lock().await;
                                persist_task_to_db(&mem, &task);
                            }
                        }

                        // Create or reuse a session for this scheduled task
                        let session_id_str = if let Some(sid) = sched_event.session_id {
                            sid.to_string()
                        } else {
                            // Create a new session for scheduled tasks
                            let sid = s.sessions.create().await;
                            let label = sched_event.label.clone().unwrap_or_else(|| "scheduled task".to_string());
                            s.sessions.set_name(sid, &label).await;
                            sid.to_string()
                        };

                        // Wrap the description with a system note so the agent
                        // doesn't re-schedule the same recurring task.
                        let prompt = format!(
                            "[SYSTEM: This is a scheduled task firing automatically. \
                             Task ID: {}. Label: {}. \
                             Do NOT create or schedule any new cron/recurring tasks — \
                             this one is already recurring. Just execute the task below.]\n\n{}",
                            sched_event.task_id,
                            sched_event.label.as_deref().unwrap_or("none"),
                            sched_event.description,
                        );

                        // Process through API path
                        let resp = process_api_message(
                            s.clone(),
                            prompt,
                            Some(session_id_str),
                        ).await;

                        // Send the result to all active channels so users see the output
                        if !resp.text.is_empty() {
                            let label_str = sched_event.label.as_deref().unwrap_or("scheduled task");
                            let msg = format!(
                                "⏰ *{}*\n\n{}",
                                label_str,
                                resp.text,
                            );

                            // Broadcast to web UI via SSE notifications
                            if let Some(handle) = get_runtime_handle().await {
                                handle.notify(Notification::CronResult {
                                    task_id: sched_event.task_id.to_string(),
                                    label: label_str.to_string(),
                                    text: resp.text.clone(),
                                    session_id: resp.session_id.clone(),
                                });
                            }

                            // Find the most recently active session for each channel
                            // and send the notification to its target (chat/user).
                            let all_sessions = s.sessions.list_sessions().await;
                            let channels = s.channels.lock().await;
                            for channel in channels.iter() {
                                let channel_id = channel.id().to_string();
                                // Find the most recent active session for this channel
                                let target = all_sessions
                                    .iter()
                                    .filter(|sess| {
                                        sess.active && sess.channel.as_deref() == Some(&channel_id)
                                    })
                                    .max_by_key(|sess| sess.message_count)
                                    .and_then(|sess| sess.target.clone());

                                if let Some(target) = target {
                                    let _ = channel.send(OutgoingMessage {
                                        channel: channel_id,
                                        target,
                                        text: msg.clone(),
                                        attachments: vec![],
                                        reply_to: None,
                                    }).await;
                                } else {
                                    debug!(channel = %channel_id, "no active session found for channel — skipping notification");
                                }
                            }
                        }
                    });
                }
                // Both channels closed — shut down
                else => {
                    break;
                }
            }
        }

        info!("agent runtime shutting down — flushing sessions");

        // Graceful shutdown: persist all sessions and working memory
        {
            let sessions = state.sessions.snapshot().await;
            let mem = state.memory.lock().await;
            for session in &sessions {
                if session.message_count == 0 {
                    continue;
                }
                let _ = mem.persist_session(
                    &session.id,
                    session.name.as_deref(),
                    session.channel.as_deref(),
                    session.target.as_deref(),
                    session.active,
                    session.message_count,
                );
                if session.active {
                    let messages = mem.working.messages(session.id);
                    if !messages.is_empty() {
                        let _ = mem.persist_session_messages(&session.id, messages);
                    }
                }
            }
            info!("session data flushed to disk");
        }

        self.event_bus.publish(Event::Shutdown);
        Ok(())
    }
}

// ── Test helpers ──────────────────────────────────────────────

/// Build a `SharedAgentState` suitable for testing (with in-memory DB, no channels).
/// The returned `ModelRouter` is shared via Arc — call `build_test_state_with_provider`
/// if you need to register a mock provider.
pub fn build_test_state(config: ClawConfig) -> claw_core::Result<SharedAgentState> {
    build_test_state_with_router(config, ModelRouter::new())
}

/// Build a `SharedAgentState` with a pre-configured `ModelRouter` (for registering mock providers).
pub fn build_test_state_with_router(
    config: ClawConfig,
    llm: ModelRouter,
) -> claw_core::Result<SharedAgentState> {
    let memory = MemoryStore::open_in_memory()?;
    let mut guardrails = GuardrailEngine::new();
    guardrails.set_allowlist(config.autonomy.tool_allowlist.clone());
    guardrails.set_denylist(config.autonomy.tool_denylist.clone());

    let budget = BudgetTracker::new(
        config.autonomy.daily_budget_usd,
        config.autonomy.max_tool_calls_per_loop,
    );
    let planner = GoalPlanner::new();
    let approval = ApprovalGate::new();
    let plugins = PluginHost::new_empty();

    Ok(SharedAgentState {
        config: config.clone(),
        llm: Arc::new(llm),
        tools: BuiltinTools::new(),
        sessions: SessionManager::new(),
        budget,
        guardrails: Arc::new(guardrails),
        approval: Arc::new(approval),
        plugins: Arc::new(plugins),
        skills: Arc::new(TokioMutex::new(SkillRegistry::new_empty())),
        event_bus: EventBus::default(),
        memory: Arc::new(TokioMutex::new(memory)),
        planner: Arc::new(TokioMutex::new(planner)),
        channels: Arc::new(TokioMutex::new(Vec::new())),
        embedder: None,
        mesh: Arc::new(TokioMutex::new(MeshNode::new().unwrap())),
        pending_mesh_tasks: Arc::new(TokioMutex::new(HashMap::new())),
        pending_sub_tasks: Arc::new(TokioMutex::new(HashMap::new())),
        scheduler: None,
        device_tools: Arc::new(DeviceTools::new()),
        reply_context: Arc::new(TokioMutex::new(None)),
        stream_tx: Arc::new(TokioMutex::new(None)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use claw_core::ToolCall;
    use claw_llm::mock::MockProvider;
    use crate::tool_dispatch::execute_tool_shared;

    fn test_config() -> ClawConfig {
        let mut config = ClawConfig::default();
        config.agent.model = "mock/test-model".to_string();
        config.agent.max_iterations = 5;
        config.autonomy.level = 3; // autonomous — no approvals for low-risk
        config
    }

    fn test_state_with_mock(mock: MockProvider) -> SharedAgentState {
        let config = test_config();
        let mut router = ModelRouter::new();
        router.add_provider(Arc::new(mock));
        build_test_state_with_router(config, router).unwrap()
    }

    #[tokio::test]
    async fn test_simple_chat_response() {
        let mock = MockProvider::new("mock").with_response("Hello from the mock LLM!");
        let state = test_state_with_mock(mock);

        let resp = process_api_message(state, "Hi there".into(), None).await;
        assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
        assert!(
            resp.text.contains("Hello from the mock LLM"),
            "expected mock response, got: {}",
            resp.text
        );
        assert!(!resp.session_id.is_empty());
    }

    #[tokio::test]
    async fn test_chat_creates_session() {
        let mock = MockProvider::new("mock")
            .with_response("Session test")
            .with_response("Session test 2");
        let state = test_state_with_mock(mock);

        let resp = process_api_message(state.clone(), "Hello".into(), None).await;
        assert!(resp.error.is_none());
        assert!(!resp.session_id.is_empty(), "should have a session ID");

        // Second message with session hint creates a deterministic session
        let resp2 = process_api_message(
            state.clone(),
            "Hello again".into(),
            Some(resp.session_id.clone()),
        )
        .await;
        assert!(resp2.error.is_none());
        // Both should be valid UUID session IDs
        assert!(resp2.session_id.parse::<Uuid>().is_ok());

        // Verify sessions were created
        let sessions = state.sessions.list().await;
        assert!(!sessions.is_empty(), "expected at least one session");
    }

    #[tokio::test]
    async fn test_tool_execution_loop() {
        let mock = MockProvider::new("mock")
            .with_tool_call(
                "memory_store",
                serde_json::json!({"key": "test_fact", "value": "hello world"}),
            )
            .with_response("I stored the fact for you.");
        let state = test_state_with_mock(mock);

        let resp =
            process_api_message(state, "Remember that test_fact is hello world".into(), None).await;
        assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
        assert!(
            resp.text.contains("stored the fact"),
            "expected final response after tool execution, got: {}",
            resp.text
        );
    }

    #[tokio::test]
    async fn test_budget_tracking() {
        let mock = MockProvider::new("mock").with_response("Budget test");
        let state = test_state_with_mock(mock);

        let snap_before = state.budget.snapshot();
        assert_eq!(snap_before.daily_spend_usd, 0.0);

        let resp = process_api_message(state.clone(), "Test budget".into(), None).await;
        assert!(resp.error.is_none());

        let snap_after = state.budget.snapshot();
        assert!(
            snap_after.daily_spend_usd > 0.0,
            "expected budget spend to increase after LLM call"
        );
    }

    #[tokio::test]
    async fn test_max_iterations_limit() {
        let mut config = test_config();
        config.agent.max_iterations = 2;

        let mock = MockProvider::new("mock")
            .with_tool_call("memory_search", serde_json::json!({"query": "test"}))
            .with_tool_call("memory_search", serde_json::json!({"query": "test2"}))
            .with_tool_call("memory_search", serde_json::json!({"query": "test3"}));
        let mut router = ModelRouter::new();
        router.add_provider(Arc::new(mock));
        let state = build_test_state_with_router(config, router).unwrap();

        let resp = process_api_message(state, "Search forever".into(), None).await;
        // Should not error — the loop just stops after max iterations
        assert!(resp.error.is_none() || resp.text.is_empty());
    }

    #[tokio::test]
    async fn test_memory_store_and_search() {
        let mock = MockProvider::new("mock")
            .with_tool_call(
                "memory_store",
                serde_json::json!({"key": "capital", "value": "Paris is the capital of France"}),
            )
            .with_response("Stored!");
        let state = test_state_with_mock(mock);

        let resp = process_api_message(state.clone(), "Store fact".into(), None).await;
        assert!(resp.error.is_none());

        // Verify fact was stored
        let mem = state.memory.lock().await;
        let results = mem.semantic.search("capital");
        assert!(!results.is_empty(), "expected stored fact to be searchable");
    }

    #[tokio::test]
    async fn test_goal_create() {
        let mock = MockProvider::new("mock")
            .with_tool_call(
                "goal_create",
                serde_json::json!({
                    "description": "Write integration tests",
                    "priority": 1
                }),
            )
            .with_response("Goal created.");
        let state = test_state_with_mock(mock);

        let resp = process_api_message(state.clone(), "Create a goal".into(), None).await;
        assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

        let planner = state.planner.lock().await;
        let goals = planner.active_goals();
        assert!(!goals.is_empty(), "expected at least one active goal");
    }

    #[tokio::test]
    async fn test_llm_error_propagation() {
        let mock = MockProvider::new("mock").with_error("rate limited");
        let state = test_state_with_mock(mock);

        let resp = process_api_message(state, "Hi".into(), None).await;
        // The error propagates (either the LLM error or a failover ModelNotFound)
        assert!(resp.error.is_some(), "expected error to propagate");
    }

    #[tokio::test]
    async fn test_streaming_response() {
        let mock = MockProvider::new("mock").with_response("Streaming test response");
        let state = test_state_with_mock(mock);

        let (chunk_tx, mut chunk_rx) = mpsc::channel::<StreamEvent>(256);

        let state_clone = state.clone();
        tokio::spawn(async move {
            process_stream_message(state_clone, "Hello stream".into(), None, chunk_tx).await;
        });

        // Collect events
        let mut got_session = false;
        let mut got_text = false;
        let mut got_done = false;
        while let Some(event) = chunk_rx.recv().await {
            match event {
                StreamEvent::Session { .. } => got_session = true,
                StreamEvent::TextDelta { .. } => got_text = true,
                StreamEvent::Done => {
                    got_done = true;
                    break;
                }
                StreamEvent::Error { message } => panic!("unexpected error: {message}"),
                _ => {}
            }
        }
        assert!(got_session, "expected session event");
        assert!(got_text, "expected at least one text delta event");
        assert!(got_done, "expected done event");
    }

    #[tokio::test]
    async fn test_skill_system_prompt_injection() {
        let mock = MockProvider::new("mock").with_response("Mock LLM response");
        let state = test_state_with_mock(mock);

        // Register a SKILL.md-style skill
        let skill = claw_skills::SkillDefinition {
            name: "test-skill".into(),
            description: "A test skill for prompt injection".into(),
            version: "1.0.0".into(),
            tags: vec!["testing".into()],
            author: None,
            body: "# Test Skill\n\nUse shell_exec to echo hello.".into(),
            file_path: std::path::PathBuf::from("/skills/test-skill/SKILL.md"),
            base_dir: std::path::PathBuf::from("/skills/test-skill"),
        };
        state.skills.lock().await.register(skill);

        // Verify skill appears in system prompt block
        let block = state.skills.lock().await.system_prompt_block();
        assert!(block.is_some());
        let block = block.unwrap();
        assert!(block.contains("<name>test-skill</name>"));
        assert!(block.contains("<description>A test skill for prompt injection</description>"));
        assert!(block.contains("file_read"));
    }

    #[tokio::test]
    async fn test_skills_not_exposed_as_tools() {
        let mock = MockProvider::new("mock").with_response("Mock LLM response");
        let state = test_state_with_mock(mock);

        // Register a skill
        let skill = claw_skills::SkillDefinition {
            name: "my-skill".into(),
            description: "Should not be a tool".into(),
            version: "1.0.0".into(),
            tags: vec![],
            author: None,
            body: "Instructions.".into(),
            file_path: std::path::PathBuf::from("/skills/my-skill/SKILL.md"),
            base_dir: std::path::PathBuf::from("/skills/my-skill"),
        };
        state.skills.lock().await.register(skill);

        // Skills should NOT appear in the tool list
        let all_tools = state.tools.tools();
        assert!(!all_tools.iter().any(|t| t.name.contains("skill")));

        // Calling a "skill-*" tool name should return "Tool not found"
        let call = ToolCall {
            id: "test-call".into(),
            tool_name: "skill-my-skill".into(),
            arguments: serde_json::json!({}),
        };
        let result = execute_tool_shared(&state, &call).await;
        assert!(result.is_error);
        assert!(result.content.contains("Tool not found"));
    }

    #[tokio::test]
    async fn test_plugin_tool_dispatch_format() {
        let mock = MockProvider::new("mock").with_response("unused");
        let state = test_state_with_mock(mock);

        // Non-existent plugin should error
        let call = ToolCall {
            id: "test-call".into(),
            tool_name: "nonexistent_plugin.some_tool".into(),
            arguments: serde_json::json!({}),
        };
        let result = execute_tool_shared(&state, &call).await;
        assert!(result.is_error);
    }
}
