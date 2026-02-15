use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
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
    guardrail::GuardrailVerdict,
};
use claw_channels::adapter::{
    ApprovalPrompt, Channel, ChannelEvent, IncomingMessage, OutgoingMessage,
};
use claw_config::ClawConfig;
use claw_core::{Event, EventBus, Message, Role, Tool, ToolCall, ToolResult};
use claw_llm::{LlmProvider, LlmRequest, ModelRouter, StopReason};
use claw_memory::MemoryStore;
use claw_mesh::{MeshMessage, MeshNode};
use claw_plugin::PluginHost;
use claw_skills::SkillRegistry;

use crate::scheduler::SchedulerHandle;
use crate::session::SessionManager;
use crate::tools::BuiltinTools;
use claw_device::DeviceTools;

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
fn build_default_system_prompt() -> String {
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

/// Kinds of queries the server can ask the runtime.
pub enum QueryKind {
    Status,
    Sessions,
    SessionMessages(String),
    Goals,
    Tools,
    Facts,
    MemorySearch(String),
    Config,
    AuditLog(usize),
    MeshPeers,
    MeshStatus,
    SubTasks,
    ScheduledTasks,
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
        handle.await.map_err(|e| format!("task panicked: {}", e))
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
    pub async fn query(&self, kind: QueryKind) -> Result<serde_json::Value, String> {
        handle_query(&self.state, kind, self.started_at).await
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
                                    let uuid_arg: Option<String> = trimmed.splitn(2, ' ').nth(1)
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
                                                            Err(e) => format!("⚠️ {}", e),
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
                                                        Err(e) => format!("⚠️ {}", e),
                                                    }
                                                } else {
                                                    let ids: Vec<String> = map.keys().map(|id| id.to_string()).collect();
                                                    drop(map);
                                                    format!("⚠️ {} pending approvals. Specify which:\n{}", count,
                                                        ids.iter().map(|id| format!("  /approve {}", id)).collect::<Vec<_>>().join("\n"))
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
                                            Err(e) => format!("⚠️ {}", e),
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

// ─── Concurrent free functions operating on SharedAgentState ─────────────────
// Spawned as independent tokio tasks. Use fine-grained locks on memory/planner
// so LLM calls, tool execution, and approval waits never block other requests.

/// Handle a query by reading shared state — lightweight reads only.
async fn handle_query(
    state: &SharedAgentState,
    kind: QueryKind,
    started_at: std::time::Instant,
) -> Result<serde_json::Value, String> {
    let result = match kind {
        QueryKind::Status => {
            let budget = state.budget.snapshot();
            let session_count = state.sessions.active_count().await;
            let channels = state.channels.lock().await;
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "status": "running",
                "uptime_secs": started_at.elapsed().as_secs(),
                "model": &state.config.agent.model,
                "autonomy_level": state.config.autonomy.level,
                "budget": {
                    "spent_usd": budget.daily_spend_usd,
                    "daily_limit_usd": budget.daily_limit_usd,
                    "total_spend_usd": budget.total_spend_usd,
                    "total_tool_calls": budget.total_tool_calls,
                },
                "sessions": session_count,
                "channels": channels.iter().map(|c| c.id().to_string()).collect::<Vec<_>>(),
            })
        }
        QueryKind::Sessions => {
            let sessions = state.sessions.list_sessions().await;
            let list: Vec<serde_json::Value> = sessions
                .iter()
                .filter(|s| s.message_count > 0)
                .map(|s| {
                    serde_json::json!({
                        "id": s.id.to_string(),
                        "name": s.name,
                        "active": s.active,
                        "message_count": s.message_count,
                        "channel": s.channel,
                        "created_at": s.created_at.to_rfc3339(),
                    })
                })
                .collect();
            serde_json::json!({ "sessions": list })
        }
        QueryKind::SessionMessages(ref session_id_str) => {
            if let Ok(sid) = session_id_str.parse::<Uuid>() {
                let mem = state.memory.lock().await;
                let mut messages_slice = mem.working.messages(sid);

                // If working memory is empty, try loading from SQLite
                let persisted;
                if messages_slice.is_empty() {
                    if let Ok(msgs) = mem.load_session_messages(&sid) {
                        if !msgs.is_empty() {
                            persisted = msgs;
                            messages_slice = &persisted;
                        }
                    }
                }

                let list: Vec<serde_json::Value> = messages_slice
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "id": m.id.to_string(),
                            "role": m.role,
                            "content": m.text_content(),
                            "tool_calls": m.tool_calls.iter().map(|tc| serde_json::json!({
                                "id": tc.id,
                                "tool_name": tc.tool_name,
                                "arguments": tc.arguments,
                            })).collect::<Vec<_>>(),
                            "timestamp": m.timestamp.to_rfc3339(),
                        })
                    })
                    .collect();
                serde_json::json!({ "messages": list })
            } else {
                serde_json::json!({ "error": "invalid session_id" })
            }
        }
        QueryKind::Goals => {
            let planner = state.planner.lock().await;
            let goals: Vec<serde_json::Value> = planner
                .all()
                .iter()
                .map(|g| {
                    serde_json::json!({
                        "id": g.id.to_string(),
                        "title": g.description,
                        "description": g.description,
                        "status": format!("{:?}", g.status),
                        "priority": g.priority,
                        "progress": g.progress,
                        "created_at": g.created_at.to_rfc3339(),
                        "updated_at": g.updated_at.to_rfc3339(),
                        "steps": g.steps.iter().map(|s| serde_json::json!({
                            "id": s.id.to_string(),
                            "description": s.description,
                            "status": format!("{:?}", s.status),
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            serde_json::json!({ "goals": goals })
        }
        QueryKind::Tools => {
            let tools: Vec<serde_json::Value> = state
                .tools
                .tools()
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                        "risk_level": t.risk_level,
                        "is_mutating": t.is_mutating,
                        "capabilities": t.capabilities,
                        "provider": t.provider,
                    })
                })
                .collect();
            let mut plugin_tools: Vec<serde_json::Value> = state
                .plugins
                .tools()
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                        "risk_level": t.risk_level,
                        "is_mutating": t.is_mutating,
                        "capabilities": t.capabilities,
                        "provider": t.provider,
                    })
                })
                .collect();
            let mut all = tools;
            all.append(&mut plugin_tools);
            // Skills are prompt-injected (SKILL.md), not listed as tools.
            // They appear in the system prompt via <available_skills>.
            // Add device tools (browser, android, ios)
            let device_tools: Vec<serde_json::Value> = DeviceTools::tools()
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                        "risk_level": t.risk_level,
                        "is_mutating": t.is_mutating,
                        "capabilities": t.capabilities,
                        "provider": "device",
                    })
                })
                .collect();
            all.extend(device_tools);
            serde_json::json!({ "tools": all })
        }
        QueryKind::Facts => {
            let mem = state.memory.lock().await;
            let facts: Vec<serde_json::Value> = mem
                .semantic
                .all_facts()
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "id": f.id.to_string(),
                        "category": f.category,
                        "key": f.key,
                        "value": f.value,
                        "confidence": f.confidence,
                        "source": f.source,
                        "created_at": f.created_at.to_rfc3339(),
                        "updated_at": f.updated_at.to_rfc3339(),
                    })
                })
                .collect();
            let count = facts.len();
            serde_json::json!({ "facts": facts, "count": count })
        }
        QueryKind::MemorySearch(ref query_text) => {
            // Embed query for vector search if embedder is available
            let query_embedding = if let Some(ref embedder) = state.embedder {
                match embedder.embed(&[query_text.as_str()]).await {
                    Ok(vecs) if !vecs.is_empty() => Some(vecs.into_iter().next().unwrap()),
                    _ => None,
                }
            } else {
                None
            };

            let mem = state.memory.lock().await;
            let episodes = mem.episodic.search(query_text);

            // Use vector search for facts when embedding is available
            let fact_results: Vec<serde_json::Value> = if let Some(ref qemb) = query_embedding {
                let vector_hits = mem.semantic.vector_search(qemb, 20);
                if !vector_hits.is_empty() {
                    vector_hits
                        .iter()
                        .map(|(f, score)| {
                            serde_json::json!({
                                "type": "fact",
                                "category": f.category,
                                "key": f.key,
                                "value": f.value,
                                "confidence": f.confidence,
                                "relevance": score,
                            })
                        })
                        .collect()
                } else {
                    mem.semantic
                        .search(query_text)
                        .iter()
                        .take(20)
                        .map(|f| {
                            serde_json::json!({
                                "type": "fact",
                                "category": f.category,
                                "key": f.key,
                                "value": f.value,
                                "confidence": f.confidence,
                            })
                        })
                        .collect()
                }
            } else {
                mem.semantic
                    .search(query_text)
                    .iter()
                    .take(20)
                    .map(|f| {
                        serde_json::json!({
                            "type": "fact",
                            "category": f.category,
                            "key": f.key,
                            "value": f.value,
                            "confidence": f.confidence,
                        })
                    })
                    .collect()
            };

            let ep_results: Vec<serde_json::Value> = episodes
                .iter()
                .take(10)
                .map(|e| {
                    serde_json::json!({
                        "type": "episode",
                        "summary": e.summary,
                        "outcome": e.outcome,
                        "tags": e.tags,
                        "created_at": e.created_at.to_rfc3339(),
                    })
                })
                .collect();
            let mut results = ep_results;
            results.extend(fact_results);
            serde_json::json!({ "results": results, "query": query_text })
        }
        QueryKind::Config => {
            let channels = state.channels.lock().await;
            serde_json::json!({
                "agent": {
                    "model": &state.config.agent.model,
                    "fallback_model": &state.config.agent.fallback_model,
                    "fast_model": &state.config.agent.fast_model,
                    "max_tokens": state.config.agent.max_tokens,
                    "temperature": state.config.agent.temperature,
                    "max_iterations": state.config.agent.max_iterations,
                    "thinking_level": &state.config.agent.thinking_level,
                },
                "autonomy": {
                    "level": state.config.autonomy.level,
                    "daily_budget_usd": state.config.autonomy.daily_budget_usd,
                    "max_tool_calls_per_loop": state.config.autonomy.max_tool_calls_per_loop,
                    "approval_threshold": state.config.autonomy.approval_threshold,
                    "proactive": state.config.autonomy.proactive,
                    "tool_allowlist": &state.config.autonomy.tool_allowlist,
                    "tool_denylist": &state.config.autonomy.tool_denylist,
                },
                "memory": {
                    "db_path": state.config.memory.db_path.display().to_string(),
                    "max_episodes": state.config.memory.max_episodes,
                    "vector_search": state.config.memory.vector_search,
                    "embedding_dims": state.config.memory.embedding_dims,
                },
                "server": {
                    "listen": &state.config.server.listen,
                    "web_ui": state.config.server.web_ui,
                    "cors": state.config.server.cors,
                },
                "channels": channels.iter().map(|c| serde_json::json!({
                    "id": c.id(),
                    "type": c.channel_type(),
                })).collect::<Vec<_>>(),
                "plugins": {
                    "plugin_dir": state.config.plugins.plugin_dir.display().to_string(),
                    "registry_url": &state.config.plugins.registry_url,
                },
            })
        }
        QueryKind::AuditLog(limit) => {
            let mem = state.memory.lock().await;
            let entries: Vec<serde_json::Value> = mem
                .audit_log(limit)
                .into_iter()
                .map(|(timestamp, event_type, action, details)| {
                    serde_json::json!({
                        "timestamp": timestamp,
                        "event_type": event_type,
                        "action": action,
                        "details": details,
                    })
                })
                .collect();
            let count = entries.len();
            serde_json::json!({ "audit_log": entries, "count": count })
        }
        QueryKind::MeshPeers => {
            let mesh = state.mesh.lock().await;
            let peers: Vec<serde_json::Value> = mesh
                .peer_list()
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "peer_id": p.peer_id,
                        "hostname": p.hostname,
                        "capabilities": p.capabilities,
                        "os": p.os,
                    })
                })
                .collect();
            let count = peers.len();
            serde_json::json!({ "peers": peers, "count": count })
        }
        QueryKind::MeshStatus => {
            let mesh = state.mesh.lock().await;
            serde_json::json!({
                "enabled": state.config.mesh.enabled,
                "running": mesh.is_running(),
                "peer_id": mesh.peer_id(),
                "peer_count": mesh.peer_count(),
                "listen": &state.config.mesh.listen,
                "mdns": state.config.mesh.mdns,
                "capabilities": &state.config.mesh.capabilities,
                "p2p": true,
            })
        }
        QueryKind::SubTasks => {
            let tasks = state.pending_sub_tasks.lock().await;
            let list: Vec<serde_json::Value> = tasks
                .values()
                .map(|t| {
                    serde_json::json!({
                        "task_id": t.task_id.to_string(),
                        "role": t.role,
                        "task_description": t.task_description,
                        "status": t.status,
                        "result": t.result,
                        "error": t.error,
                        "depends_on": t.depends_on.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
                        "elapsed_secs": t.created_at.elapsed().as_secs(),
                    })
                })
                .collect();
            let count = list.len();
            let running = list.iter().filter(|t| t["status"] == "running").count();
            let completed = list.iter().filter(|t| t["status"] == "completed").count();
            let failed = list.iter().filter(|t| t["status"] == "failed").count();
            serde_json::json!({
                "sub_tasks": list,
                "count": count,
                "running": running,
                "completed": completed,
                "failed": failed,
            })
        }
        QueryKind::ScheduledTasks => {
            if let Some(ref scheduler) = state.scheduler {
                let tasks = scheduler.list_all().await;
                let list: Vec<serde_json::Value> = tasks
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "id": t.id.to_string(),
                            "label": t.label,
                            "description": t.description,
                            "kind": t.kind,
                            "active": t.active,
                            "fire_count": t.fire_count,
                            "last_fired": t.last_fired.map(|d| d.to_rfc3339()),
                            "created_at": t.created_at.to_rfc3339(),
                        })
                    })
                    .collect();
                let active = list.iter().filter(|t| t["active"] == true).count();
                serde_json::json!({
                    "scheduled_tasks": list,
                    "count": list.len(),
                    "active": active,
                })
            } else {
                serde_json::json!({
                    "scheduled_tasks": [],
                    "count": 0,
                    "active": 0,
                    "scheduler_enabled": false,
                })
            }
        }
    };
    Ok(result)
}

/// Process an incoming mesh message — update local state and handle tasks.
async fn process_mesh_message(state: SharedAgentState, message: MeshMessage) {
    let our_peer_id = {
        let mesh = state.mesh.lock().await;
        mesh.peer_id().to_string()
    };

    // Only process messages addressed to us
    if !message.is_for_peer(&our_peer_id) {
        return;
    }

    // Let the mesh node update its peer table
    let handled = {
        let mut mesh = state.mesh.lock().await;
        mesh.handle_message(&message)
    };

    if handled {
        return; // Peer bookkeeping only — no further action needed
    }

    // Handle messages that require runtime processing
    match message {
        MeshMessage::TaskAssign(task) => {
            info!(
                task_id = %task.task_id,
                from = %task.from_peer,
                desc = %task.description,
                "received task assignment from mesh peer"
            );

            // Execute the task by processing it as a chat message
            let result_text =
                match process_api_message(state.clone(), task.description.clone(), None).await {
                    resp if resp.error.is_none() => resp.text,
                    resp => format!("Error: {}", resp.error.unwrap_or_default()),
                };

            // Send the result back to the originator
            let result_msg = MeshMessage::TaskResult {
                task_id: task.task_id,
                peer_id: our_peer_id.clone(),
                success: true,
                result: result_text,
            };
            let mesh = state.mesh.lock().await;
            if let Err(e) = mesh.send_to(&task.from_peer, &result_msg).await {
                warn!(error = %e, from = %task.from_peer, "failed to send task result");
            }
        }
        MeshMessage::TaskResult {
            task_id,
            peer_id,
            success,
            result,
        } => {
            info!(
                task_id = %task_id,
                from = %peer_id,
                success = success,
                "received task result from mesh peer"
            );

            // Resolve the pending mesh task if someone is waiting for it
            let resolved = {
                let mut pending = state.pending_mesh_tasks.lock().await;
                if let Some(tx) = pending.remove(&task_id) {
                    let _ = tx.send(MeshTaskResult {
                        task_id,
                        peer_id: peer_id.clone(),
                        success,
                        result: result.clone(),
                    });
                    true
                } else {
                    false
                }
            };

            if !resolved {
                // No one waiting — check if it's for a delegated goal step
                let mut planner = state.planner.lock().await;
                if success {
                    planner.complete_delegated_task(task_id, result.clone());
                } else {
                    planner.fail_delegated_task(task_id, result.clone());
                }
            }
        }
        MeshMessage::DirectMessage {
            from_peer, content, ..
        } => {
            info!(
                from = %from_peer,
                content = %content,
                "received direct message from mesh peer"
            );
        }
        MeshMessage::SyncDelta {
            peer_id,
            delta_type,
            data,
        } => {
            debug!(
                from = %peer_id,
                delta_type = %delta_type,
                "received sync delta from mesh peer"
            );

            match delta_type.as_str() {
                "fact" => {
                    // Apply incoming fact to our local memory
                    if let (Some(category), Some(key), Some(value)) = (
                        data.get("category").and_then(|v| v.as_str()),
                        data.get("key").and_then(|v| v.as_str()),
                        data.get("value").and_then(|v| v.as_str()),
                    ) {
                        let confidence = data
                            .get("confidence")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.8);
                        let source = format!("mesh:{}", peer_id);
                        let mut mem = state.memory.lock().await;
                        // Upsert into in-memory semantic store
                        mem.semantic.upsert(claw_memory::semantic::Fact {
                            id: uuid::Uuid::new_v4(),
                            category: category.to_string(),
                            key: key.to_string(),
                            value: value.to_string(),
                            confidence,
                            source: Some(source.clone()),
                            embedding: None,
                            created_at: chrono::Utc::now(),
                            updated_at: chrono::Utc::now(),
                        });
                        // Persist to SQLite
                        let _ = mem.persist_fact(category, key, value);
                        info!(
                            category = category,
                            key = key,
                            from = %peer_id,
                            "synced fact from mesh peer"
                        );
                    } else {
                        warn!(from = %peer_id, "received malformed fact sync delta");
                    }
                }
                "episode" => {
                    // Apply incoming episode summary
                    if let Some(summary) = data.get("summary").and_then(|v| v.as_str()) {
                        let outcome = data
                            .get("outcome")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        let tags: Vec<String> = data
                            .get("tags")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let mut mem = state.memory.lock().await;
                        let episode = claw_memory::episodic::Episode {
                            id: uuid::Uuid::new_v4(),
                            session_id: uuid::Uuid::new_v4(),
                            summary: summary.to_string(),
                            outcome,
                            tags,
                            created_at: chrono::Utc::now(),
                            updated_at: chrono::Utc::now(),
                        };
                        mem.episodic.record(episode);
                        info!(
                            summary = summary,
                            from = %peer_id,
                            "synced episode from mesh peer"
                        );
                    }
                }
                other => {
                    debug!(delta_type = other, "unknown sync delta type — ignoring");
                }
            }
        }
        _ => {}
    }
}

/// Process a non-streaming API chat message — spawned as a concurrent task.
async fn process_api_message(
    state: SharedAgentState,
    text: String,
    session_id_hint: Option<String>,
) -> ApiResponse {
    let session_id = if let Some(ref hint) = session_id_hint {
        if let Ok(id) = hint.parse::<Uuid>() {
            state.sessions.get_or_insert(id, "api", "api_user").await
        } else {
            state.sessions.find_or_create("api", hint).await
        }
    } else {
        state.sessions.create_for_channel("api", "api_user").await
    };

    let incoming = IncomingMessage {
        id: Uuid::new_v4().to_string(),
        channel: "api".to_string(),
        sender: "api_user".to_string(),
        sender_name: Some("API User".to_string()),
        group: None,
        text: Some(text),
        attachments: vec![],
        is_mention: false,
        is_reply_to_bot: false,
        metadata: serde_json::Value::Null,
    };

    match process_message_shared(&state, "api", incoming, Some(session_id)).await {
        Ok(response_text) => ApiResponse {
            text: response_text,
            session_id: session_id.to_string(),
            error: None,
        },
        Err(e) => ApiResponse {
            text: String::new(),
            session_id: session_id.to_string(),
            error: Some(e.to_string()),
        },
    }
}

