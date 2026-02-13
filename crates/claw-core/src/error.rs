use thiserror::Error;

/// Unified error type for the entire Claw runtime.
#[derive(Error, Debug)]
pub enum ClawError {
    // ── Agent errors ───────────────────────────────────────────
    #[error("agent error: {0}")]
    Agent(String),

    #[error("planning failed: {0}")]
    Planning(String),

    #[error("goal not achievable: {0}")]
    GoalUnachievable(String),

    // ── LLM errors ─────────────────────────────────────────────
    #[error("llm provider error: {0}")]
    LlmProvider(String),

    #[error("llm rate limited, retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("llm context window exceeded: used {used} of {max} tokens")]
    ContextOverflow { used: usize, max: usize },

    #[error("model not found: {0}")]
    ModelNotFound(String),

    // ── Tool errors ────────────────────────────────────────────
    #[error("tool not found: {0}")]
    ToolNotFound(String),

    #[error("tool execution failed: {tool}: {reason}")]
    ToolExecution { tool: String, reason: String },

    #[error("tool denied by guardrail: {tool}: {reason}")]
    ToolDenied { tool: String, reason: String },

    // ── Plugin errors ──────────────────────────────────────────
    #[error("plugin error: {plugin}: {reason}")]
    Plugin { plugin: String, reason: String },

    #[error("plugin sandbox violation: {0}")]
    SandboxViolation(String),

    // ── Channel errors ─────────────────────────────────────────
    #[error("channel error: {channel}: {reason}")]
    Channel { channel: String, reason: String },

    #[error("channel not connected: {0}")]
    ChannelNotConnected(String),

    // ── Memory errors ──────────────────────────────────────────
    #[error("memory error: {0}")]
    Memory(String),

    // ── Mesh / networking errors ───────────────────────────────
    #[error("mesh peer unreachable: {0}")]
    PeerUnreachable(String),

    #[error("mesh sync conflict: {0}")]
    SyncConflict(String),

    // ── Config errors ──────────────────────────────────────────
    #[error("config error: {0}")]
    Config(String),

    #[error("config validation failed: {field}: {reason}")]
    ConfigValidation { field: String, reason: String },

    // ── Autonomy / guardrail errors ────────────────────────────
    #[error("autonomy level insufficient: requires L{required}, current L{current}")]
    AutonomyInsufficient { required: u8, current: u8 },

    #[error("budget exceeded: {resource}: used {used}, limit {limit}")]
    BudgetExceeded {
        resource: String,
        used: f64,
        limit: f64,
    },

    #[error("human approval required: {0}")]
    HumanApprovalRequired(String),

    // ── Generic wrappers ───────────────────────────────────────
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, ClawError>;