/// Process a channel message — spawned as a concurrent task.
/// Uses the streaming path so we can send real-time progress updates
/// (typing indicators, tool-call notifications) to the channel while
/// the agent works through multi-step tasks.
async fn process_channel_message(
    state: SharedAgentState,
    channel_id: &str,
    incoming: IncomingMessage,
) -> claw_core::Result<()> {
    let target = incoming
        .group
        .as_deref()
        .unwrap_or(&incoming.sender)
        .to_string();
    let channel_id_owned = channel_id.to_string();

    // Spawn periodic typing indicator so the user sees activity
    // (Telegram typing indicators expire after ~5 s)
    let state_typing = state.clone();
    let cid_typing = channel_id_owned.clone();
    let target_typing = target.clone();
    let typing_handle = tokio::spawn(async move {
        loop {
            send_typing_to_channel(&state_typing, &cid_typing, &target_typing).await;
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        }
    });

    // Create streaming channel
    let (tx, mut rx) = mpsc::channel::<StreamEvent>(128);

    // Spawn the streaming processor
    let state_stream = state.clone();
    let cid_stream = channel_id_owned.clone();
    let stream_handle = tokio::spawn(async move {
        let result =
            process_message_streaming_shared(&state_stream, &cid_stream, incoming, &tx, None).await;
        match &result {
            Ok(()) => {
                let _ = tx.send(StreamEvent::Done).await;
            }
            Err(e) => {
                let _ = tx
                    .send(StreamEvent::Error {
                        message: e.to_string(),
                    })
                    .await;
            }
        }
        result
    });

    // Consume stream events and forward progress as a single live-edited message
    let mut final_text = String::new();
    let mut progress_lines: Vec<String> = Vec::new();
    let mut progress_msg_id: Option<String> = None;
    let mut current_tool_ids: HashMap<String, usize> = HashMap::new(); // tool_call_id → index in progress_lines
    let mut last_edit_time = std::time::Instant::now() - std::time::Duration::from_secs(60);
    let edit_throttle = std::time::Duration::from_millis(1500);
    let mut pending_edit = false;

    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ToolCall { name, id, args } => {
                let emoji = tool_progress_emoji(&name);
                let desc = describe_tool_call(&name, &args);
                let line = format!("{}  {}", emoji, desc);
                let idx = progress_lines.len();
                progress_lines.push(line);
                current_tool_ids.insert(id, idx);
                pending_edit = true;
            }
            StreamEvent::ToolResult {
                id,
                is_error,
                ref content,
                ..
            } => {
                if let Some(&idx) = current_tool_ids.get(&id) {
                    if let Some(line) = progress_lines.get_mut(idx) {
                        // Replace the leading emoji with a status indicator.
                        // The line format is "{emoji}  {description}" — find the
                        // double-space separator and keep everything after it.
                        if let Some(sep) = line.find("  ") {
                            let description = &line[sep + 2..]; // 2 bytes for "  "
                            // Extract a brief result summary (first meaningful line)
                            let summary = extract_result_summary(content, 60);
                            if is_error {
                                if summary.is_empty() {
                                    *line = format!("❌  {}", description);
                                } else {
                                    *line = format!("❌  {} — {}", description, summary);
                                }
                            } else if summary.is_empty() {
                                *line = format!("✅  {}", description);
                            } else {
                                *line = format!("✅  {} → {}", description, summary);
                            }
                        }
                    }
                    current_tool_ids.remove(&id);
                    pending_edit = true;
                }
            }
            StreamEvent::TextDelta { content } => {
                final_text.push_str(&content);
            }
            StreamEvent::ApprovalRequired {
                id,
                tool_name,
                tool_args,
                reason,
                risk_level,
            } => {
                send_approval_prompt_shared(
                    &state,
                    &channel_id_owned,
                    &target,
                    &id,
                    &tool_name,
                    &tool_args,
                    &reason,
                    risk_level,
                )
                .await;
            }
            StreamEvent::Done => break,
            StreamEvent::Error { message } => {
                let _ = send_response_shared(
                    &state,
                    &channel_id_owned,
                    &target,
                    &format!("❌ Error: {}", message),
                )
                .await;
                break;
            }
            _ => {}
        }

        // Throttled edit/send of the progress message
        if pending_edit && !progress_lines.is_empty() {
            let now = std::time::Instant::now();
            if now.duration_since(last_edit_time) >= edit_throttle {
                let text = format!("🤖 *Working on it…*\n\n{}", progress_lines.join("\n"));
                match &progress_msg_id {
                    Some(msg_id) => {
                        let _ =
                            edit_channel_message(&state, &channel_id_owned, &target, msg_id, &text)
                                .await;
                    }
                    None => {
                        progress_msg_id = send_channel_message_returning_id(
                            &state,
                            &channel_id_owned,
                            &target,
                            &text,
                        )
                        .await;
                    }
                }
                last_edit_time = now;
                pending_edit = false;
            }
        }
    }

    // Final update of progress message — show all steps as completed
    if !progress_lines.is_empty() {
        let text = format!("🤖 *Done*\n\n{}", progress_lines.join("\n"));
        match &progress_msg_id {
            Some(msg_id) => {
                let _ =
                    edit_channel_message(&state, &channel_id_owned, &target, msg_id, &text).await;
            }
            None => {
                let _ = send_response_shared(&state, &channel_id_owned, &target, &text).await;
            }
        }
    }

    // Stop typing indicator
    typing_handle.abort();

    // Send final response
    if !final_text.is_empty() {
        send_response_shared(&state, &channel_id_owned, &target, &final_text).await?;
    }

    // Ensure streaming task completes cleanly
    match stream_handle.await {
        Ok(Err(e)) => warn!(error = %e, "channel streaming task error"),
        Err(e) if !e.is_cancelled() => warn!(error = %e, "channel streaming task panicked"),
        _ => {}
    }

    Ok(())
}

/// Process a streaming API message — spawned as a concurrent task.
async fn process_stream_message(
    state: SharedAgentState,
    text: String,
    session_id_hint: Option<String>,
    tx: mpsc::Sender<StreamEvent>,
) {
    let session_id = if let Some(ref hint) = session_id_hint {
        if let Ok(id) = hint.parse::<Uuid>() {
            state.sessions.get_or_insert(id, "api", "api_user").await
        } else {
            state.sessions.find_or_create("api", hint).await
        }
    } else {
        state.sessions.create_for_channel("api", "api_user").await
    };

    let _ = tx
        .send(StreamEvent::Session {
            session_id: session_id.to_string(),
        })
        .await;

    let incoming = IncomingMessage {
        id: Uuid::new_v4().to_string(),
        channel: "api".to_string(),
        sender: "api_user".to_string(),
        sender_name: Some("API User".to_string()),
        group: None,
        text: Some(text.clone()),
        attachments: vec![],
        is_mention: false,
        is_reply_to_bot: false,
        metadata: serde_json::Value::Null,
    };

    match process_message_streaming_shared(&state, "api", incoming, &tx, Some(session_id)).await {
        Ok(()) => {
            let _ = tx.send(StreamEvent::Done).await;
        }
        Err(e) => {
            let _ = tx
                .send(StreamEvent::Error {
                    message: e.to_string(),
                })
                .await;
        }
    }
}

/// Detect when the model is being lazy — responding with text that suggests the user
/// should finish the work themselves, instead of actually using tools to complete it.
/// Returns `true` if the response looks like a lazy cop-out.
/// Detect whether the model is stopping prematurely ("being lazy") instead of
/// actually completing the task.
///
/// ## Approach (inspired by Codex / Claude Code patterns)
///
/// Production agents use three main strategies:
/// - **Codex**: Pure structural — only loop if tool calls were emitted. No text analysis.
/// - **Claude Code hooks**: Transcript-level checks (e.g., "did tests run?").
/// - **Ralph Wiggum**: Require explicit `<promise>TASK COMPLETE</promise>` tag.
///
/// We use a **conservative hybrid**: only flag as lazy when the text contains strong
/// deferral language ("you can…", "feel free to…") AND the model has done very little
/// work in this session (low iteration count). If the model has already executed many
/// tool calls and iterations, it's more likely genuinely finished.
///
/// `iteration` is the current loop iteration (0-based).
fn is_lazy_stop(text: &str, iteration: usize) -> bool {
    // Very short responses are never lazy — they're confirmations
    if text.len() < 100 {
        return false;
    }

    let lower = text.to_lowercase();

    // ── Strong completion indicators — if present, trust the model ──
    let completion_signals = [
        "all files created",
        "project is complete",
        "everything is set up and working",
        "all done",
        "finished creating all",
        "built the complete",
        "full implementation",
        "all components created",
        "fully functional",
        "here's what i built",
        "here is what i built",
        "i've created all",
        "i have created all",
        "task complete",
    ];
    if completion_signals.iter().any(|p| lower.contains(*p)) {
        return false;
    }

    // ── Deferral phrases — the model is pushing work to the user ──
    let deferral_phrases = [
        "you can customize",
        "you can further",
        "you can modify",
        "you can adjust",
        "you can extend",
        "you can add more",
        "feel free to",
        "i'll leave",
        "left as an exercise",
        "up to you to",
        "you'll need to",
        "you should create",
        "you would need to",
        "the remaining",
        "repeat this for",
        "do the same for",
        "continue this pattern",
        "follow the same pattern",
        "and so on for",
    ];

    let deferral_count: usize = deferral_phrases
        .iter()
        .filter(|p| lower.contains(**p))
        .count();

    // ── Scaffolding-only — model ran a create-* command and stopped ──
    // Only check in early iterations (< 5) when the model hasn't done much yet
    if iteration < 5 {
        let scaffolding_stops = [
            "has been set up",
            "is now set up",
            "successfully set up",
            "ready for development",
            "you can start developing",
            "you can start building",
            "you can now start",
        ];
        let is_scaffolding = scaffolding_stops.iter().any(|p| lower.contains(*p));
        if is_scaffolding && deferral_count >= 1 {
            return true;
        }
    }

    // ── General laziness — require strong signal ──
    // After many iterations (8+), the model has done real work; need 3+ deferrals
    // In early iterations (< 8), 2+ deferrals is suspicious
    let threshold = if iteration >= 8 { 3 } else { 2 };
    deferral_count >= threshold
}

/// Truncate a tool result to fit within the token budget.
/// Preserves the beginning and end of the content, replacing the middle with a note.
fn truncate_tool_result(content: &str, max_tokens: usize) -> String {
    if max_tokens == 0 {
        return content.to_string(); // 0 = no limit
    }
    let max_chars = max_tokens * 4; // ~4 chars per token
    if content.len() <= max_chars {
        return content.to_string();
    }

    // Keep first 60% and last 20% of allowed chars, replace middle with truncation note
    let head_chars = (max_chars * 6) / 10;
    let tail_chars = (max_chars * 2) / 10;
    let head: String = content.chars().take(head_chars).collect();
    let tail: String = content
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let omitted_chars = content.len() - head_chars - tail_chars;
    let omitted_tokens = omitted_chars / 4;

    format!(
        "{}\n\n[... truncated {} tokens ({} chars) to fit context window ...]\n\n{}",
        head, omitted_tokens, omitted_chars, tail
    )
}

/// Perform LLM-powered compaction if the context is getting large.
/// Uses the fast_model if available, otherwise the primary model.
async fn maybe_compact_context(
    state: &SharedAgentState,
    session_id: Uuid,
) -> claw_core::Result<bool> {
    let needs_compaction = {
        let mem = state.memory.lock().await;
        mem.working.needs_compaction(session_id)
    };

    if !needs_compaction {
        return Ok(false);
    }

    let compaction_data = {
        let mem = state.memory.lock().await;
        mem.working.prepare_compaction_request(session_id)
    };

    let (text_to_summarize, messages_to_remove) = match compaction_data {
        Some(data) => data,
        None => return Ok(false),
    };

    info!(session = %session_id, messages = messages_to_remove, "performing LLM-powered context compaction");

    // Use fast model for compaction if available, otherwise primary
    let compaction_model = state
        .config
        .agent
        .fast_model
        .as_deref()
        .unwrap_or(&state.config.agent.model);

    let compaction_prompt = format!(
        "Summarize this conversation history concisely. Preserve:\n\
         - The user's original request and goals\n\
         - Key decisions and outcomes\n\
         - File paths, commands, and technical details that were discussed\n\
         - Any errors encountered and how they were resolved\n\
         - Current state of progress (what's done, what remains)\n\n\
         Keep the summary under 500 words. Be factual and specific.\n\n\
         Conversation to summarize:\n{}",
        text_to_summarize
    );

    let request = LlmRequest {
        model: compaction_model.to_string(),
        messages: vec![Message::text(Uuid::nil(), Role::User, &compaction_prompt)],
        tools: vec![],
        system: Some(
            "You are a precise conversation summarizer. Output only the summary, nothing else."
                .to_string(),
        ),
        max_tokens: 2048,
        temperature: 0.3,
        thinking_level: Some("off".to_string()),
        stream: false,
    };

    match state.llm.complete(&request, None).await {
        Ok(response) => {
            let summary = response.message.text_content();
            let mut mem = state.memory.lock().await;
            mem.working
                .apply_llm_compaction(session_id, &summary, messages_to_remove);
            let new_token_count = mem.working.token_count(session_id);
            info!(
                session = %session_id,
                compacted_messages = messages_to_remove,
                new_tokens = new_token_count,
                "LLM compaction complete"
            );
            Ok(true)
        }
        Err(e) => {
            // Fallback to naive compaction if LLM fails
            warn!(session = %session_id, error = %e, "LLM compaction failed, using naive compaction");
            let mut mem = state.memory.lock().await;
            mem.working.compact(session_id);
            Ok(true)
        }
    }
}

/// Core non-streaming message processing using shared state with fine-grained locks.
async fn process_message_shared(
    state: &SharedAgentState,
    channel_id: &str,
    incoming: IncomingMessage,
    override_session_id: Option<Uuid>,
) -> claw_core::Result<String> {
    let target = incoming.group.as_deref().unwrap_or(&incoming.sender);
    let target_owned = target.to_string();
    let channel_id_owned = channel_id.to_string();
    let session_id = match override_session_id {
        Some(id) => id,
        None => state.sessions.find_or_create(channel_id, target).await,
    };

    info!(
        session = %session_id,
        channel = channel_id,
        sender = %incoming.sender,
        "processing message"
    );

    state.event_bus.publish(Event::MessageReceived {
        session_id,
        message_id: Uuid::new_v4(),
        channel: channel_id.to_string(),
    });

    let user_text = incoming.text.unwrap_or_default();

    // 1. RECEIVE + RECALL — embed query (before lock) then search memory
    // Generate query embedding outside the memory lock (async I/O)
    let query_embedding = if let Some(ref embedder) = state.embedder {
        match embedder.embed(&[&user_text]).await {
            Ok(vecs) if !vecs.is_empty() => Some(vecs.into_iter().next().unwrap()),
            _ => None,
        }
    } else {
        None
    };

    let (context_parts, active_goals) = {
        let mut mem = state.memory.lock().await;
        let user_msg = Message::text(session_id, Role::User, &user_text);
        mem.working.push(user_msg);
        drop(mem);
        state.sessions.record_message(session_id).await;
        let mem = state.memory.lock().await;

        let relevant_episodes = mem.episodic.search(&user_text);

        // Build a combined keyword query from user text for broader matching
        // Also extract key nouns/terms that the word-level search can match
        let search_terms = extract_search_keywords(&user_text);

        // Collect facts from multiple search strategies, dedup by category+key
        let mut seen_fact_keys = std::collections::HashSet::new();
        let mut relevant_facts: Vec<String> = Vec::new();

        // Strategy 1: Vector search (best quality when embeddings available)
        if let Some(ref qemb) = query_embedding {
            for (fact, _score) in mem.semantic.vector_search(qemb, 10) {
                let fk = format!("{}:{}", fact.category, fact.key);
                if seen_fact_keys.insert(fk) {
                    relevant_facts.push(format!(
                        "- [{}] {}: {}",
                        fact.category, fact.key, fact.value
                    ));
                }
            }
        }

        // Strategy 2: Word-level keyword search on user text
        for fact in mem.semantic.search(&user_text).iter().take(10) {
            let fk = format!("{}:{}", fact.category, fact.key);
            if seen_fact_keys.insert(fk) {
                relevant_facts.push(format!(
                    "- [{}] {}: {}",
                    fact.category, fact.key, fact.value
                ));
            }
        }

        // Strategy 3: Search with extracted keywords (catches domain-specific terms)
        if search_terms != user_text.to_lowercase() {
            for fact in mem.semantic.search(&search_terms).iter().take(5) {
                let fk = format!("{}:{}", fact.category, fact.key);
                if seen_fact_keys.insert(fk) {
                    relevant_facts.push(format!(
                        "- [{}] {}: {}",
                        fact.category, fact.key, fact.value
                    ));
                }
            }
        }

        // Cap total facts to avoid bloating the system prompt
        relevant_facts.truncate(15);

        let mut parts = Vec::new();
        if !relevant_episodes.is_empty() {
            let episodes_text: Vec<String> = relevant_episodes
                .iter()
                .take(5)
                .map(|e| format!("- {}", e.summary))
                .collect();
            parts.push(format!(
                "Relevant past conversations:\n{}",
                episodes_text.join("\n")
            ));
        }
        if !relevant_facts.is_empty() {
            parts.push(format!(
                "Relevant knowledge:\n{}",
                relevant_facts.join("\n")
            ));
        }

        // Always load learned lessons — these are high-value self-corrections
        let lessons: Vec<String> = mem
            .semantic
            .category("learned_lessons")
            .iter()
            .map(|f| format!("- **{}**: {}", f.key, f.value))
            .collect();
        if !lessons.is_empty() {
            parts.push(format!(
                "Lessons learned from past sessions (apply these!):\n{}",
                lessons.join("\n")
            ));
        }

        drop(mem); // release memory lock

        let planner = state.planner.lock().await;
        let goals: Vec<_> = planner.active_goals().into_iter().cloned().collect();
        drop(planner); // release planner lock

        (parts, goals)
    };

    // 2. BUILD system prompt — no locks needed
    let mut system_prompt = state
        .config
        .agent
        .system_prompt
        .clone()
        .unwrap_or_else(build_default_system_prompt);
    if !context_parts.is_empty() {
        system_prompt.push_str("\n\n<memory>\n");
        system_prompt.push_str(&context_parts.join("\n\n"));
        system_prompt.push_str("\n</memory>");
    }
    if !active_goals.is_empty() {
        system_prompt.push_str("\n\n<active_goals>\n");
        for goal in &active_goals {
            system_prompt.push_str(&format!(
                "- [{}] {} (progress: {:.0}%)\n",
                goal.id,
                goal.description,
                goal.progress * 100.0
            ));
        }
        system_prompt.push_str("</active_goals>");
    }

    // Add mesh peer context so the LLM knows about the network
    {
        let mesh = state.mesh.lock().await;
        if mesh.is_running() {
            let peers = mesh.peer_list();
            if !peers.is_empty() {
                system_prompt.push_str("\n\n<mesh_network>\n");
                system_prompt.push_str(&format!(
                    "Your peer ID: {}\n",
                    &mesh.peer_id()[..12.min(mesh.peer_id().len())]
                ));
                system_prompt.push_str(&format!(
                    "Your capabilities: [{}]\n",
                    state.config.mesh.capabilities.join(", ")
                ));
                system_prompt.push_str(&format!("Connected peers ({}):\n", peers.len()));
                for p in &peers {
                    system_prompt.push_str(&format!(
                        "  - {} ({}) — capabilities: [{}]\n",
                        p.hostname,
                        &p.peer_id[..8.min(p.peer_id.len())],
                        p.capabilities.join(", "),
                    ));
                }
                system_prompt.push_str(
                    "Use mesh_delegate to send tasks to peers with capabilities you lack.\n",
                );
                system_prompt.push_str("</mesh_network>");
            }
        }
    }

    // Add available skills to system prompt (SKILL.md prompt-injection)
    {
        let skills = state.skills.lock().await;
        if let Some(block) = skills.system_prompt_block() {
            system_prompt.push_str(&block);
        }
    }

    // Add credential provider context so the LLM knows how to retrieve secrets
    if state.config.credentials.provider != "none" {
        system_prompt.push_str("\n\n<credentials>\n");
        system_prompt.push_str(&format!(
            "Provider: {}\n",
            state.config.credentials.provider
        ));
        if let Some(ref vault) = state.config.credentials.default_vault {
            system_prompt.push_str(&format!("Default vault: {}\n", vault));
        }
        let has_service_account = state.config.credentials.service_account_token.is_some();
        if has_service_account {
            system_prompt.push_str(
                "Mode: service account (headless — no biometric prompts)\n\
                 OP_SERVICE_ACCOUNT_TOKEN is set in the environment. The `op` CLI works without the desktop app.\n\
                 You can call `op` commands directly — no Touch ID or user interaction required.\n\n"
            );
        } else {
            system_prompt.push_str(
                "Mode: desktop app integration (biometric / Touch ID)\n\
                 The 1Password desktop app handles authentication via biometric unlock.\n\
                 IMPORTANT: To avoid repeated Touch ID prompts, batch credential lookups using `op run`:\n\
                   export FIELD1=\"op://Vault/Item/field1\"\n\
                   export FIELD2=\"op://Vault/Item/field2\"\n\
                   op run -- sh -c 'echo \"user=$FIELD1 pass=$FIELD2\"'\n\
                 This triggers biometric ONCE for the entire batch instead of per-command.\n\
                 For single lookups, `op read \"op://Vault/Item/field\"` is fine (one prompt).\n\n"
            );
        }
        system_prompt.push_str(
            "When a task requires credentials (passwords, API keys, tokens):\n\
             1. Check memory first with memory_search for the item name/vault mapping\n\
             2. Retrieve the credential using the provider CLI (e.g. `op read \"op://Vault/Item/field\"` or `op item get`)\n\
             3. Use the credential directly — never store the secret itself in memory\n\
             4. After first successful retrieval, store the MAPPING in memory (e.g. \"Plesk credentials → 1Password item 'Plesk Admin' in vault 'Servers'\")\n\
             The operator has pre-configured this provider. Proceed with credential retrieval without asking for permission.\n"
        );
        system_prompt.push_str("</credentials>");
    }

    let mut all_tools = state.tools.tools();
    all_tools.extend(state.plugins.tools());
    all_tools.extend(DeviceTools::tools());
    state.budget.reset_loop();
    let context_window = claw_config::resolve_context_window(
        state.config.agent.context_window,
        &state.config.agent.model,
    );
    let tool_result_max_tokens = state.config.agent.tool_result_max_tokens;
    {
        let mut mem = state.memory.lock().await;
        mem.working.set_context_window(
            session_id,
            context_window,
            state.config.agent.compaction_threshold,
        );
    }

    let autonomy_level = AutonomyLevel::from_u8(state.config.autonomy.level);
    let mut iteration = 0;
    let max_iterations = state.config.agent.max_iterations;
    let mut final_response = String::new();
    let mut consecutive_llm_failures: u32 = 0;

    // Wall-clock deadline for this request
    let started_at = std::time::Instant::now();
    let timeout_secs = state.config.agent.request_timeout_secs;
    let deadline = if timeout_secs > 0 {
        Some(started_at + std::time::Duration::from_secs(timeout_secs))
    } else {
        None
    };

    // Run serialization — acquire per-session lock to prevent interleaving
    let session_lock = state.sessions.run_lock(session_id).await;
    let _run_guard = session_lock.lock().await;

    // Track tool names from the previous turn to avoid misfiring lazy-stop
    let mut last_turn_tool_names: Vec<String> = Vec::new();

    // 3. THINK + ACT loop
    loop {
        iteration += 1;
        if iteration > max_iterations {
            warn!(session = %session_id, "max agent iterations reached");
            break;
        }

        // Check wall-clock timeout
        if let Some(dl) = deadline {
            if std::time::Instant::now() >= dl {
                warn!(session = %session_id, elapsed_secs = started_at.elapsed().as_secs(), "request timeout reached");
                final_response = format!(
                    "I ran out of time ({}s limit reached after {} iterations). Here's what I accomplished so far. \
                     You can send another message to continue where I left off.",
                    timeout_secs,
                    iteration - 1
                );
                break;
            }
        }

        state.event_bus.publish(Event::AgentThinking { session_id });
        state.budget.check()?;

        // Try LLM-powered compaction before reading messages if context is large
        let _ = maybe_compact_context(state, session_id).await;

        // Read messages — brief lock
        let messages = {
            let mem = state.memory.lock().await;
            mem.working.messages(session_id).to_vec()
        };

        let request = LlmRequest {
            model: if consecutive_llm_failures >= 3 {
                // After 3 consecutive failures from primary, switch to fallback for this run
                state
                    .config
                    .agent
                    .fallback_model
                    .as_deref()
                    .unwrap_or(&state.config.agent.model)
                    .to_string()
            } else {
                state.config.agent.model.clone()
            },
            messages,
            tools: all_tools.clone(),
            system: Some(system_prompt.clone()),
            max_tokens: state.config.agent.max_tokens,
            temperature: state.config.agent.temperature,
            thinking_level: Some(state.config.agent.thinking_level.clone()),
            stream: false,
        };

        // Call LLM with overflow recovery and model fallback
        let response = match state
            .llm
            .complete(&request, state.config.agent.fallback_model.as_deref())
            .await
        {
            Ok(resp) => {
                consecutive_llm_failures = 0;
                resp
            }
            Err(ref e)
                if matches!(
                    e,
                    claw_core::ClawError::ContextOverflow { .. }
                        | claw_core::ClawError::LlmProvider(_)
                ) && iteration <= max_iterations =>
            {
                consecutive_llm_failures += 1;
                // Context might be too large — force compaction and retry
                warn!(session = %session_id, error = %e, consecutive_failures = consecutive_llm_failures,
                    "LLM call failed, attempting emergency compaction");
                {
                    let mut mem = state.memory.lock().await;
                    mem.working.compact(session_id);
                }
                // Retry with compacted context
                let messages = {
                    let mem = state.memory.lock().await;
                    mem.working.messages(session_id).to_vec()
                };
                let retry_request = LlmRequest {
                    messages,
                    ..request
                };
                state
                    .llm
                    .complete(&retry_request, state.config.agent.fallback_model.as_deref())
                    .await?
            }
            Err(e) => return Err(e),
        };

        state
            .budget
            .record_spend(response.usage.estimated_cost_usd)?;

        // Store assistant message — brief lock
        {
            let mut mem = state.memory.lock().await;
            let mut assistant_msg = response.message.clone();
            assistant_msg.session_id = session_id;
            mem.working.push(assistant_msg);
        }
        state.sessions.record_message(session_id).await;

        if !response.has_tool_calls {
            // Check WHY the model stopped — don't just break blindly
            match response.stop_reason {
                StopReason::MaxTokens => {
                    // Model was cut off mid-output — inject continuation prompt and loop
                    info!(session = %session_id, iteration, "model hit max_tokens, injecting continuation prompt");
                    let mut mem = state.memory.lock().await;
                    let continue_msg = Message::text(
                        session_id,
                        Role::User,
                        "[SYSTEM: Your previous response was truncated because it exceeded the output token limit. \
                         Continue exactly where you left off. Do NOT repeat what you already said or re-explain — \
                         just keep going with the next tool calls or remaining work.]",
                    );
                    mem.working.push(continue_msg);
                    continue;
                }
                _ => {
                    // Model chose to stop — check if it's being lazy
                    // Skip lazy-stop if last turn started a dev server / background process
                    let text = response.message.text_content();
                    let lower = text.to_lowercase();
                    let just_started_server = last_turn_tool_names
                        .iter()
                        .any(|name| name == "process_start" || name == "terminal_run")
                        && (lower.contains("localhost")
                            || lower.contains("running")
                            || lower.contains("dev server")
                            || lower.contains("started"));
                    if !just_started_server
                        && is_lazy_stop(&text, iteration as usize)
                        && iteration < max_iterations
                    {
                        info!(session = %session_id, iteration, "detected lazy model stop, re-prompting");
                        let mut mem = state.memory.lock().await;
                        let nudge_msg = Message::text(
                            session_id,
                            Role::User,
                            "[SYSTEM: You stopped but the task is NOT complete. Do NOT describe what could be done — \
                             actually DO it. Use your tools to create the remaining files and finish the job. \
                             Continue working now.]",
                        );
                        mem.working.push(nudge_msg);
                        continue;
                    }
                    // Genuinely done
                    final_response = text;
                    state.event_bus.publish(Event::AgentResponse {
                        session_id,
                        message_id: response.message.id,
                    });
                    break;
                }
            }
        }

        // 4. GUARD + ACT — execute tool calls with guardrail checks
        // Partition into parallel-safe and sequential tool calls
        let tool_calls_ref = &response.message.tool_calls;
        let parallel_enabled = state.config.agent.parallel_tool_calls;
        let can_parallelize = parallel_enabled && tool_calls_ref.len() > 1;

        if can_parallelize
            && tool_calls_ref
                .iter()
                .all(|tc| is_parallel_safe(&tc.tool_name))
        {
            // All tool calls are parallel-safe — run them all concurrently
            let mut join_set = tokio::task::JoinSet::new();
            for tool_call in tool_calls_ref.clone() {
                state.budget.record_tool_call()?;
                state.event_bus.publish(Event::AgentToolCall {
                    session_id,
                    tool_name: tool_call.tool_name.clone(),
                    tool_call_id: tool_call.id.clone(),
                });
                let tool_def = all_tools
                    .iter()
                    .find(|t| t.name == tool_call.tool_name)
                    .cloned()
                    .unwrap_or_else(|| Tool {
                        name: tool_call.tool_name.clone(),
                        description: String::new(),
                        parameters: serde_json::Value::Null,
                        capabilities: vec![],
                        is_mutating: true,
                        risk_level: 5,
                        provider: None,
                    });
                let verdict = state
                    .guardrails
                    .evaluate(&tool_def, &tool_call, autonomy_level);
                let s = state.clone();
                let tc = tool_call.clone();
                let tc_id = tool_call.id.clone();
                join_set.spawn(async move {
                    let result = match verdict {
                        GuardrailVerdict::Approve => execute_tool_shared(&s, &tc).await,
                        GuardrailVerdict::Deny(reason) => ToolResult {
                            tool_call_id: tc_id.clone(),
                            content: format!("DENIED: {}", reason),
                            is_error: true,
                            data: None,
                        },
                        _ => execute_tool_shared(&s, &tc).await,
                    };
                    (tc_id, tc.tool_name.clone(), result)
                });
            }

            // Collect results as they complete
            while let Some(join_result) = join_set.join_next().await {
                if let Ok((tc_id, _tool_name, tool_result)) = join_result {
                    let is_error = tool_result.is_error;
                    state.event_bus.publish(Event::AgentToolResult {
                        session_id,
                        tool_call_id: tc_id.clone(),
                        is_error,
                    });
                    let truncated_content =
                        truncate_tool_result(&tool_result.content, tool_result_max_tokens);
                    {
                        let mut mem = state.memory.lock().await;
                        let result_msg = Message {
                            id: Uuid::new_v4(),
                            session_id,
                            role: Role::Tool,
                            content: vec![claw_core::MessageContent::ToolResult {
                                tool_call_id: tc_id,
                                content: truncated_content,
                                is_error: tool_result.is_error,
                            }],
                            timestamp: chrono::Utc::now(),
                            tool_calls: vec![],
                            metadata: Default::default(),
                        };
                        mem.working.push(result_msg);
                    }
                }
            }
        } else {
            // Sequential execution (original path) — either parallel disabled or has mutating tools
            for tool_call in &response.message.tool_calls {
                state.budget.record_tool_call()?;

                state.event_bus.publish(Event::AgentToolCall {
                    session_id,
                    tool_name: tool_call.tool_name.clone(),
                    tool_call_id: tool_call.id.clone(),
                });

                let tool_def = all_tools
                    .iter()
                    .find(|t| t.name == tool_call.tool_name)
                    .cloned()
                    .unwrap_or_else(|| Tool {
                        name: tool_call.tool_name.clone(),
                        description: String::new(),
                        parameters: serde_json::Value::Null,
                        capabilities: vec![],
                        is_mutating: true,
                        risk_level: 5,
                        provider: None,
                    });

                let verdict = state
                    .guardrails
                    .evaluate(&tool_def, tool_call, autonomy_level);
                let tool_result = match verdict {
                    GuardrailVerdict::Approve => execute_tool_shared(state, tool_call).await,
                    GuardrailVerdict::Deny(reason) => ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        content: format!("DENIED: {}", reason),
                        is_error: true,
                        data: None,
                    },
                    GuardrailVerdict::Escalate(reason) => {
                        // Generate approval ID upfront so we can include it in the prompt
                        let approval_id = Uuid::new_v4();

                        // Send approval prompt to the originating channel
                        send_approval_prompt_shared(
                            state,
                            &channel_id_owned,
                            &target_owned,
                            &approval_id.to_string(),
                            &tool_call.tool_name,
                            &tool_call.arguments,
                            &reason,
                            tool_def.risk_level,
                        )
                        .await;

                        // Now wait for approval (resolved via callback query, /approve command, or API)
                        let response = state
                            .approval
                            .request_approval_with_id(
                                approval_id,
                                &tool_call.tool_name,
                                &tool_call.arguments,
                                &reason,
                                tool_def.risk_level,
                                120,
                            )
                            .await;
                        match response {
                            ApprovalResponse::Approved => {
                                execute_tool_shared(state, tool_call).await
                            }
                            ApprovalResponse::Denied => ToolResult {
                                tool_call_id: tool_call.id.clone(),
                                content: "DENIED: Human denied the action".into(),
                                is_error: true,
                                data: None,
                            },
                            ApprovalResponse::TimedOut => ToolResult {
                                tool_call_id: tool_call.id.clone(),
                                content: "DENIED: Approval request timed out".into(),
                                is_error: true,
                                data: None,
                            },
                        }
                    }
                };

                let is_error = tool_result.is_error;
                state.event_bus.publish(Event::AgentToolResult {
                    session_id,
                    tool_call_id: tool_call.id.clone(),
                    is_error,
                });

                // Truncate tool result to fit context window
                let truncated_content =
                    truncate_tool_result(&tool_result.content, tool_result_max_tokens);

                // Store tool result — brief lock
                {
                    let mut mem = state.memory.lock().await;
                    let result_msg = Message {
                        id: Uuid::new_v4(),
                        session_id,
                        role: Role::Tool,
                        content: vec![claw_core::MessageContent::ToolResult {
                            tool_call_id: tool_call.id.clone(),
                            content: truncated_content,
                            is_error: tool_result.is_error,
                        }],
                        timestamp: chrono::Utc::now(),
                        tool_calls: vec![],
                        metadata: Default::default(),
                    };
                    mem.working.push(result_msg);
                }
            }
        }

        // Try LLM-powered compaction if context is getting large
        let _ = maybe_compact_context(state, session_id).await;

        // Record this turn's tool names for next iteration's lazy-stop check
        last_turn_tool_names = response
            .message
            .tool_calls
            .iter()
            .map(|tc| tc.tool_name.clone())
            .collect();
    }

    // Auto-resume: if we hit max_iterations or timeout with active goals, schedule a resume
    if state.config.agent.auto_resume {
        let was_interrupted = iteration > max_iterations
            || deadline.is_some_and(|dl| std::time::Instant::now() >= dl);
        if was_interrupted {
            let has_active_goals = {
                let planner = state.planner.lock().await;
                !planner.active_goals().is_empty()
            };
            if has_active_goals {
                if let Some(ref scheduler) = state.scheduler {
                    let resume_desc = format!(
                        "Auto-resume: Continue working on unfinished tasks from session {}. \
                         Review active goals with goal_list and continue where you left off.",
                        session_id
                    );
                    let task_id = scheduler
                        .add_one_shot(
                            resume_desc,
                            60, // Resume in 60 seconds
                            Some(format!("auto-resume:{}", session_id)),
                            Some(session_id),
                        )
                        .await;
                    info!(
                        task_id = %task_id,
                        session = %session_id,
                        "scheduled auto-resume in 60s for interrupted task"
                    );
                    final_response
                        .push_str("\n\n⏱️ I'll automatically resume this work in about 1 minute.");
                }
            }
        }
    }

    // 5. REMEMBER — record episodic memory + audit
    {
        let mut mem = state.memory.lock().await;
        mem.audit("message", "processed", Some(&user_text))?;

        // Build a brief summary for episodic memory from the conversation
        let messages = mem.working.messages(session_id);
        let msg_count = messages.len();
        if msg_count >= 2 {
            let summary = build_episode_summary(messages, &user_text, &final_response);
            let episode = claw_memory::episodic::Episode {
                id: uuid::Uuid::new_v4(),
                session_id,
                summary,
                outcome: if final_response.is_empty() {
                    None
                } else {
                    Some("completed".to_string())
                },
                tags: extract_episode_tags(&user_text),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            mem.episodic.record(episode);
        }
    }

    // 6. LEARN — extract lessons from error→correction→success patterns
    maybe_extract_lessons(state, session_id).await;

    // Auto-set session label from first user message if not yet set
    if let Some(session) = state.sessions.get(session_id).await {
        if session.name.is_none() && !user_text.is_empty() {
            let label: String = user_text.chars().take(60).collect();
            let label = label
                .split('\n')
                .next()
                .unwrap_or(&label)
                .trim()
                .to_string();
            state.sessions.set_name(session_id, &label).await;
        }
    }

    Ok(final_response)
}

/// Core streaming message processing using shared state with fine-grained locks.
async fn process_message_streaming_shared(
    state: &SharedAgentState,
    channel_id: &str,
    incoming: IncomingMessage,
    tx: &mpsc::Sender<StreamEvent>,
    override_session_id: Option<Uuid>,
) -> claw_core::Result<()> {
    let target = incoming.group.as_deref().unwrap_or(&incoming.sender);
    let session_id = match override_session_id {
        Some(id) => id,
        None => state.sessions.find_or_create(channel_id, target).await,
    };

    // Store reply context so channel_send_file tool can route to the right channel
    {
        let mut ctx = state.reply_context.lock().await;
        *ctx = Some((channel_id.to_string(), target.to_string()));
    }

    // Store stream tx so sub-agents can forward their events to the parent stream
    {
        let mut stx = state.stream_tx.lock().await;
        *stx = Some(tx.clone());
    }

    let user_text = incoming.text.unwrap_or_default();

    // 1. RECEIVE + RECALL — embed query (before lock) then search memory
    let query_embedding = if let Some(ref embedder) = state.embedder {
        match embedder.embed(&[&user_text]).await {
            Ok(vecs) if !vecs.is_empty() => Some(vecs.into_iter().next().unwrap()),
            _ => None,
        }
    } else {
        None
    };

    let (context_parts, active_goals) = {
        let mut mem = state.memory.lock().await;
        let user_msg = Message::text(session_id, Role::User, &user_text);
        mem.working.push(user_msg);
        drop(mem);
        state.sessions.record_message(session_id).await;
        let mem = state.memory.lock().await;

        let relevant_episodes = mem.episodic.search(&user_text);

        // Build a combined keyword query from user text for broader matching
        let search_terms = extract_search_keywords(&user_text);

        // Collect facts from multiple search strategies, dedup by category+key
        let mut seen_fact_keys = std::collections::HashSet::new();
        let mut relevant_facts: Vec<String> = Vec::new();

        // Strategy 1: Vector search
        if let Some(ref qemb) = query_embedding {
            for (fact, _score) in mem.semantic.vector_search(qemb, 10) {
                let fk = format!("{}:{}", fact.category, fact.key);
                if seen_fact_keys.insert(fk) {
                    relevant_facts.push(format!(
                        "- [{}] {}: {}",
                        fact.category, fact.key, fact.value
                    ));
                }
            }
        }

        // Strategy 2: Word-level keyword search on user text
        for fact in mem.semantic.search(&user_text).iter().take(10) {
            let fk = format!("{}:{}", fact.category, fact.key);
            if seen_fact_keys.insert(fk) {
                relevant_facts.push(format!(
                    "- [{}] {}: {}",
                    fact.category, fact.key, fact.value
                ));
            }
        }

        // Strategy 3: Search with extracted keywords
        if search_terms != user_text.to_lowercase() {
            for fact in mem.semantic.search(&search_terms).iter().take(5) {
                let fk = format!("{}:{}", fact.category, fact.key);
                if seen_fact_keys.insert(fk) {
                    relevant_facts.push(format!(
                        "- [{}] {}: {}",
                        fact.category, fact.key, fact.value
                    ));
                }
            }
        }

        relevant_facts.truncate(15);

        let mut parts = Vec::new();
        if !relevant_episodes.is_empty() {
            let episodes_text: Vec<String> = relevant_episodes
                .iter()
                .take(5)
                .map(|e| format!("- {}", e.summary))
                .collect();
            parts.push(format!(
                "Relevant past conversations:\n{}",
                episodes_text.join("\n")
            ));
        }
        if !relevant_facts.is_empty() {
            parts.push(format!(
                "Relevant knowledge:\n{}",
                relevant_facts.join("\n")
            ));
        }

        // Always load learned lessons — these are high-value self-corrections
        let lessons: Vec<String> = mem
            .semantic
            .category("learned_lessons")
            .iter()
            .map(|f| format!("- **{}**: {}", f.key, f.value))
            .collect();
        if !lessons.is_empty() {
            parts.push(format!(
                "Lessons learned from past sessions (apply these!):\n{}",
                lessons.join("\n")
            ));
        }

        drop(mem);

        let planner = state.planner.lock().await;
        let goals: Vec<_> = planner.active_goals().into_iter().cloned().collect();
        drop(planner);

        (parts, goals)
    };

    // 2. BUILD system prompt — no locks needed
    let mut system_prompt = state
        .config
        .agent
        .system_prompt
        .clone()
        .unwrap_or_else(build_default_system_prompt);
    if !context_parts.is_empty() {
        system_prompt.push_str("\n\n<memory>\n");
        system_prompt.push_str(&context_parts.join("\n\n"));
        system_prompt.push_str("\n</memory>");
    }
    if !active_goals.is_empty() {
        system_prompt.push_str("\n\n<active_goals>\n");
        for goal in &active_goals {
            system_prompt.push_str(&format!(
                "- [{}] {} (progress: {:.0}%)\n",
                goal.id,
                goal.description,
                goal.progress * 100.0
            ));
        }
        system_prompt.push_str("</active_goals>");
    }

    // Add mesh peer context so the LLM knows about the network
    {
        let mesh = state.mesh.lock().await;
        if mesh.is_running() {
            let peers = mesh.peer_list();
            if !peers.is_empty() {
                system_prompt.push_str("\n\n<mesh_network>\n");
                system_prompt.push_str(&format!(
                    "Your peer ID: {}\n",
                    &mesh.peer_id()[..12.min(mesh.peer_id().len())]
                ));
                system_prompt.push_str(&format!(
                    "Your capabilities: [{}]\n",
                    state.config.mesh.capabilities.join(", ")
                ));
                system_prompt.push_str(&format!("Connected peers ({}):\n", peers.len()));
                for p in &peers {
                    system_prompt.push_str(&format!(
                        "  - {} ({}) — capabilities: [{}]\n",
                        p.hostname,
                        &p.peer_id[..8.min(p.peer_id.len())],
                        p.capabilities.join(", "),
                    ));
                }
                system_prompt.push_str(
                    "Use mesh_delegate to send tasks to peers with capabilities you lack.\n",
                );
                system_prompt.push_str("</mesh_network>");
            }
        }
    }

    // Add available skills to system prompt (SKILL.md prompt-injection)
    {
        let skills = state.skills.lock().await;
        if let Some(block) = skills.system_prompt_block() {
            system_prompt.push_str(&block);
        }
    }

    // Add credential provider context so the LLM knows how to retrieve secrets
    if state.config.credentials.provider != "none" {
        system_prompt.push_str("\n\n<credentials>\n");
        system_prompt.push_str(&format!(
            "Provider: {}\n",
            state.config.credentials.provider
        ));
        if let Some(ref vault) = state.config.credentials.default_vault {
            system_prompt.push_str(&format!("Default vault: {}\n", vault));
        }
        let has_service_account = state.config.credentials.service_account_token.is_some();
        if has_service_account {
            system_prompt.push_str(
                "Mode: service account (headless — no biometric prompts)\n\
                 OP_SERVICE_ACCOUNT_TOKEN is set in the environment. The `op` CLI works without the desktop app.\n\
                 You can call `op` commands directly — no Touch ID or user interaction required.\n\n"
            );
        } else {
            system_prompt.push_str(
                "Mode: desktop app integration (biometric / Touch ID)\n\
                 The 1Password desktop app handles authentication via biometric unlock.\n\
                 IMPORTANT: To avoid repeated Touch ID prompts, batch credential lookups using `op run`:\n\
                   export FIELD1=\"op://Vault/Item/field1\"\n\
                   export FIELD2=\"op://Vault/Item/field2\"\n\
                   op run -- sh -c 'echo \"user=$FIELD1 pass=$FIELD2\"'\n\
                 This triggers biometric ONCE for the entire batch instead of per-command.\n\
                 For single lookups, `op read \"op://Vault/Item/field\"` is fine (one prompt).\n\n"
            );
        }
        system_prompt.push_str(
            "When a task requires credentials (passwords, API keys, tokens):\n\
             1. Check memory first with memory_search for the item name/vault mapping\n\
             2. Retrieve the credential using the provider CLI (e.g. `op read \"op://Vault/Item/field\"` or `op item get`)\n\
             3. Use the credential directly — never store the secret itself in memory\n\
             4. After first successful retrieval, store the MAPPING in memory (e.g. \"Plesk credentials → 1Password item 'Plesk Admin' in vault 'Servers'\")\n\
             The operator has pre-configured this provider. Proceed with credential retrieval without asking for permission.\n"
        );
        system_prompt.push_str("</credentials>");
    }

    let mut all_tools = state.tools.tools();
    all_tools.extend(state.plugins.tools());
    all_tools.extend(DeviceTools::tools());
    state.budget.reset_loop();

    let autonomy_level = AutonomyLevel::from_u8(state.config.autonomy.level);
    let mut iteration = 0;
    let max_iterations = state.config.agent.max_iterations;
    let tool_result_max_tokens = state.config.agent.tool_result_max_tokens;
    let mut consecutive_llm_failures: u32 = 0;

    // Wall-clock deadline for this request
    let started_at = std::time::Instant::now();
    let timeout_secs = state.config.agent.request_timeout_secs;
    let deadline = if timeout_secs > 0 {
        Some(started_at + std::time::Duration::from_secs(timeout_secs))
    } else {
        None
    };

    // Run serialization — acquire per-session lock to prevent interleaving
    let session_lock = state.sessions.run_lock(session_id).await;
    let _run_guard = session_lock.lock().await;

    // Track tool names from the previous turn to avoid misfiring lazy-stop
    // after legitimate completion (e.g. process_start for dev server).
    let mut last_turn_tool_names: Vec<String> = Vec::new();

    // Configure context window for this session
    let context_window = claw_config::resolve_context_window(
        state.config.agent.context_window,
        &state.config.agent.model,
    );
    {
        let mut mem = state.memory.lock().await;
        mem.working.set_context_window(
            session_id,
            context_window,
            state.config.agent.compaction_threshold,
        );
    }

    // 3. THINK + ACT loop with streaming
    loop {
        iteration += 1;
        if iteration > max_iterations {
            warn!(session = %session_id, "max agent iterations reached");
            break;
        }

        // Check wall-clock timeout
        if let Some(dl) = deadline {
            if std::time::Instant::now() >= dl {
                warn!(session = %session_id, elapsed_secs = started_at.elapsed().as_secs(), "request timeout reached in streaming loop");
                let _ = tx.send(StreamEvent::TextDelta {
                    content: format!(
                        "\n\n⏱️ Time limit reached ({}s, {} iterations). Send another message to continue.",
                        timeout_secs, iteration - 1
                    ),
                }).await;
                break;
            }
        }

        state.budget.check()?;

        // Try LLM-powered compaction before reading messages if context is large
        let _ = maybe_compact_context(state, session_id).await;

        // Read messages — brief lock
        let messages = {
            let mem = state.memory.lock().await;
            mem.working.messages(session_id).to_vec()
        };

        let request = LlmRequest {
            model: if consecutive_llm_failures >= 3 {
                state
                    .config
                    .agent
                    .fallback_model
                    .as_deref()
                    .unwrap_or(&state.config.agent.model)
                    .to_string()
            } else {
                state.config.agent.model.clone()
            },
            messages,
            tools: all_tools.clone(),
            system: Some(system_prompt.clone()),
            max_tokens: state.config.agent.max_tokens,
            temperature: state.config.agent.temperature,
            thinking_level: Some(state.config.agent.thinking_level.clone()),
            stream: true,
        };

        // Stream from LLM with overflow recovery and model fallback
        let mut chunk_rx = match state
            .llm
            .stream(&request, state.config.agent.fallback_model.as_deref())
            .await
        {
            Ok(rx) => {
                consecutive_llm_failures = 0;
                rx
            }
            Err(ref e)
                if matches!(
                    e,
                    claw_core::ClawError::ContextOverflow { .. }
                        | claw_core::ClawError::LlmProvider(_)
                ) && iteration <= max_iterations =>
            {
                consecutive_llm_failures += 1;
                warn!(session = %session_id, error = %e, consecutive_failures = consecutive_llm_failures,
                    "stream call failed, attempting emergency compaction");
                {
                    let mut mem = state.memory.lock().await;
                    mem.working.compact(session_id);
                }
                let messages = {
                    let mem = state.memory.lock().await;
                    mem.working.messages(session_id).to_vec()
                };
                let retry_request = LlmRequest {
                    messages,
                    ..request
                };
                state
                    .llm
                    .stream(&retry_request, state.config.agent.fallback_model.as_deref())
                    .await?
            }
            Err(e) => return Err(e),
        };

        let mut full_text = String::new();
        let mut tool_calls: Vec<claw_core::ToolCall> = Vec::new();
        let mut total_usage = claw_llm::Usage::default();
        let mut has_tool_calls = false;
        let mut stop_reason = StopReason::EndTurn;

        // Process stream chunks — no lock needed
        while let Some(chunk) = chunk_rx.recv().await {
            match chunk {
                claw_llm::StreamChunk::TextDelta(text) => {
                    full_text.push_str(&text);
                    let _ = tx.send(StreamEvent::TextDelta { content: text }).await;
                }
                claw_llm::StreamChunk::Thinking(text) => {
                    let _ = tx.send(StreamEvent::Thinking { content: text }).await;
                }
                claw_llm::StreamChunk::ToolCall(tc) => {
                    let _ = tx
                        .send(StreamEvent::ToolCall {
                            name: tc.tool_name.clone(),
                            id: tc.id.clone(),
                            args: tc.arguments.clone(),
                        })
                        .await;
                    tool_calls.push(tc);
                    has_tool_calls = true;
                }
                claw_llm::StreamChunk::Usage(usage) => {
                    total_usage.merge(&usage);
                    let _ = tx
                        .send(StreamEvent::Usage {
                            input_tokens: usage.input_tokens,
                            output_tokens: usage.output_tokens,
                            cost_usd: usage.estimated_cost_usd,
                        })
                        .await;
                }
                claw_llm::StreamChunk::Done(reason) => {
                    stop_reason = reason;
                    break;
                }
                claw_llm::StreamChunk::Error(e) => {
                    let _ = tx.send(StreamEvent::Error { message: e }).await;
                    return Ok(());
                }
            }
        }

        state.budget.record_spend(total_usage.estimated_cost_usd)?;

        // Store assistant message — brief lock
        {
            let mut mem = state.memory.lock().await;
            let mut assistant_msg = Message::text(session_id, Role::Assistant, &full_text);
            assistant_msg.tool_calls = tool_calls.clone();
            mem.working.push(assistant_msg);
        }
        state.sessions.record_message(session_id).await;

        if !has_tool_calls {
            // Check WHY the model stopped — don't just break blindly
            match stop_reason {
                StopReason::MaxTokens => {
                    // Model was cut off mid-output — inject continuation prompt and loop
                    info!(session = %session_id, iteration, "model hit max_tokens in stream, injecting continuation prompt");
                    let mut mem = state.memory.lock().await;
                    let continue_msg = Message::text(
                        session_id,
                        Role::User,
                        "[SYSTEM: Your previous response was truncated because it exceeded the output token limit. \
                         Continue exactly where you left off. Do NOT repeat what you already said or re-explain — \
                         just keep going with the next tool calls or remaining work.]",
                    );
                    mem.working.push(continue_msg);
                    let _ = tx
                        .send(StreamEvent::TextDelta {
                            content: "\n\n*Continuing...*\n\n".to_string(),
                        })
                        .await;
                    continue;
                }
                _ => {
                    // Model chose to stop — check if it's being lazy
                    // BUT: skip lazy-stop if the model just started a dev server
                    // or background process — that's a legitimate final step.
                    let just_started_server = last_turn_tool_names
                        .iter()
                        .any(|name| name == "process_start" || name == "terminal_run")
                        && (full_text.to_lowercase().contains("localhost")
                            || full_text.to_lowercase().contains("running")
                            || full_text.to_lowercase().contains("dev server")
                            || full_text.to_lowercase().contains("started"));
                    if !just_started_server
                        && is_lazy_stop(&full_text, iteration as usize)
                        && iteration < max_iterations
                    {
                        info!(session = %session_id, iteration, "detected lazy model stop in stream, re-prompting");
                        let mut mem = state.memory.lock().await;
                        let nudge_msg = Message::text(
                            session_id,
                            Role::User,
                            "[SYSTEM: You stopped but the task is NOT complete. Do NOT describe what could be done — \
                             actually DO it. Use your tools to create the remaining files and finish the job. \
                             Continue working now.]",
                        );
                        mem.working.push(nudge_msg);
                        let _ = tx
                            .send(StreamEvent::TextDelta {
                                content: "\n\n*Continuing...*\n\n".to_string(),
                            })
                            .await;
                        continue;
                    }
                    // Genuinely done
                    break;
                }
            }
        }

        // 4. Execute tool calls with guardrails — parallel when safe
        let parallel_enabled = state.config.agent.parallel_tool_calls;
        let can_parallelize = parallel_enabled && tool_calls.len() > 1;

        if can_parallelize && tool_calls.iter().all(|tc| is_parallel_safe(&tc.tool_name)) {
            // All tool calls are parallel-safe — run them all concurrently
            let mut join_set = tokio::task::JoinSet::new();
            for tool_call in tool_calls.clone() {
                state.budget.record_tool_call()?;
                let tool_def = all_tools
                    .iter()
                    .find(|t| t.name == tool_call.tool_name)
                    .cloned()
                    .unwrap_or_else(|| Tool {
                        name: tool_call.tool_name.clone(),
                        description: String::new(),
                        parameters: serde_json::Value::Null,
                        capabilities: vec![],
                        is_mutating: true,
                        risk_level: 5,
                        provider: None,
                    });
                let verdict = state
                    .guardrails
                    .evaluate(&tool_def, &tool_call, autonomy_level);
                let s = state.clone();
                let tc = tool_call.clone();
                let tc_id = tool_call.id.clone();
                join_set.spawn(async move {
                    let result = match verdict {
                        GuardrailVerdict::Approve => execute_tool_shared(&s, &tc).await,
                        GuardrailVerdict::Deny(reason) => ToolResult {
                            tool_call_id: tc_id.clone(),
                            content: format!("DENIED: {}", reason),
                            is_error: true,
                            data: None,
                        },
                        _ => execute_tool_shared(&s, &tc).await,
                    };
                    (tc_id, result)
                });
            }

            // Collect results as they complete and stream them back
            while let Some(join_result) = join_set.join_next().await {
                if let Ok((tc_id, tool_result)) = join_result {
                    let _ = tx
                        .send(StreamEvent::ToolResult {
                            id: tc_id.clone(),
                            content: tool_result.content.clone(),
                            is_error: tool_result.is_error,
                            data: tool_result.data.clone(),
                        })
                        .await;
                    let truncated_content =
                        truncate_tool_result(&tool_result.content, tool_result_max_tokens);
                    {
                        let mut mem = state.memory.lock().await;
                        let result_msg = Message {
                            id: Uuid::new_v4(),
                            session_id,
                            role: Role::Tool,
                            content: vec![claw_core::MessageContent::ToolResult {
                                tool_call_id: tc_id,
                                content: truncated_content,
                                is_error: tool_result.is_error,
                            }],
                            timestamp: chrono::Utc::now(),
                            tool_calls: vec![],
                            metadata: Default::default(),
                        };
                        mem.working.push(result_msg);
                    }
                }
            }
        } else {
            // Sequential execution (original path)
            for tool_call in &tool_calls {
                state.budget.record_tool_call()?;

                let tool_def = all_tools
                    .iter()
                    .find(|t| t.name == tool_call.tool_name)
                    .cloned()
                    .unwrap_or_else(|| Tool {
                        name: tool_call.tool_name.clone(),
                        description: String::new(),
                        parameters: serde_json::Value::Null,
                        capabilities: vec![],
                        is_mutating: true,
                        risk_level: 5,
                        provider: None,
                    });

                let verdict = state
                    .guardrails
                    .evaluate(&tool_def, tool_call, autonomy_level);
                let tool_result = match verdict {
                    GuardrailVerdict::Approve => execute_tool_shared(state, tool_call).await,
                    GuardrailVerdict::Deny(reason) => ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        content: format!("DENIED: {}", reason),
                        is_error: true,
                        data: None,
                    },
                    GuardrailVerdict::Escalate(_reason) => {
                        let approval_id = Uuid::new_v4();

                        // Emit approval event to stream so UI can show approve/deny
                        let _ = tx
                            .send(StreamEvent::ApprovalRequired {
                                id: approval_id.to_string(),
                                tool_name: tool_call.tool_name.clone(),
                                tool_args: tool_call.arguments.clone(),
                                reason: _reason.clone(),
                                risk_level: tool_def.risk_level,
                            })
                            .await;

                        // Wait for approval — no lock held during this potentially long wait
                        let response = state
                            .approval
                            .request_approval_with_id(
                                approval_id,
                                &tool_call.tool_name,
                                &tool_call.arguments,
                                &_reason,
                                tool_def.risk_level,
                                120,
                            )
                            .await;
                        match response {
                            ApprovalResponse::Approved => {
                                execute_tool_shared(state, tool_call).await
                            }
                            ApprovalResponse::Denied => ToolResult {
                                tool_call_id: tool_call.id.clone(),
                                content: "DENIED: Human denied the action".into(),
                                is_error: true,
                                data: None,
                            },
                            ApprovalResponse::TimedOut => ToolResult {
                                tool_call_id: tool_call.id.clone(),
                                content: "DENIED: Approval request timed out".into(),
                                is_error: true,
                                data: None,
                            },
                        }
                    }
                };

                let _ = tx
                    .send(StreamEvent::ToolResult {
                        id: tool_call.id.clone(),
                        content: tool_result.content.clone(),
                        is_error: tool_result.is_error,
                        data: tool_result.data.clone(),
                    })
                    .await;

                // Truncate tool result to fit context window
                let truncated_content =
                    truncate_tool_result(&tool_result.content, tool_result_max_tokens);

                // Store tool result — brief lock
                {
                    let mut mem = state.memory.lock().await;
                    let result_msg = Message {
                        id: Uuid::new_v4(),
                        session_id,
                        role: Role::Tool,
                        content: vec![claw_core::MessageContent::ToolResult {
                            tool_call_id: tool_call.id.clone(),
                            content: truncated_content,
                            is_error: tool_result.is_error,
                        }],
                        timestamp: chrono::Utc::now(),
                        tool_calls: vec![],
                        metadata: Default::default(),
                    };
                    mem.working.push(result_msg);
                }
            }
        }

        // Try LLM-powered compaction if context is getting large
        let _ = maybe_compact_context(state, session_id).await;

        // Record this turn's tool names for next iteration's lazy-stop check
        last_turn_tool_names = tool_calls.iter().map(|tc| tc.tool_name.clone()).collect();
    }

    // Auto-resume: if we hit max_iterations or timeout with active goals, schedule a resume
    if state.config.agent.auto_resume {
        let was_interrupted = iteration > max_iterations
            || deadline.is_some_and(|dl| std::time::Instant::now() >= dl);
        if was_interrupted {
            let has_active_goals = {
                let planner = state.planner.lock().await;
                !planner.active_goals().is_empty()
            };
            if has_active_goals {
                if let Some(ref scheduler) = state.scheduler {
                    let resume_desc = format!(
                        "Auto-resume: Continue working on unfinished tasks from session {}. \
                         Review active goals with goal_list and continue where you left off.",
                        session_id
                    );
                    let task_id = scheduler
                        .add_one_shot(
                            resume_desc,
                            60, // Resume in 60 seconds
                            Some(format!("auto-resume:{}", session_id)),
                            Some(session_id),
                        )
                        .await;
                    info!(
                        task_id = %task_id,
                        session = %session_id,
                        "scheduled auto-resume in 60s for interrupted streaming task"
                    );
                    let _ = tx
                        .send(StreamEvent::TextDelta {
                            content:
                                "\n\n⏱️ I'll automatically resume this work in about 1 minute."
                                    .to_string(),
                        })
                        .await;
                }
            }
        }
    }

    // 5. REMEMBER — record episodic memory + audit
    {
        let mut mem = state.memory.lock().await;
        mem.audit("message", "processed", Some(&user_text))?;

        // Build a brief summary for episodic memory
        let messages = mem.working.messages(session_id);
        let msg_count = messages.len();
        if msg_count >= 2 {
            // Get last assistant text for summary
            let last_assistant = messages
                .iter()
                .rev()
                .find(|m| m.role == Role::Assistant)
                .map(|m| m.text_content())
                .unwrap_or_default();
            let summary = build_episode_summary(messages, &user_text, &last_assistant);
            let episode = claw_memory::episodic::Episode {
                id: uuid::Uuid::new_v4(),
                session_id,
                summary,
                outcome: Some("completed".to_string()),
                tags: extract_episode_tags(&user_text),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            mem.episodic.record(episode);
        }
    }

    // 6. LEARN — extract lessons from error→correction→success patterns
    maybe_extract_lessons(state, session_id).await;

    // Auto-set session label from first user message if not yet set
    if let Some(session) = state.sessions.get(session_id).await {
        if session.name.is_none() && !user_text.is_empty() {
            let label: String = user_text.chars().take(60).collect();
            let label = label
                .split('\n')
                .next()
                .unwrap_or(&label)
                .trim()
                .to_string();
            state.sessions.set_name(session_id, &label).await;
        }
    }

    // Clear reply context — this streaming session is done
    {
        let mut ctx = state.reply_context.lock().await;
        *ctx = None;
    }

    // Clear stream tx
    {
        let mut stx = state.stream_tx.lock().await;
        *stx = None;
    }

    Ok(())
}

/// Execute a tool call using shared state — dispatches to builtins, memory/goal tools, or plugins.
async fn execute_tool_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    debug!(tool = %call.tool_name, "executing tool");

    // Memory and goal tools need locks on shared state
    match call.tool_name.as_str() {
        "memory_search" => return exec_memory_search_shared(state, call).await,
        "memory_store" => return exec_memory_store_shared(state, call).await,
        "memory_delete" => return exec_memory_delete_shared(state, call).await,
        "memory_list" => return exec_memory_list_shared(state, call).await,
        "goal_create" => return exec_goal_create_shared(state, call).await,
        "goal_list" => return exec_goal_list_shared(state, call).await,
        "goal_complete_step" => return exec_goal_complete_step_shared(state, call).await,
        "goal_update_status" => return exec_goal_update_status_shared(state, call).await,
        "llm_generate" => return exec_llm_generate_shared(state, call).await,
        "web_search" => return exec_web_search_shared(state, call).await,
        "mesh_peers" => return exec_mesh_peers_shared(state, call).await,
        "mesh_delegate" => return exec_mesh_delegate_shared(state, call).await,
        "mesh_status" => return exec_mesh_status_shared(state, call).await,
        "channel_send_file" => return exec_channel_send_file(state, call).await,
        "sub_agent_spawn" => return exec_sub_agent_spawn(state, call).await,
        "sub_agent_wait" => return exec_sub_agent_wait(state, call).await,
        "sub_agent_status" => return exec_sub_agent_status(state, call).await,
        "cron_schedule" => return exec_cron_schedule(state, call).await,
        "cron_list" => return exec_cron_list(state, call).await,
        "cron_cancel" => return exec_cron_cancel(state, call).await,
        _ => {}
    }

    // Builtin tools (shell, file ops) — no lock needed
    if state.tools.has_tool(&call.tool_name) {
        match state.tools.execute(call).await {
            Ok(result) => return result,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!("Error: {}", e),
                    is_error: true,
                    data: None,
                };
            }
        }
    }

    // Device tools — browser_*, android_*, ios_*
    if DeviceTools::has_tool(&call.tool_name) {
        match state.device_tools.execute(call).await {
            Ok(result) => return result,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!("Device error: {}", e),
                    is_error: true,
                    data: None,
                };
            }
        }
    }

    // Plugin tools — "plugin_name.tool_name" (dot-separated)
    if call.tool_name.contains('.') {
        match state.plugins.execute(call).await {
            Ok(result) => return result,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!("Plugin error: {}", e),
                    is_error: true,
                    data: None,
                };
            }
        }
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!("Tool not found: {}", call.tool_name),
        is_error: true,
        data: None,
    }
}

async fn exec_llm_generate_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let prompt = match call.arguments["prompt"].as_str() {
        Some(p) => p,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'prompt' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let max_tokens = call.arguments["max_tokens"].as_u64().unwrap_or(2048) as u32;

    let request = LlmRequest {
        model: state.config.agent.model.clone(),
        messages: vec![claw_core::Message {
            id: uuid::Uuid::new_v4(),
            session_id: uuid::Uuid::nil(),
            role: claw_core::Role::User,
            content: vec![claw_core::MessageContent::Text {
                text: prompt.to_string(),
            }],
            timestamp: chrono::Utc::now(),
            tool_calls: vec![],
            metadata: serde_json::Map::new(),
        }],
        tools: vec![],
        system: Some("You are a helpful assistant. Respond concisely and directly.".into()),
        max_tokens,
        temperature: state.config.agent.temperature,
        thinking_level: None,
        stream: false,
    };

    match state
        .llm
        .complete(&request, state.config.agent.fallback_model.as_deref())
        .await
    {
        Ok(response) => {
            let text = response
                .message
                .content
                .iter()
                .filter_map(|c| match c {
                    claw_core::MessageContent::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            ToolResult {
                tool_call_id: call.id.clone(),
                content: text,
                is_error: false,
                data: None,
            }
        }
        Err(e) => ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("LLM generation failed: {}", e),
            is_error: true,
            data: None,
        },
    }
}

/// Execute a web search using the Brave Search API.
async fn exec_web_search_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let query = match call.arguments["query"].as_str() {
        Some(q) => q,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'query' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let count = call.arguments["count"].as_u64().unwrap_or(5).min(20) as u32;

    let api_key = match &state.config.services.brave_api_key {
        Some(key) if !key.is_empty() => key.clone(),
        _ => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Web search is not configured. To enable it:\n\
                    1. Get a free API key at https://api.search.brave.com/\n\
                    2. Add to your config: claw set services.brave_api_key YOUR_KEY\n\
                    3. Or run: claw setup"
                    .into(),
                is_error: true,
                data: None,
            };
        }
    };

    info!(query = query, count = count, "executing web search");

    let client = reqwest::Client::new();
    let resp = match client
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("Accept", "application/json")
        .header("X-Subscription-Token", &api_key)
        .query(&[("q", query), ("count", &count.to_string())])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Web search request failed: {}", e),
                is_error: true,
                data: None,
            };
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("Brave Search API error ({}): {}", status, body),
            is_error: true,
            data: None,
        };
    }

    let data: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Failed to parse search results: {}", e),
                is_error: true,
                data: None,
            };
        }
    };

    // Extract web results
    let mut results = Vec::new();
    if let Some(web_results) = data["web"]
        .as_object()
        .and_then(|w| w["results"].as_array())
    {
        for (i, result) in web_results.iter().enumerate() {
            let title = result["title"].as_str().unwrap_or("Untitled");
            let url = result["url"].as_str().unwrap_or("");
            let description = result["description"].as_str().unwrap_or("");
            results.push(format!(
                "{}. {}\n   {}\n   {}",
                i + 1,
                title,
                url,
                description
            ));
        }
    }

    if results.is_empty() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("No results found for: {}", query),
            is_error: false,
            data: None,
        };
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!(
            "Search results for '{}':\n\n{}",
            query,
            results.join("\n\n")
        ),
        is_error: false,
        data: None,
    }
}

// ─── Mesh tool implementations ──────────────────────────────────────────────

/// List connected mesh peers and their capabilities.
async fn exec_mesh_peers_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let capability_filter = call.arguments.get("capability").and_then(|v| v.as_str());

    let mesh = state.mesh.lock().await;
    if !mesh.is_running() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content:
                "Mesh networking is not running. Enable it in your config: [mesh] enabled = true"
                    .into(),
            is_error: true,
            data: None,
        };
    }

    let peers: Vec<_> = mesh
        .peer_list()
        .into_iter()
        .filter(|p| {
            if let Some(cap) = capability_filter {
                p.capabilities.iter().any(|c| c == cap)
            } else {
                true
            }
        })
        .collect();

    if peers.is_empty() {
        let msg = if let Some(cap) = capability_filter {
            format!(
                "No peers found with capability '{}'. {} total peers connected.",
                cap,
                mesh.peer_count()
            )
        } else {
            "No peers connected to the mesh.".to_string()
        };
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: msg,
            is_error: false,
            data: None,
        };
    }

    let mut lines = vec![format!("Connected peers ({}):", peers.len())];
    for p in &peers {
        lines.push(format!(
            "  • {} ({}) — capabilities: [{}], os: {}",
            p.hostname,
            &p.peer_id[..8.min(p.peer_id.len())],
            p.capabilities.join(", "),
            p.os,
        ));
    }

    let peer_data: Vec<serde_json::Value> = peers
        .iter()
        .map(|p| {
            serde_json::json!({
                "peer_id": p.peer_id,
                "hostname": p.hostname,
                "capabilities": p.capabilities,
                "os": p.os,
            })
        })
        .collect();

    ToolResult {
        tool_call_id: call.id.clone(),
        content: lines.join("\n"),
        is_error: false,
        data: Some(serde_json::json!({ "peers": peer_data })),
    }
}

/// Delegate a task to a mesh peer and await the result.
async fn exec_mesh_delegate_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let task_desc = match call.arguments.get("task").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'task' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let explicit_peer = call.arguments.get("peer_id").and_then(|v| v.as_str());
    let capability = call.arguments.get("capability").and_then(|v| v.as_str());
    let priority = call
        .arguments
        .get("priority")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as u8;
    let timeout_secs = call
        .arguments
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(120);

    // Resolve target peer
    let (target_peer_id, target_hostname) = {
        let mesh = state.mesh.lock().await;
        if !mesh.is_running() {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Mesh networking is not running.".into(),
                is_error: true,
                data: None,
            };
        }

        if let Some(pid) = explicit_peer {
            // Verify the peer exists
            match mesh.peers().get(pid) {
                Some(p) => (pid.to_string(), p.hostname.clone()),
                None => {
                    return ToolResult {
                        tool_call_id: call.id.clone(),
                        content: format!(
                            "Peer '{}' not found in mesh. Use mesh_peers to see available peers.",
                            pid
                        ),
                        is_error: true,
                        data: None,
                    };
                }
            }
        } else if let Some(cap) = capability {
            match mesh.find_best_peer_for_capability(cap) {
                Some(p) => (p.peer_id.clone(), p.hostname.clone()),
                None => {
                    return ToolResult {
                        tool_call_id: call.id.clone(),
                        content: format!(
                            "No peer with capability '{}' found. Available peers: {}",
                            cap,
                            mesh.peer_list()
                                .iter()
                                .map(|p| format!("{} [{}]", p.hostname, p.capabilities.join(",")))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        is_error: true,
                        data: None,
                    };
                }
            }
        } else {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content:
                    "Error: must provide either 'peer_id' or 'capability' to select a target peer."
                        .into(),
                is_error: true,
                data: None,
            };
        }
    };

    // Build the task assignment
    let our_peer_id = {
        let mesh = state.mesh.lock().await;
        mesh.peer_id().to_string()
    };

    let mut task = claw_mesh::TaskAssignment::new(&our_peer_id, &target_peer_id, &task_desc)
        .with_priority(priority);

    if let Some(cap) = capability {
        task = task.with_capability(cap);
    }

    let task_id = task.task_id;

    info!(
        task_id = %task_id,
        target_peer = %target_peer_id,
        target_host = %target_hostname,
        task = %task_desc,
        "delegating task to mesh peer"
    );

    // Register a oneshot channel to await the result
    let (result_tx, result_rx) = oneshot::channel::<MeshTaskResult>();
    {
        state
            .pending_mesh_tasks
            .lock()
            .await
            .insert(task_id, result_tx);
    }

    // Send the task via mesh
    let msg = MeshMessage::TaskAssign(task);
    {
        let mesh = state.mesh.lock().await;
        if let Err(e) = mesh.send_to(&target_peer_id, &msg).await {
            // Clean up pending task
            state.pending_mesh_tasks.lock().await.remove(&task_id);
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Failed to send task to peer: {}", e),
                is_error: true,
                data: None,
            };
        }
    }

    // Await the result with timeout
    match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), result_rx).await {
        Ok(Ok(result)) => {
            info!(
                task_id = %task_id,
                peer = %result.peer_id,
                success = result.success,
                "received delegated task result"
            );
            ToolResult {
                tool_call_id: call.id.clone(),
                content: format!(
                    "Task delegated to {} ({}) — {}\n\nResult:\n{}",
                    target_hostname,
                    &target_peer_id[..8.min(target_peer_id.len())],
                    if result.success { "SUCCESS" } else { "FAILED" },
                    result.result,
                ),
                is_error: !result.success,
                data: Some(serde_json::json!({
                    "task_id": task_id.to_string(),
                    "peer_id": result.peer_id,
                    "success": result.success,
                })),
            }
        }
        Ok(Err(_)) => {
            // Channel dropped — peer disconnected or runtime shutting down
            state.pending_mesh_tasks.lock().await.remove(&task_id);
            ToolResult {
                tool_call_id: call.id.clone(),
                content: format!(
                    "Task {} was cancelled — peer may have disconnected.",
                    task_id
                ),
                is_error: true,
                data: None,
            }
        }
        Err(_) => {
            // Timeout
            state.pending_mesh_tasks.lock().await.remove(&task_id);
            ToolResult {
                tool_call_id: call.id.clone(),
                content: format!(
                    "Task {} timed out after {}s waiting for response from {} ({}).",
                    task_id,
                    timeout_secs,
                    target_hostname,
                    &target_peer_id[..8.min(target_peer_id.len())]
                ),
                is_error: true,
                data: None,
            }
        }
    }
}

/// Get the status of the mesh network.
async fn exec_mesh_status_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let mesh = state.mesh.lock().await;
    let running = mesh.is_running();
    let peer_id = mesh.peer_id().to_string();
    let peer_count = mesh.peer_count();
    drop(mesh);

    let status = if running {
        format!(
            "Mesh network: RUNNING\n\
             Our peer ID: {}\n\
             Connected peers: {}\n\
             Listen address: {}\n\
             mDNS discovery: {}\n\
             Our capabilities: [{}]",
            &peer_id[..12.min(peer_id.len())],
            peer_count,
            state.config.mesh.listen,
            if state.config.mesh.mdns {
                "enabled"
            } else {
                "disabled"
            },
            state.config.mesh.capabilities.join(", "),
        )
    } else {
        "Mesh network: NOT RUNNING\nEnable it in config: [mesh] enabled = true".to_string()
    };

    ToolResult {
        tool_call_id: call.id.clone(),
        content: status,
        is_error: false,
        data: Some(serde_json::json!({
            "running": running,
            "peer_id": peer_id,
            "peer_count": peer_count,
            "capabilities": state.config.mesh.capabilities,
        })),
    }
}

async fn exec_memory_search_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let query = call.arguments["query"].as_str().unwrap_or("");
    let mem_type = call.arguments["type"].as_str().unwrap_or("all");

    // Generate query embedding for vector search
    let query_embedding = if let Some(ref embedder) = state.embedder {
        match embedder.embed(&[query]).await {
            Ok(vecs) if !vecs.is_empty() => Some(vecs.into_iter().next().unwrap()),
            _ => None,
        }
    } else {
        None
    };

    let mem = state.memory.lock().await;
    let mut results = Vec::new();

    if mem_type == "episodic" || mem_type == "all" {
        let episodes = mem.episodic.search(query);
        for ep in episodes.iter().take(10) {
            results.push(format!(
                "[Episode {}] {}{}",
                ep.created_at.format("%Y-%m-%d"),
                ep.summary,
                ep.outcome
                    .as_ref()
                    .map(|o| format!(" → {}", o))
                    .unwrap_or_default()
            ));
        }
    }

    if mem_type == "semantic" || mem_type == "all" {
        // Combine vector + keyword search, dedup by category:key
        let mut seen = std::collections::HashSet::new();

        // Vector search first (highest quality)
        if let Some(ref qemb) = query_embedding {
            for (fact, score) in mem.semantic.vector_search(qemb, 15) {
                let fk = format!("{}:{}", fact.category, fact.key);
                if seen.insert(fk) {
                    results.push(format!(
                        "[Fact: {}/{}] {} (relevance: {:.0}%)",
                        fact.category,
                        fact.key,
                        fact.value,
                        score * 100.0
                    ));
                }
            }
        }

        // Word-level keyword search (catches things without embeddings)
        for fact in mem.semantic.search(query).iter().take(15) {
            let fk = format!("{}:{}", fact.category, fact.key);
            if seen.insert(fk) {
                results.push(format!(
                    "[Fact: {}/{}] {} (confidence: {:.0}%)",
                    fact.category,
                    fact.key,
                    fact.value,
                    fact.confidence * 100.0
                ));
            }
        }

        // Also search with extracted keywords for broader matching
        let keywords = extract_search_keywords(query);
        if keywords != query.to_lowercase() {
            for fact in mem.semantic.search(&keywords).iter().take(5) {
                let fk = format!("{}:{}", fact.category, fact.key);
                if seen.insert(fk) {
                    results.push(format!(
                        "[Fact: {}/{}] {}",
                        fact.category, fact.key, fact.value
                    ));
                }
            }
        }
    }

    let content = if results.is_empty() {
        format!(
            "No relevant memories found for query: \"{}\". Try memory_list to see all stored facts.",
            query
        )
    } else {
        results.join("\n")
    };

    ToolResult {
        tool_call_id: call.id.clone(),
        content,
        is_error: false,
        data: None,
    }
}

async fn exec_memory_store_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let category = call.arguments["category"].as_str().unwrap_or("general");
    let key = call.arguments["key"].as_str().unwrap_or("unknown");
    let value = call.arguments["value"].as_str().unwrap_or("");

    // Generate embedding if an embedder is configured
    let embedding = if let Some(ref embedder) = state.embedder {
        let text_for_embedding = format!("{} {} {}", category, key, value);
        match embedder.embed(&[&text_for_embedding]).await {
            Ok(vecs) if !vecs.is_empty() => Some(vecs.into_iter().next().unwrap()),
            Ok(_) => None,
            Err(e) => {
                warn!(error = %e, "failed to generate embedding for fact, storing without vector");
                None
            }
        }
    } else {
        None
    };

    let mut mem = state.memory.lock().await;

    let fact = claw_memory::semantic::Fact {
        id: Uuid::new_v4(),
        category: category.to_string(),
        key: key.to_string(),
        value: value.to_string(),
        confidence: 1.0,
        source: Some("agent".to_string()),
        embedding: embedding.clone(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    mem.semantic.upsert(fact);

    if let Err(e) = mem.persist_fact_with_embedding(category, key, value, embedding.as_deref()) {
        warn!(error = %e, "failed to persist fact to SQLite");
    }
    drop(mem); // Release memory lock before mesh operations

    // Broadcast fact to mesh peers for sync
    {
        let mesh = state.mesh.lock().await;
        if mesh.is_running() && mesh.peer_count() > 0 {
            let sync_msg = MeshMessage::SyncDelta {
                peer_id: mesh.peer_id().to_string(),
                delta_type: "fact".to_string(),
                data: serde_json::json!({
                    "category": category,
                    "key": key,
                    "value": value,
                    "confidence": 1.0,
                }),
            };
            if let Err(e) = mesh.broadcast(&sync_msg).await {
                debug!(error = %e, "failed to broadcast fact to mesh peers");
            } else {
                debug!(
                    category = category,
                    key = key,
                    "broadcast fact to mesh peers"
                );
            }
        }
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!("Stored fact: {}/{} = {}", category, key, value),
        is_error: false,
        data: None,
    }
}

async fn exec_memory_delete_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let category = call.arguments["category"].as_str().unwrap_or("");
    let key = call.arguments.get("key").and_then(|v| v.as_str());

    if category.is_empty() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: "Error: 'category' is required".to_string(),
            is_error: true,
            data: None,
        };
    }

    let mut mem = state.memory.lock().await;

    let result_msg = if let Some(key) = key {
        // Delete a specific fact
        let removed_mem = mem.semantic.remove(category, key);
        let removed_db = mem.delete_fact(category, key).unwrap_or(false);
        if removed_mem || removed_db {
            format!("Deleted fact: {}/{}", category, key)
        } else {
            format!("Fact not found: {}/{}", category, key)
        }
    } else {
        // Delete entire category
        let count_mem = mem.semantic.remove_category(category);
        let count_db = mem.delete_facts_by_category(category).unwrap_or(0);
        let count = count_mem.max(count_db);
        if count > 0 {
            format!("Deleted {} fact(s) from category '{}'", count, category)
        } else {
            format!("Category '{}' not found or already empty", category)
        }
    };

    drop(mem);

    ToolResult {
        tool_call_id: call.id.clone(),
        content: result_msg,
        is_error: false,
        data: None,
    }
}

async fn exec_memory_list_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let filter_category = call.arguments.get("category").and_then(|v| v.as_str());

    let mem = state.memory.lock().await;
    let mut lines = Vec::new();

    if let Some(cat) = filter_category {
        // List facts in a specific category
        let facts = mem.semantic.category(cat);
        if facts.is_empty() {
            lines.push(format!("Category '{}': (empty)", cat));
        } else {
            lines.push(format!("Category '{}' ({} facts):", cat, facts.len()));
            for fact in facts {
                lines.push(format!(
                    "  - {}: {} (confidence: {:.0}%, updated: {})",
                    fact.key,
                    fact.value,
                    fact.confidence * 100.0,
                    fact.updated_at.format("%Y-%m-%d %H:%M")
                ));
            }
        }
    } else {
        // List all categories with their facts
        let mut categories: Vec<&str> = mem.semantic.categories();
        categories.sort();
        if categories.is_empty() {
            lines.push("Memory is empty — no facts stored.".to_string());
        } else {
            let total = mem.semantic.count();
            lines.push(format!(
                "Total: {} facts across {} categories\n",
                total,
                categories.len()
            ));
            for cat in categories {
                let facts = mem.semantic.category(cat);
                lines.push(format!("📁 {} ({}):", cat, facts.len()));
                for fact in facts.iter().take(20) {
                    lines.push(format!(
                        "  - {}: {}",
                        fact.key,
                        if fact.value.len() > 120 {
                            format!("{}…", &fact.value[..120])
                        } else {
                            fact.value.clone()
                        }
                    ));
                }
                if facts.len() > 20 {
                    lines.push(format!("  ... and {} more", facts.len() - 20));
                }
            }
        }
    }

    drop(mem);

    ToolResult {
        tool_call_id: call.id.clone(),
        content: lines.join("\n"),
        is_error: false,
        data: None,
    }
}

async fn exec_goal_create_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let description = call.arguments["description"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let priority = call.arguments["priority"].as_u64().unwrap_or(5) as u8;
    let steps: Vec<String> = call.arguments["steps"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let mut planner = state.planner.lock().await;
    let goal = planner.create_goal(description.clone(), priority);
    let goal_id = goal.id;
    if !steps.is_empty() {
        planner.set_plan(goal_id, steps.clone());
    }

    // Persist goal to SQLite
    {
        let mem = state.memory.lock().await;
        if let Err(e) = mem.persist_goal(&goal_id, &description, "active", priority, 0.0, None) {
            warn!(error = %e, "failed to persist goal to SQLite");
        }
        // Persist steps
        if let Some(goal) = planner.get(goal_id) {
            for step in &goal.steps {
                if let Err(e) = mem.persist_goal_step(
                    &step.id,
                    &goal_id,
                    &step.description,
                    &format!("{:?}", step.status).to_lowercase(),
                    None,
                ) {
                    warn!(error = %e, "failed to persist goal step to SQLite");
                }
            }
        }
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!(
            "Created goal '{}' (id: {}, priority: {}, {} steps)",
            description,
            goal_id,
            priority,
            steps.len()
        ),
        is_error: false,
        data: None,
    }
}

async fn exec_goal_list_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let planner = state.planner.lock().await;
    let goals = planner.all();
    if goals.is_empty() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: "No goals.".to_string(),
            is_error: false,
            data: None,
        };
    }

    let mut lines = Vec::new();
    for goal in goals {
        let status_tag = match goal.status {
            claw_autonomy::planner::GoalStatus::Completed => " ✅ COMPLETED",
            claw_autonomy::planner::GoalStatus::Cancelled => " ❌ CANCELLED",
            _ => "",
        };
        lines.push(format!(
            "• [{}] {} (priority: {}, progress: {:.0}%{})",
            goal.id,
            goal.description,
            goal.priority,
            goal.progress * 100.0,
            status_tag
        ));
        for step in &goal.steps {
            let icon = match step.status {
                claw_autonomy::planner::StepStatus::Completed => "✅",
                claw_autonomy::planner::StepStatus::InProgress => "🔄",
                claw_autonomy::planner::StepStatus::Failed => "❌",
                _ => "⬜",
            };
            lines.push(format!(
                "    {} [step:{}] {}",
                icon, step.id, step.description
            ));
        }
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: lines.join("\n"),
        is_error: false,
        data: None,
    }
}

async fn exec_goal_complete_step_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let goal_id_str = call.arguments["goal_id"].as_str().unwrap_or("");
    let step_id_str = call.arguments["step_id"].as_str().unwrap_or("");
    let result = call.arguments["result"].as_str().unwrap_or("").to_string();

    let goal_id = match goal_id_str.parse::<Uuid>() {
        Ok(id) => id,
        Err(_) => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Invalid goal_id: {}", goal_id_str),
                is_error: true,
                data: None,
            };
        }
    };
    let step_id = match step_id_str.parse::<Uuid>() {
        Ok(id) => id,
        Err(_) => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Invalid step_id: {}", step_id_str),
                is_error: true,
                data: None,
            };
        }
    };

    let mut planner = state.planner.lock().await;
    planner.complete_step(goal_id, step_id, result.clone());

    // Get updated progress
    let (progress, status) = planner
        .get(goal_id)
        .map(|g| (g.progress, format!("{:?}", g.status)))
        .unwrap_or((0.0, "unknown".into()));

    // Persist updated goal to SQLite
    {
        let goal_desc = planner
            .get(goal_id)
            .map(|g| g.description.clone())
            .unwrap_or_default();
        let goal_priority = planner.get(goal_id).map(|g| g.priority).unwrap_or(5);
        let step_desc = planner
            .get(goal_id)
            .and_then(|g| g.steps.iter().find(|s| s.id == step_id))
            .map(|s| s.description.clone())
            .unwrap_or_default();
        let mem = state.memory.lock().await;
        let _ = mem.persist_goal(
            &goal_id,
            &goal_desc,
            &status.to_lowercase(),
            goal_priority,
            progress,
            None,
        );
        let _ = mem.persist_goal_step(&step_id, &goal_id, &step_desc, "completed", Some(&result));
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!(
            "Step completed. Goal progress: {:.0}%, status: {}",
            progress * 100.0,
            status
        ),
        is_error: false,
        data: None,
    }
}

async fn exec_goal_update_status_shared(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let goal_id_str = call.arguments["goal_id"].as_str().unwrap_or("");
    let status_str = call.arguments["status"].as_str().unwrap_or("active");
    let reason = call.arguments["reason"].as_str().unwrap_or("").to_string();

    let goal_id = match goal_id_str.parse::<Uuid>() {
        Ok(id) => id,
        Err(_) => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Invalid goal_id: {}", goal_id_str),
                is_error: true,
                data: None,
            };
        }
    };

    let mut planner = state.planner.lock().await;

    // Find the goal and update its status
    let updated = if let Some(goal) = planner.all_mut().iter_mut().find(|g| g.id == goal_id) {
        let new_status = match status_str {
            "completed" => claw_autonomy::planner::GoalStatus::Completed,
            "failed" => claw_autonomy::planner::GoalStatus::Failed,
            "paused" => claw_autonomy::planner::GoalStatus::Paused,
            "cancelled" => claw_autonomy::planner::GoalStatus::Cancelled,
            "active" => claw_autonomy::planner::GoalStatus::Active,
            _ => {
                return ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!(
                        "Invalid status: {}. Use: active, completed, failed, paused, cancelled",
                        status_str
                    ),
                    is_error: true,
                    data: None,
                };
            }
        };
        goal.status = new_status;
        if !reason.is_empty() {
            goal.retrospective = Some(reason.clone());
        }
        goal.updated_at = chrono::Utc::now();
        true
    } else {
        false
    };

    if !updated {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("Goal not found: {}", goal_id_str),
            is_error: true,
            data: None,
        };
    }

    // Persist updated goal to SQLite — read current values so we don't clobber description/priority/progress
    {
        let goal_desc = planner
            .get(goal_id)
            .map(|g| g.description.clone())
            .unwrap_or_default();
        let goal_priority = planner.get(goal_id).map(|g| g.priority).unwrap_or(5);
        let goal_progress = planner.get(goal_id).map(|g| g.progress).unwrap_or(0.0);
        let mem = state.memory.lock().await;
        let _ = mem.persist_goal(
            &goal_id,
            &goal_desc,
            status_str,
            goal_priority,
            goal_progress,
            None,
        );
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!(
            "Goal {} status updated to '{}'{}",
            goal_id,
            status_str,
            if reason.is_empty() {
                String::new()
            } else {
                format!(": {}", reason)
            }
        ),
        is_error: false,
        data: None,
    }
}

/// Execute channel_send_file — send a file through the active chat channel.
async fn exec_channel_send_file(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    use claw_channels::adapter::{Attachment, OutgoingMessage};

    let file_path_raw = match call.arguments["file_path"].as_str() {
        Some(p) => p,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'file_path' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    // Expand ~ to home directory
    let file_path_str = if file_path_raw == "~" || file_path_raw.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            format!("{}{}", home.display(), &file_path_raw[1..])
        } else {
            file_path_raw.to_string()
        }
    } else {
        file_path_raw.to_string()
    };

    let file_path = std::path::Path::new(&file_path_str);

    // Verify file exists
    if !file_path.exists() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("Error: file not found: {}", file_path_str),
            is_error: true,
            data: None,
        };
    }

    // Read the reply context to know which channel/target to send to
    let (channel_id, target) = {
        let ctx = state.reply_context.lock().await;
        match ctx.as_ref() {
            Some((cid, tgt)) => (cid.clone(), tgt.clone()),
            None => {
                return ToolResult {
                    tool_call_id: call.id.clone(),
                    content: "Error: no active channel context — channel_send_file can only be used when responding to a channel message".into(),
                    is_error: true,
                    data: None,
                };
            }
        }
    };

    // Determine MIME type from extension
    let filename = file_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let lower = filename.to_lowercase();
    let media_type = if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else if lower.ends_with(".mp3") {
        "audio/mpeg"
    } else if lower.ends_with(".m4a") {
        "audio/mp4"
    } else if lower.ends_with(".ogg") {
        "audio/ogg"
    } else if lower.ends_with(".wav") {
        "audio/wav"
    } else if lower.ends_with(".aac") {
        "audio/aac"
    } else if lower.ends_with(".flac") {
        "audio/flac"
    } else if lower.ends_with(".aiff") {
        "audio/aiff"
    } else if lower.ends_with(".mp4") {
        "video/mp4"
    } else if lower.ends_with(".mov") {
        "video/quicktime"
    } else if lower.ends_with(".avi") {
        "video/x-msvideo"
    } else if lower.ends_with(".mkv") {
        "video/x-matroska"
    } else if lower.ends_with(".webm") {
        "video/webm"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".zip") {
        "application/zip"
    } else if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".csv") {
        "text/csv"
    } else if lower.ends_with(".txt") || lower.ends_with(".log") || lower.ends_with(".md") {
        "text/plain"
    } else {
        "application/octet-stream"
    };

    let caption = call.arguments["caption"].as_str().unwrap_or("").to_string();

    // Send through the channel with the file as an attachment
    let channels = state.channels.lock().await;
    for channel in channels.iter() {
        if channel.id() == &channel_id {
            let msg = OutgoingMessage {
                channel: channel_id.clone(),
                target: target.clone(),
                text: caption.clone(),
                attachments: vec![Attachment {
                    filename: filename.clone(),
                    media_type: media_type.to_string(),
                    data: file_path_str.clone(), // Pass the file path — adapters read it
                }],
                reply_to: None,
            };

            match channel.send(msg).await {
                Ok(()) => {
                    info!(file = %file_path_str, channel = %channel_id, "channel_send_file: file sent successfully");
                    return ToolResult {
                        tool_call_id: call.id.clone(),
                        content: format!(
                            "File sent successfully: {} ({}, {})",
                            filename, media_type, channel_id
                        ),
                        is_error: false,
                        data: None,
                    };
                }
                Err(e) => {
                    warn!(error = %e, file = %file_path_str, "channel_send_file: failed to send");
                    return ToolResult {
                        tool_call_id: call.id.clone(),
                        content: format!("Error sending file: {}", e),
                        is_error: true,
                        data: None,
                    };
                }
            }
        }
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!("Error: channel '{}' not found", channel_id),
        is_error: true,
        data: None,
    }
}

/// Check if a tool can safely be executed in parallel with other tools.
/// Tools that don't mutate shared state or that operate on independent resources are parallel-safe.
fn is_parallel_safe(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "http_fetch"
            | "web_search"
            | "file_read"
            | "file_list"
            | "file_find"
            | "file_grep"
            | "memory_search"
            | "memory_list"
            | "mesh_peers"
            | "mesh_delegate"
            | "mesh_status"
            | "goal_list"
            | "sub_agent_spawn"
            | "sub_agent_status"
            | "process_list"
            | "process_output"
            | "terminal_view"
    )
}

// ─── Sub-Agent Tool Implementations ─────────────────────────────────────────

/// Role-specific system prompt prefixes for sub-agents.
fn sub_agent_system_prompt(role: &str) -> String {
    let role_instruction = match role {
        "planner" => {
            "You are a planning agent. Your job is to analyze the task, break it down into clear steps, identify required files and dependencies, and create a detailed project plan. Output a structured plan with specific file paths, technologies, and implementation order. Do NOT write code — only plan."
        }
        "coder" | "developer" => {
            "You are a coding agent. Your job is to write production-quality code based on the task description. When research findings are provided from a preceding agent, use them thoroughly — match the structure, content, styling, and details described. Write complete, well-structured files with proper imports, error handling, and documentation. Use your tools to create files and test that they compile/run correctly. After writing code, run the build and fix any errors before finishing."
        }
        "reviewer" => {
            "You are a code review agent. Your job is to review the code that was written, check for bugs, security issues, missing error handling, and style problems. Run tests and linters if available. Report issues clearly with file paths and line numbers. Suggest specific fixes."
        }
        "tester" | "qa" => {
            "You are a testing agent. Your job is to write and run tests for the code. Create unit tests, integration tests, and end-to-end tests as appropriate. Verify that the application works correctly. Report any failures with details."
        }
        "researcher" => {
            "You are a research agent. Your job is to gather information needed for the task. When given URLs, ALWAYS fetch them with http_fetch first — read the actual content, structure, and details before summarizing. Search the web, read documentation, find examples, and compile your findings into a clear, structured summary. Focus on finding practical, actionable information. For website rebuilds: extract page structure, navigation items, section headings, key copy, feature lists, and design notes."
        }
        "devops" | "deployer" => {
            "You are a DevOps agent. Your job is to set up build systems, CI/CD, deployment configurations, Docker files, and infrastructure. Ensure the project can be built, tested, and deployed reliably."
        }
        "debugger" | "fixer" => {
            "You are a debugging agent. Your job is to find and fix errors in the code. Read error messages carefully, trace the root cause, and apply fixes. Run the code again to verify the fix works."
        }
        _ => "You are a specialized agent. Execute the assigned task thoroughly using your tools.",
    };

    format!(
        "You are a Claw 🦞 sub-agent with the role: {}.\n\n{}\n\n\
         Work autonomously — complete the task using your tools without asking for clarification.\n\
         When done, output a clear summary of what you accomplished and any important findings.",
        role, role_instruction
    )
}

/// Spawn a sub-agent to work on a task concurrently.
async fn exec_sub_agent_spawn(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let role = match call.arguments.get("role").and_then(|v| v.as_str()) {
        Some(r) => r.to_string(),
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'role' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let task = match call.arguments.get("task").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'task' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let context_summary = call
        .arguments
        .get("context_summary")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let depends_on: Vec<Uuid> = call
        .arguments
        .get("depends_on")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().and_then(|s| s.parse::<Uuid>().ok()))
                .collect()
        })
        .unwrap_or_default();

    let _model_override = call
        .arguments
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Optional goal/step linking — auto-complete goal step when sub-agent finishes
    let goal_id: Option<Uuid> = call
        .arguments
        .get("goal_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<Uuid>().ok());

    let step_id: Option<Uuid> = call
        .arguments
        .get("step_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<Uuid>().ok());

    let task_id = Uuid::new_v4();
    let parent_session_id = Uuid::new_v4();

    // Determine initial status based on dependencies
    let initial_status = if depends_on.is_empty() {
        SubTaskStatus::Pending
    } else {
        SubTaskStatus::WaitingForDeps
    };

    // If linked to a goal step, mark it as in-progress in the planner
    if let (Some(gid), Some(sid)) = (goal_id, step_id) {
        let mut planner = state.planner.lock().await;
        planner.assign_to_sub_agent(gid, sid, task_id, Some(role.clone()));
        info!(task_id = %task_id, goal_id = %gid, step_id = %sid, "linked sub-agent to goal step");
    }

    // Register the sub-task
    {
        let sub_task_state = SubTaskState {
            task_id,
            role: role.clone(),
            task_description: task.clone(),
            status: initial_status,
            result: None,
            error: None,
            parent_session_id,
            depends_on: depends_on.clone(),
            created_at: std::time::Instant::now(),
            goal_id,
            step_id,
        };
        state
            .pending_sub_tasks
            .lock()
            .await
            .insert(task_id, sub_task_state);
    }

    // Spawn the sub-agent task (uses boxed future to break async type cycle)
    let s = state.clone();
    tokio::spawn(run_sub_agent_task(
        s,
        task_id,
        role.clone(),
        task.clone(),
        context_summary,
        depends_on.clone(),
    ));

    info!(
        task_id = %task_id,
        role = %role,
        deps = ?depends_on,
        "spawned sub-agent"
    );

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!(
            "Sub-agent spawned successfully.\n\
             Task ID: {}\n\
             Role: {}\n\
             Status: {}\n\
             Dependencies: {}\n\n\
             Use sub_agent_wait with this task_id to collect the result when ready.",
            task_id,
            role,
            if depends_on.is_empty() {
                "running"
            } else {
                "waiting for dependencies"
            },
            if depends_on.is_empty() {
                "none".to_string()
            } else {
                depends_on
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ),
        is_error: false,
        data: Some(serde_json::json!({
            "task_id": task_id.to_string(),
            "role": role,
            "status": if depends_on.is_empty() { "running" } else { "waiting_for_deps" },
        })),
    }
}

/// Internal: run the sub-agent task through a fresh agent loop.
/// Returns a boxed future to break the async type recursion cycle
/// (process_message_shared → exec_sub_agent_spawn → run_sub_agent_task → process_api_message → process_message_shared).
///
/// If a parent stream_tx is available, sub-agent events (tool calls, results, progress)
/// are forwarded to the parent stream so they render in the web UI and channel outputs.
fn run_sub_agent_task(
    state: SharedAgentState,
    task_id: Uuid,
    role: String,
    task_description: String,
    context_summary: Option<String>,
    depends_on: Vec<Uuid>,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        // Wait for dependencies if needed
        let effective_task = if !depends_on.is_empty() {
            info!(task_id = %task_id, deps = ?depends_on, "sub-agent waiting for dependencies");
            loop {
                let all_done = {
                    let tasks = state.pending_sub_tasks.lock().await;
                    depends_on.iter().all(|dep_id| {
                        tasks
                            .get(dep_id)
                            .map(|t| {
                                t.status == SubTaskStatus::Completed
                                    || t.status == SubTaskStatus::Failed
                            })
                            .unwrap_or(true)
                    })
                };
                if all_done {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }

            // Collect dependency results to provide context
            let dep_results: Vec<String> = {
                let tasks = state.pending_sub_tasks.lock().await;
                depends_on
                    .iter()
                    .filter_map(|dep_id| {
                        tasks.get(dep_id).map(|t| {
                            format!(
                                "[{} agent ({})] {}",
                                t.role,
                                if t.status == SubTaskStatus::Completed {
                                    "completed"
                                } else {
                                    "failed"
                                },
                                t.result
                                    .as_deref()
                                    .or(t.error.as_deref())
                                    .unwrap_or("no output")
                            )
                        })
                    })
                    .collect()
            };

            // Build the task message with dependency results
            let mut full_task = task_description.clone();
            if !dep_results.is_empty() {
                full_task.push_str("\n\n## Results from preceding agents:\n");
                for dr in &dep_results {
                    full_task.push_str(dr);
                    full_task.push('\n');
                }
            }
            full_task
        } else {
            task_description.clone()
        };

        // Update status to running
        {
            let mut tasks = state.pending_sub_tasks.lock().await;
            if let Some(t) = tasks.get_mut(&task_id) {
                t.status = SubTaskStatus::Running;
            }
        }

        // Create a fresh session for this sub-agent
        let session_id = state.sessions.create().await;
        let label = format!("sub-agent:{}", role);
        state.sessions.set_name(session_id, &label).await;

        // Build the task message with context
        let mut prompt = String::new();
        if let Some(ref ctx) = context_summary {
            prompt.push_str("## Context from parent agent:\n");
            prompt.push_str(ctx);
            prompt.push_str("\n\n");
        }
        prompt.push_str("## Your Task:\n");
        prompt.push_str(&effective_task);

        // Inject role-specific system prompt
        let mut sub_state = state.clone();
        let mut sub_config = sub_state.config.clone();
        sub_config.agent.system_prompt = Some(sub_agent_system_prompt(&role));
        if sub_config.agent.max_iterations < 100 {
            sub_config.agent.max_iterations = 100;
        }
        sub_state.config = sub_config;

        // Check if we have a parent stream tx to forward events to
        let parent_tx = {
            let stx = state.stream_tx.lock().await;
            stx.clone()
        };

        let (result_text, result_error) = if let Some(ref ptx) = parent_tx {
            // Use streaming path — forward sub-agent events to parent stream
            let role_tag = role.clone();

            // Send a marker so the UI/channel knows a sub-agent started
            let _ = ptx
                .send(StreamEvent::TextDelta {
                    content: format!("\n\n🤖 *Sub-agent ({}) working…*\n", role_tag),
                })
                .await;

            // Create a local stream that forwards events to the parent
            let (sub_tx, mut sub_rx) = mpsc::channel::<StreamEvent>(128);
            let ptx_fwd = ptx.clone();
            let role_fwd = role_tag.clone();
            let forwarder = tokio::spawn(async move {
                let mut sub_text = String::new();
                while let Some(event) = sub_rx.recv().await {
                    match event {
                        StreamEvent::ToolCall { name, id, args } => {
                            // Forward tool calls with sub-agent prefix in the name
                            let prefixed_name = format!("[{}] {}", role_fwd, name);
                            let _ = ptx_fwd
                                .send(StreamEvent::ToolCall {
                                    name: prefixed_name,
                                    id,
                                    args,
                                })
                                .await;
                        }
                        StreamEvent::ToolResult {
                            id,
                            content,
                            is_error,
                            data,
                        } => {
                            // Forward tool results as-is (they match by id)
                            let _ = ptx_fwd
                                .send(StreamEvent::ToolResult {
                                    id,
                                    content,
                                    is_error,
                                    data,
                                })
                                .await;
                        }
                        StreamEvent::TextDelta { content } => {
                            sub_text.push_str(&content);
                        }
                        StreamEvent::Error { message } => {
                            let _ = ptx_fwd
                                .send(StreamEvent::TextDelta {
                                    content: format!(
                                        "\n⚠️ Sub-agent ({}) error: {}\n",
                                        role_fwd, message
                                    ),
                                })
                                .await;
                        }
                        StreamEvent::Done => break,
                        _ => {} // Skip session, usage, etc.
                    }
                }
                sub_text
            });

            // Build incoming message for the streaming path
            let incoming = IncomingMessage {
                id: Uuid::new_v4().to_string(),
                channel: "sub-agent".to_string(),
                sender: format!("sub-agent:{}", role_tag),
                sender_name: Some(format!("Sub-agent ({})", role_tag)),
                group: None,
                text: Some(prompt.clone()),
                attachments: vec![],
                is_mention: false,
                is_reply_to_bot: false,
                metadata: serde_json::Value::Null,
            };

            // Clear the parent stream_tx in sub_state so nested sub-agents
            // don't double-forward (they'll get their own copy if needed)
            {
                let mut stx = sub_state.stream_tx.lock().await;
                *stx = Some(sub_tx.clone());
            }

            let stream_result = process_message_streaming_shared(
                &sub_state,
                "sub-agent",
                incoming,
                &sub_tx,
                Some(session_id),
            )
            .await;

            let _ = sub_tx.send(StreamEvent::Done).await;
            drop(sub_tx);

            // Wait for forwarder to finish and get the accumulated text
            let sub_final_text = match forwarder.await {
                Ok(text) => text,
                Err(_) => String::new(),
            };

            // Send completion marker
            let _ = ptx
                .send(StreamEvent::TextDelta {
                    content: format!("\n✅ *Sub-agent ({}) done*\n\n", role_tag),
                })
                .await;

            match stream_result {
                Ok(()) => (sub_final_text, None),
                Err(e) => (sub_final_text, Some(e.to_string())),
            }
        } else {
            // No parent stream — fall back to non-streaming API path
            let result = process_api_message(sub_state, prompt, Some(session_id.to_string())).await;
            (result.text, result.error)
        };

        // Update the sub-task state with the result
        let (is_error, goal_link) = {
            let mut tasks = state.pending_sub_tasks.lock().await;
            if let Some(t) = tasks.get_mut(&task_id) {
                if result_error.is_some() {
                    t.status = SubTaskStatus::Failed;
                    t.error = result_error.clone();
                    t.result = Some(result_text.clone());
                    (true, (t.goal_id, t.step_id))
                } else {
                    t.status = SubTaskStatus::Completed;
                    t.result = Some(result_text.clone());
                    (false, (t.goal_id, t.step_id))
                }
            } else {
                (false, (None, None))
            }
        };

        // Auto-update linked goal step if goal_id/step_id were provided
        if let (Some(gid), Some(sid)) = goal_link {
            let mut planner = state.planner.lock().await;
            if is_error {
                let err_msg = result_error.unwrap_or_else(|| "Sub-agent failed".into());
                let updated = planner.fail_sub_agent_task(task_id, err_msg.clone());
                if updated {
                    info!(task_id = %task_id, role = %role, "auto-failed linked goal step");
                    // Persist updated goal + step to SQLite
                    if let Some(goal) = planner.get(gid) {
                        let mem = state.memory.lock().await;
                        let _ = mem.persist_goal(
                            &gid,
                            &goal.description,
                            &format!("{:?}", goal.status).to_lowercase(),
                            goal.priority,
                            goal.progress,
                            None,
                        );
                        let _ = mem.persist_goal_step(&sid, &gid, "", "failed", Some(&err_msg));
                    }
                }
            } else {
                let summary = result_text.chars().take(500).collect::<String>();
                let updated = planner.complete_sub_agent_task(task_id, summary.clone());
                if updated {
                    info!(task_id = %task_id, role = %role, "auto-completed linked goal step");
                    // Persist updated goal + step to SQLite
                    if let Some(goal) = planner.get(gid) {
                        let mem = state.memory.lock().await;
                        let _ = mem.persist_goal(
                            &gid,
                            &goal.description,
                            &format!("{:?}", goal.status).to_lowercase(),
                            goal.priority,
                            goal.progress,
                            None,
                        );
                        let _ = mem.persist_goal_step(&sid, &gid, "", "completed", Some(&summary));
                    }
                }
            }
        }

        info!(task_id = %task_id, role = %role, "sub-agent task completed");
    })
}

/// Wait for one or more sub-agent tasks to complete.
async fn exec_sub_agent_wait(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let task_ids: Vec<Uuid> = call
        .arguments
        .get("task_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().and_then(|s| s.parse::<Uuid>().ok()))
                .collect()
        })
        .unwrap_or_default();

    if task_ids.is_empty() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: "Error: 'task_ids' must contain at least one task ID".into(),
            is_error: true,
            data: None,
        };
    }

    let timeout_secs = call
        .arguments
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let started = std::time::Instant::now();

    // Poll until all tasks are done
    loop {
        let all_done = {
            let tasks = state.pending_sub_tasks.lock().await;
            task_ids.iter().all(|id| {
                tasks
                    .get(id)
                    .map(|t| {
                        t.status == SubTaskStatus::Completed || t.status == SubTaskStatus::Failed
                    })
                    .unwrap_or(true)
            })
        };

        if all_done {
            break;
        }

        // Check timeout
        if timeout_secs > 0 && started.elapsed().as_secs() >= timeout_secs {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!(
                    "Timeout: waited {}s but not all sub-agent tasks completed.",
                    timeout_secs
                ),
                is_error: true,
                data: None,
            };
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    // Collect results
    let tasks = state.pending_sub_tasks.lock().await;
    let mut results = Vec::new();
    let mut result_data = Vec::new();

    for id in &task_ids {
        if let Some(t) = tasks.get(id) {
            let status_str = match t.status {
                SubTaskStatus::Completed => "completed",
                SubTaskStatus::Failed => "failed",
                _ => "unknown",
            };
            results.push(format!(
                "## {} agent [{}] — {}\n{}",
                t.role,
                id,
                status_str,
                t.result
                    .as_deref()
                    .or(t.error.as_deref())
                    .unwrap_or("no output"),
            ));
            result_data.push(serde_json::json!({
                "task_id": id.to_string(),
                "role": t.role,
                "status": status_str,
                "result": t.result,
                "error": t.error,
            }));
        } else {
            results.push(format!("## Task {} — not found", id));
        }
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: results.join("\n\n"),
        is_error: false,
        data: Some(serde_json::json!({ "tasks": result_data })),
    }
}

/// Check the status of sub-agent tasks without blocking.
async fn exec_sub_agent_status(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let task_ids: Vec<Uuid> = call
        .arguments
        .get("task_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().and_then(|s| s.parse::<Uuid>().ok()))
                .collect()
        })
        .unwrap_or_default();

    let tasks = state.pending_sub_tasks.lock().await;

    let entries: Vec<&SubTaskState> = if task_ids.is_empty() {
        tasks.values().collect()
    } else {
        task_ids.iter().filter_map(|id| tasks.get(id)).collect()
    };

    if entries.is_empty() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: "No sub-agent tasks found.".into(),
            is_error: false,
            data: None,
        };
    }

    let mut lines = vec![format!("Sub-agent tasks ({}):", entries.len())];
    let mut data = Vec::new();

    for t in &entries {
        let status_str = match t.status {
            SubTaskStatus::WaitingForDeps => "waiting_for_deps",
            SubTaskStatus::Pending => "pending",
            SubTaskStatus::Running => "running",
            SubTaskStatus::Completed => "completed",
            SubTaskStatus::Failed => "failed",
        };
        let elapsed = t.created_at.elapsed().as_secs();
        lines.push(format!(
            "  • {} ({}) — {} [{}s elapsed]{}",
            t.role,
            &t.task_id.to_string()[..8],
            status_str,
            elapsed,
            if let Some(ref r) = t.result {
                format!(" — result: {}...", &r[..r.len().min(100)])
            } else {
                String::new()
            }
        ));
        data.push(serde_json::json!({
            "task_id": t.task_id.to_string(),
            "role": t.role,
            "status": status_str,
            "elapsed_secs": elapsed,
            "has_result": t.result.is_some(),
            "depends_on": t.depends_on.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
        }));
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: lines.join("\n"),
        is_error: false,
        data: Some(serde_json::json!({ "tasks": data })),
    }
}

// ─── Scheduler Tool Implementation ─────────────────────────────────────────

/// Persist a ScheduledTask to the memory database.
fn persist_task_to_db(mem: &claw_memory::MemoryStore, task: &crate::scheduler::ScheduledTask) {
    let kind_json = serde_json::to_string(&task.kind).unwrap_or_default();
    let created_at = task.created_at.to_rfc3339();
    let last_fired = task.last_fired.map(|t| t.to_rfc3339());
    if let Err(e) = mem.persist_scheduled_task(
        &task.id.to_string(),
        task.label.as_deref(),
        &task.description,
        &kind_json,
        &created_at,
        task.session_id.as_ref().map(|s| s.to_string()).as_deref(),
        task.active,
        task.fire_count,
        last_fired.as_deref(),
    ) {
        warn!(task_id = %task.id, error = %e, "failed to persist scheduled task to DB");
    }
}

/// Schedule a recurring cron or one-shot delayed task.
async fn exec_cron_schedule(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let description = match call.arguments.get("description").and_then(|v| v.as_str()) {
        Some(d) => d.to_string(),
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'description' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let cron_expr = call
        .arguments
        .get("cron_expr")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let delay_seconds = call.arguments.get("delay_seconds").and_then(|v| v.as_u64());
    let label = call
        .arguments
        .get("label")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let scheduler = match &state.scheduler {
        Some(s) => s,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: scheduler is not available".into(),
                is_error: true,
                data: None,
            };
        }
    };

    if let Some(cron) = cron_expr {
        // Recurring cron task
        match scheduler
            .add_cron(description.clone(), &cron, label.clone(), None)
            .await
        {
            Ok(task_id) => {
                // Persist to SQLite
                if let Some(task) = scheduler.get(task_id).await {
                    let mem = state.memory.lock().await;
                    persist_task_to_db(&mem, &task);
                }
                ToolResult {
                    tool_call_id: call.id.clone(),
                    content: format!(
                        "Recurring task scheduled.\n\
                         Task ID: {}\n\
                         Cron: {}\n\
                         Label: {}\n\
                         Description: {}",
                        task_id,
                        cron,
                        label.unwrap_or_else(|| "none".to_string()),
                        description,
                    ),
                    is_error: false,
                    data: Some(serde_json::json!({
                        "task_id": task_id.to_string(),
                        "type": "cron",
                        "cron_expr": cron,
                    })),
                }
            }
            Err(e) => ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Error scheduling cron task: {}", e),
                is_error: true,
                data: None,
            },
        }
    } else if let Some(delay) = delay_seconds {
        // One-shot delayed task
        let task_id = scheduler
            .add_one_shot(description.clone(), delay, label.clone(), None)
            .await;
        // Persist to SQLite
        if let Some(task) = scheduler.get(task_id).await {
            let mem = state.memory.lock().await;
            persist_task_to_db(&mem, &task);
        }
        ToolResult {
            tool_call_id: call.id.clone(),
            content: format!(
                "One-shot task scheduled.\n\
                 Task ID: {}\n\
                 Fires in: {}s\n\
                 Label: {}\n\
                 Description: {}",
                task_id,
                delay,
                label.unwrap_or_else(|| "none".to_string()),
                description,
            ),
            is_error: false,
            data: Some(serde_json::json!({
                "task_id": task_id.to_string(),
                "type": "one_shot",
                "delay_seconds": delay,
            })),
        }
    } else {
        ToolResult {
            tool_call_id: call.id.clone(),
            content: "Error: must provide either 'cron_expr' or 'delay_seconds'".into(),
            is_error: true,
            data: None,
        }
    }
}

async fn exec_cron_list(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let scheduler = match &state.scheduler {
        Some(s) => s,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: scheduler is not available".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let tasks = scheduler.list_all().await;
    if tasks.is_empty() {
        return ToolResult {
            tool_call_id: call.id.clone(),
            content: "No scheduled tasks.".into(),
            is_error: false,
            data: Some(serde_json::json!({ "tasks": [] })),
        };
    }

    let mut lines = Vec::new();
    let mut active_count = 0u32;
    let mut json_tasks = Vec::new();
    for task in &tasks {
        let kind_str = match &task.kind {
            crate::scheduler::ScheduleKind::Cron { expression } => format!("cron: {}", expression),
            crate::scheduler::ScheduleKind::OneShot { fire_at } => {
                format!("one-shot: {}", fire_at.format("%Y-%m-%d %H:%M:%S UTC"))
            }
        };
        let status = if task.active { "active" } else { "inactive" };
        if task.active {
            active_count += 1;
        }
        let label_str = task.label.as_deref().unwrap_or("(none)");
        lines.push(format!(
            "• [{}] {} | {} | label: {} | fires: {} | desc: {}",
            status, task.id, kind_str, label_str, task.fire_count, task.description
        ));
        json_tasks.push(serde_json::json!({
            "id": task.id.to_string(),
            "label": task.label,
            "description": task.description,
            "kind": kind_str,
            "active": task.active,
            "fire_count": task.fire_count,
            "last_fired": task.last_fired.map(|t| t.to_rfc3339()),
            "created_at": task.created_at.to_rfc3339(),
        }));
    }

    ToolResult {
        tool_call_id: call.id.clone(),
        content: format!(
            "{} task(s) ({} active):\n{}",
            tasks.len(),
            active_count,
            lines.join("\n")
        ),
        is_error: false,
        data: Some(serde_json::json!({ "tasks": json_tasks })),
    }
}

async fn exec_cron_cancel(state: &SharedAgentState, call: &ToolCall) -> ToolResult {
    let task_id_str = match call.arguments.get("task_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: missing 'task_id' argument".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let task_id = match uuid::Uuid::parse_str(task_id_str) {
        Ok(id) => id,
        Err(_) => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("Error: invalid UUID: {}", task_id_str),
                is_error: true,
                data: None,
            };
        }
    };

    let scheduler = match &state.scheduler {
        Some(s) => s,
        None => {
            return ToolResult {
                tool_call_id: call.id.clone(),
                content: "Error: scheduler is not available".into(),
                is_error: true,
                data: None,
            };
        }
    };

    let removed = scheduler.remove(task_id).await;
    if removed {
        // Remove from SQLite
        let mem = state.memory.lock().await;
        let _ = mem.delete_scheduled_task(&task_id.to_string());
        ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("Task {} cancelled and removed.", task_id),
            is_error: false,
            data: Some(serde_json::json!({ "task_id": task_id.to_string(), "removed": true })),
        }
    } else {
        ToolResult {
            tool_call_id: call.id.clone(),
            content: format!("Task {} not found.", task_id),
            is_error: true,
            data: None,
        }
    }
}

/// Send a response back through the appropriate channel.
async fn send_response_shared(
    state: &SharedAgentState,
    channel_id: &str,
    target: &str,
    text: &str,
) -> claw_core::Result<()> {
    let channels = state.channels.lock().await;
    for channel in channels.iter() {
        if channel.id() == channel_id {
            channel
                .send(OutgoingMessage {
                    channel: channel_id.to_string(),
                    target: target.to_string(),
                    text: text.to_string(),
                    attachments: vec![],
                    reply_to: None,
                })
                .await?;
            return Ok(());
        }
    }
    warn!(channel = channel_id, "channel not found for response");
    Ok(())
}

/// Send a typing indicator to a specific channel target.
async fn send_typing_to_channel(state: &SharedAgentState, channel_id: &str, target: &str) {
    let channels = state.channels.lock().await;
    for channel in channels.iter() {
        if channel.id() == channel_id {
            let _ = channel.send_typing(target).await;
            return;
        }
    }
}

/// Map a tool name to an emoji for channel progress messages.
fn tool_progress_emoji(name: &str) -> &'static str {
    if name.starts_with("browser_") {
        return "🌐";
    }
    if name.starts_with("android_") || name.starts_with("ios_") {
        return "📱";
    }
    match name {
        "shell_exec" => "⚡",
        "file_write" | "file_create" | "file_patch" => "📝",
        "file_read" | "directory_list" | "file_list" | "file_find" => "📖",
        "process_start" | "terminal_run" => "🚀",
        "web_search" | "brave_search" => "🔍",
        "memory_store" | "memory_search" | "memory_forget" => "🧠",
        "goal_create" | "goal_update" => "🎯",
        "mesh_delegate" => "🌐",
        "channel_send_file" => "📎",
        _ => "🔧",
    }
}

/// Build a human-readable description of a tool call from its name and arguments.
fn describe_tool_call(name: &str, args: &serde_json::Value) -> String {
    match name {
        "shell_exec" => {
            let cmd = args["command"].as_str().unwrap_or("…");
            // Truncate long commands but keep the first useful part
            let short: String = cmd.chars().take(60).collect();
            if cmd.len() > 60 {
                format!("`{}…`", short.trim())
            } else {
                format!("`{}`", short.trim())
            }
        }
        "file_write" | "file_create" => {
            let path = args["path"]
                .as_str()
                .or_else(|| args["file_path"].as_str())
                .unwrap_or("…");
            // Show just the filename or last 2 path components
            let short = short_path(path);
            format!("Writing `{}`", short)
        }
        "file_patch" => {
            let path = args["path"]
                .as_str()
                .or_else(|| args["file_path"].as_str())
                .unwrap_or("…");
            format!("Editing `{}`", short_path(path))
        }
        "file_read" => {
            let path = args["path"]
                .as_str()
                .or_else(|| args["file_path"].as_str())
                .unwrap_or("…");
            format!("Reading `{}`", short_path(path))
        }
        "file_list" | "directory_list" | "file_find" => {
            let path = args["path"]
                .as_str()
                .or_else(|| args["directory"].as_str())
                .unwrap_or("…");
            format!("Exploring `{}`", short_path(path))
        }
        "process_start" | "terminal_run" => {
            let cmd = args["command"].as_str().unwrap_or("…");
            let short: String = cmd.chars().take(50).collect();
            format!("Starting `{}`", short.trim())
        }
        "terminal_view" => {
            let id = args["terminal_id"]
                .as_str()
                .or_else(|| args["id"].as_str())
                .unwrap_or("terminal");
            format!("Checking {}", id)
        }
        "web_search" | "brave_search" => {
            let q = args["query"].as_str().unwrap_or("…");
            let short: String = q.chars().take(40).collect();
            format!("Searching \"{}\"", short.trim())
        }
        n if n.starts_with("browser_") => {
            let action = n.strip_prefix("browser_").unwrap_or(n);
            match action {
                "navigate" => {
                    let url = args["url"].as_str().unwrap_or("…");
                    let short: String = url.chars().take(50).collect();
                    format!("Opening `{}`", short)
                }
                "click" => {
                    let sel = args["selector"].as_str().unwrap_or("element");
                    let short: String = sel.chars().take(30).collect();
                    format!("Clicking `{}`", short)
                }
                "type" | "input" => format!("Typing text"),
                "screenshot" | "snapshot" => format!("Taking screenshot"),
                "upload_file" => format!("Uploading file"),
                _ => format!("{}", action.replace('_', " ")),
            }
        }
        n if n.starts_with("android_") || n.starts_with("ios_") => {
            let parts: Vec<&str> = n.splitn(2, '_').collect();
            let action = parts.get(1).unwrap_or(&n);
            format!("{}", action.replace('_', " "))
        }
        "memory_store" => format!("Storing memory"),
        "memory_search" => format!("Searching memory"),
        "goal_create" => {
            let desc = args["description"].as_str().unwrap_or("…");
            let short: String = desc.chars().take(40).collect();
            format!("Goal: {}", short.trim())
        }
        "mesh_delegate" => format!("Delegating to peer"),
        "channel_send_file" => {
            let path = args["file_path"].as_str().unwrap_or("…");
            format!("Sending `{}`", short_path(path))
        }
        _ => format!("{}", name.replace('_', " ")),
    }
}

/// Shorten a file path to the last 2 components (e.g. `src/app/page.tsx`).
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.rsplit('/').take(3).collect();
    let mut result: Vec<&str> = parts.into_iter().rev().collect();
    // Skip empty leading component from absolute paths
    if result.first() == Some(&"") {
        result.remove(0);
    }
    result.join("/")
}

/// Extract a brief, human-readable summary from a tool result.
/// Skips boilerplate lines (Exit code, STDOUT/STDERR headers) and returns
/// the first meaningful line of content, truncated to `max_len` chars.
fn extract_result_summary(content: &str, max_len: usize) -> String {
    let skip = |line: &str| -> bool {
        let trimmed = line.trim();
        trimmed.is_empty()
            || trimmed.starts_with("Exit code:")
            || trimmed == "STDOUT:"
            || trimmed == "STDERR:"
            || trimmed == "Command completed successfully (no output)."
    };

    for line in content.lines() {
        if !skip(line) {
            let trimmed = line.trim();
            if trimmed.len() > max_len {
                return format!("{}…", &trimmed[..max_len - 1]);
            }
            return trimmed.to_string();
        }
    }
    String::new()
}

/// Send a message through a channel and return its platform message ID.
async fn send_channel_message_returning_id(
    state: &SharedAgentState,
    channel_id: &str,
    target: &str,
    text: &str,
) -> Option<String> {
    let channels = state.channels.lock().await;
    for channel in channels.iter() {
        if channel.id() == channel_id {
            return channel
                .send_returning_id(OutgoingMessage {
                    channel: channel_id.to_string(),
                    target: target.to_string(),
                    text: text.to_string(),
                    attachments: vec![],
                    reply_to: None,
                })
                .await
                .ok()
                .flatten();
        }
    }
    None
}

/// Edit a previously sent message on a channel.
async fn edit_channel_message(
    state: &SharedAgentState,
    channel_id: &str,
    target: &str,
    message_id: &str,
    text: &str,
) -> claw_core::Result<()> {
    let channels = state.channels.lock().await;
    for channel in channels.iter() {
        if channel.id() == channel_id {
            return channel.edit_message(target, message_id, text).await;
        }
    }
    Ok(())
}

/// Resolve a pending approval (from callback query, /approve command, or API).
async fn resolve_approval(
    pending: &PendingApprovals,
    id: Uuid,
    approve: bool,
) -> Result<(), String> {
    let tx = pending
        .lock()
        .await
        .remove(&id)
        .ok_or_else(|| "Approval not found or already resolved.".to_string())?;
    let response = if approve {
        ApprovalResponse::Approved
    } else {
        ApprovalResponse::Denied
    };
    let _ = tx.send(response);
    info!(id = %id, approved = approve, "approval resolved via channel");
    Ok(())
}

/// Send an approval prompt to a channel (uses inline keyboard for Telegram, text fallback for others).
async fn send_approval_prompt_shared(
    state: &SharedAgentState,
    channel_id: &str,
    target: &str,
    approval_id: &str,
    tool_name: &str,
    tool_args: &serde_json::Value,
    reason: &str,
    risk_level: u8,
) {
    let prompt = ApprovalPrompt {
        approval_id: approval_id.to_string(),
        target: target.to_string(),
        tool_name: tool_name.to_string(),
        tool_args: tool_args.clone(),
        reason: reason.to_string(),
        risk_level,
    };

    let channels = state.channels.lock().await;
    for channel in channels.iter() {
        if channel.id() == channel_id {
            if let Err(e) = channel.send_approval_prompt(prompt).await {
                warn!(error = %e, channel = channel_id, "failed to send approval prompt");
            }
            return;
        }
    }

    // If the channel isn't found (e.g. "api"), the approval will still work via the API endpoint
    debug!(
        channel = channel_id,
        "no channel found for approval prompt (API-only approval)"
    );
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

/// Build a brief episodic summary from the conversation messages.
fn build_episode_summary(messages: &[Message], user_text: &str, final_response: &str) -> String {
    // Count tool calls across all messages
    let tool_names: Vec<String> = messages
        .iter()
        .flat_map(|m| m.tool_calls.iter().map(|tc| tc.tool_name.clone()))
        .collect();
    let tool_count = tool_names.len();

    let user_preview: String = user_text.chars().take(120).collect();
    let response_preview: String = final_response.chars().take(200).collect();

    let mut summary = format!("User: {}", user_preview);
    if tool_count > 0 {
        let unique_tools: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            tool_names
                .into_iter()
                .filter(|t| seen.insert(t.clone()))
                .collect()
        };
        summary.push_str(&format!(
            " | Tools used ({}): {}",
            tool_count,
            unique_tools.join(", ")
        ));
    }
    if !response_preview.is_empty() {
        summary.push_str(&format!(" | Response: {}", response_preview));
    }
    summary
}

/// Extract simple keyword tags from user text for episodic search.
fn extract_episode_tags(user_text: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let lower = user_text.to_lowercase();

    // Detect broad task categories
    let categories = [
        (
            "code",
            &[
                "code",
                "program",
                "function",
                "bug",
                "fix",
                "implement",
                "build",
                "compile",
            ][..],
        ),
        (
            "file",
            &["file", "create", "write", "read", "edit", "delete"][..],
        ),
        (
            "web",
            &[
                "website",
                "web",
                "html",
                "css",
                "javascript",
                "react",
                "next",
            ][..],
        ),
        (
            "research",
            &["research", "search", "find", "look up", "explain"][..],
        ),
        (
            "deploy",
            &["deploy", "docker", "server", "host", "production"][..],
        ),
        ("config", &["config", "setup", "install", "configure"][..]),
        ("goal", &["goal", "plan", "task", "project"][..]),
    ];

    for (tag, keywords) in &categories {
        if keywords.iter().any(|kw| lower.contains(kw)) {
            tags.push(tag.to_string());
        }
    }

    // Cap at 5 tags
    tags.truncate(5);
    tags
}

/// Extract meaningful search keywords from user text for memory retrieval.
/// Strips common filler words and short words, keeps domain-specific terms.
fn extract_search_keywords(user_text: &str) -> String {
    // Common stop words to filter out
    const STOP_WORDS: &[&str] = &[
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "shall", "should", "may", "might", "must", "can",
        "could", "i", "me", "my", "myself", "we", "our", "ours", "you", "your", "yours", "he",
        "him", "his", "she", "her", "hers", "it", "its", "they", "them", "their", "what", "which",
        "who", "whom", "this", "that", "these", "those", "am", "and", "but", "or", "nor", "not",
        "no", "so", "if", "then", "than", "too", "very", "just", "don", "now", "here", "there",
        "how", "all", "each", "every", "both", "few", "more", "most", "some", "any", "such",
        "only", "own", "same", "also", "into", "from", "with", "for", "on", "at", "to", "of", "in",
        "by", "up", "about", "out", "off", "over", "under", "again", "further", "once", "where",
        "when", "why", "after", "before", "above", "below", "between", "please", "want", "need",
        "help", "like", "make", "let", "get", "know", "think", "tell", "show", "give", "use",
        // Dutch stop words (since user communicates in Dutch)
        "de", "het", "een", "en", "van", "ik", "te", "dat", "die", "er", "zijn", "op", "aan", "met",
        "als", "voor", "nog", "maar", "om", "ook", "dan", "wel", "niet", "wat", "kun", "je", "kan",
        "naar", "hoe", "dit", "bij", "uit", "zo", "mijn", "jij", "wij", "zij",
    ];

    let lower = user_text.to_lowercase();
    let keywords: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.')
        .filter(|w| w.len() >= 3)
        .filter(|w| !STOP_WORDS.contains(w))
        .collect();

    // Also preserve URLs, email-like patterns, and dotted identifiers from original
    let mut extra: Vec<&str> = Vec::new();
    for word in user_text.split_whitespace() {
        if word.contains("://") || word.contains('.') && word.len() > 5 {
            extra.push(word);
        }
    }

    let mut combined: Vec<String> = keywords.iter().map(|s| s.to_string()).collect();
    for e in extra {
        let el = e.to_lowercase();
        if !combined.contains(&el) {
            combined.push(el);
        }
    }

    combined.join(" ")
}

// ── Self-Learning: Lesson Detection & Extraction ─────────────────────────

/// Detect whether the conversation contains error→correction→success patterns
/// that indicate a lesson was learned. Returns true if lesson extraction should run.
fn detect_lesson_patterns(messages: &[Message]) -> bool {
    // We look for patterns like:
    //   1. Tool result with is_error=true (something failed)
    //   2. Followed by a User message (correction/guidance)
    //   3. Followed by successful tool calls or assistant response
    //
    // OR:
    //   1. Assistant refuses/says it can't do something
    //   2. User corrects/insists
    //   3. Assistant succeeds

    let mut saw_error_or_refusal = false;
    let mut saw_user_correction_after = false;
    let mut saw_success_after = false;

    for msg in messages {
        match msg.role {
            Role::Tool => {
                for content in &msg.content {
                    if let claw_core::MessageContent::ToolResult { is_error, .. } = content {
                        if *is_error {
                            saw_error_or_refusal = true;
                            saw_user_correction_after = false;
                            saw_success_after = false;
                        }
                    }
                }
            }
            Role::Assistant => {
                let text = msg.text_content().to_lowercase();
                // Detect refusal patterns
                if text.contains("kan niet")
                    || text.contains("cannot")
                    || text.contains("can't")
                    || text.contains("mag niet")
                    || text.contains("unable to")
                    || text.contains("not able")
                    || text.contains("not allowed")
                    || text.contains("weiger")
                {
                    saw_error_or_refusal = true;
                    saw_user_correction_after = false;
                    saw_success_after = false;
                }
                // If we had an error + user correction, and now assistant acts, it's a success
                if saw_user_correction_after && !msg.tool_calls.is_empty() {
                    saw_success_after = true;
                }
            }
            Role::User => {
                if saw_error_or_refusal {
                    saw_user_correction_after = true;
                }
            }
            _ => {}
        }
    }

    // Pattern detected: error/refusal → user correction → continued work
    saw_error_or_refusal && (saw_user_correction_after || saw_success_after)
}

/// Build a conversation excerpt focusing on the error→correction→success patterns
/// for the LLM to analyze. Keeps it concise to minimize token usage.
fn build_lesson_excerpt(messages: &[Message]) -> String {
    let mut excerpt = String::new();
    let mut interesting_range = false;

    for msg in messages {
        let role_str = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
            Role::System => continue, // skip system messages
        };

        let text = msg.text_content();
        let is_error_result = msg.content.iter().any(
            |c| matches!(c, claw_core::MessageContent::ToolResult { is_error, .. } if *is_error),
        );

        // Start including messages when we hit an error
        if is_error_result {
            interesting_range = true;
        }
        let text_lower = text.to_lowercase();
        if msg.role == Role::Assistant
            && (text_lower.contains("kan niet")
                || text_lower.contains("cannot")
                || text_lower.contains("can't")
                || text_lower.contains("unable")
                || text_lower.contains("not allowed")
                || text_lower.contains("weiger"))
        {
            interesting_range = true;
        }

        if interesting_range {
            let truncated: String = text.chars().take(500).collect();
            if !truncated.is_empty() {
                excerpt.push_str(&format!("[{}]: {}\n", role_str, truncated));
            }
            for tc in &msg.tool_calls {
                let args_preview: String = tc.arguments.to_string().chars().take(200).collect();
                excerpt.push_str(&format!(
                    "[Tool Call]: {}({})\n",
                    tc.tool_name, args_preview
                ));
            }
        }
    }

    // Cap the excerpt at ~4000 chars to keep the LLM call cheap
    excerpt.chars().take(4000).collect()
}

/// Use the LLM to extract lessons from a conversation where errors were corrected.
/// Returns a list of (key, lesson_text) tuples to store in semantic memory.
async fn extract_lessons_via_llm(
    state: &SharedAgentState,
    messages: &[Message],
) -> Vec<(String, String)> {
    let excerpt = build_lesson_excerpt(messages);
    if excerpt.is_empty() {
        return vec![];
    }

    let prompt = format!(
        "Analyze this conversation excerpt where mistakes were made and then corrected. \
         Extract specific, actionable lessons learned.\n\n\
         For each lesson, output a JSON array of objects with:\n\
         - \"key\": a short snake_case identifier (e.g. \"plesk_login_needs_cookie_accept\")\n\
         - \"lesson\": a concise description of what was learned, including the correct approach\n\n\
         Focus on:\n\
         - Technical procedures that were figured out through trial and error\n\
         - Form fields that needed specific values (IDs vs text, hidden fields, etc.)\n\
         - Steps that must be done in a specific order\n\
         - Common errors and their solutions\n\
         - Anything the user had to correct or point out\n\n\
         Output ONLY the JSON array, nothing else. If no clear lessons, output [].\n\n\
         Conversation excerpt:\n{}\n",
        excerpt
    );

    // Use fast model if available to keep costs low
    let model = state
        .config
        .agent
        .fast_model
        .as_deref()
        .unwrap_or(&state.config.agent.model);

    let request = LlmRequest {
        model: model.to_string(),
        messages: vec![Message::text(Uuid::nil(), Role::User, &prompt)],
        tools: vec![],
        system: Some("You are a precise lesson extractor. Output only valid JSON.".to_string()),
        max_tokens: 1024,
        temperature: 0.2,
        thinking_level: Some("off".to_string()),
        stream: false,
    };

    match state.llm.complete(&request, None).await {
        Ok(response) => {
            let text = response.message.text_content();
            // Parse JSON from the response (handle markdown code fences)
            let json_text = text
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();

            match serde_json::from_str::<Vec<serde_json::Value>>(json_text) {
                Ok(items) => items
                    .iter()
                    .filter_map(|item| {
                        let key = item["key"].as_str()?.to_string();
                        let lesson = item["lesson"].as_str()?.to_string();
                        Some((key, lesson))
                    })
                    .collect(),
                Err(e) => {
                    debug!(error = %e, "failed to parse lesson extraction JSON");
                    vec![]
                }
            }
        }
        Err(e) => {
            debug!(error = %e, "lesson extraction LLM call failed");
            vec![]
        }
    }
}

/// Run the full self-learning pipeline: detect patterns, extract lessons, persist them.
async fn maybe_extract_lessons(state: &SharedAgentState, session_id: Uuid) {
    // Read messages — brief lock
    let messages = {
        let mem = state.memory.lock().await;
        mem.working.messages(session_id).to_vec()
    };

    // Only extract if there are error→correction patterns
    if !detect_lesson_patterns(&messages) {
        return;
    }

    info!(session = %session_id, "detected error→correction pattern, extracting lessons");

    let lessons = extract_lessons_via_llm(state, &messages).await;
    if lessons.is_empty() {
        return;
    }

    info!(session = %session_id, count = lessons.len(), "extracted lessons from conversation");

    // Persist each lesson as a semantic fact
    let mut mem = state.memory.lock().await;
    for (key, lesson) in &lessons {
        let fact = claw_memory::semantic::Fact {
            id: Uuid::new_v4(),
            category: "learned_lessons".to_string(),
            key: key.clone(),
            value: lesson.clone(),
            confidence: 0.9,
            source: Some(format!("session:{}", session_id)),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        mem.semantic.upsert(fact);
        if let Err(e) = mem.persist_fact("learned_lessons", key, lesson) {
            warn!(error = %e, key = key, "failed to persist lesson to SQLite");
        }
    }
    drop(mem);

    // Generate embeddings for the lessons if embedder is available
    if let Some(ref embedder) = state.embedder {
        let texts: Vec<String> = lessons
            .iter()
            .map(|(key, lesson)| format!("learned_lessons {} {}", key, lesson))
            .collect();
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        if let Ok(embeddings) = embedder.embed(&text_refs).await {
            let mem = state.memory.lock().await;
            for (i, (key, _lesson)) in lessons.iter().enumerate() {
                if let Some(emb) = embeddings.get(i) {
                    // Re-persist with embedding
                    if let Some(fact) = mem.semantic.get("learned_lessons", key) {
                        let _ = mem.persist_fact_with_embedding(
                            "learned_lessons",
                            key,
                            &fact.value,
                            Some(emb),
                        );
                    }
                }
            }
        }
    }

    // Broadcast lessons to mesh peers
    {
        let mesh = state.mesh.lock().await;
        if mesh.is_running() && mesh.peer_count() > 0 {
            for (key, lesson) in &lessons {
                let sync_msg = MeshMessage::SyncDelta {
                    peer_id: mesh.peer_id().to_string(),
                    delta_type: "fact".to_string(),
                    data: serde_json::json!({
                        "category": "learned_lessons",
                        "key": key,
                        "value": lesson,
                        "confidence": 0.9,
                    }),
                };
                let _ = mesh.broadcast(&sync_msg).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claw_llm::mock::MockProvider;

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
        assert!(sessions.len() >= 1, "expected at least one session");
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
                StreamEvent::Error { message } => panic!("unexpected error: {}", message),
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
