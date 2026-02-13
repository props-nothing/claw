use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Root configuration — maps to `claw.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClawConfig {
    pub agent: AgentConfig,
    pub autonomy: AutonomyConfig,
    pub memory: MemoryConfig,
    pub channels: HashMap<String, ChannelConfig>,
    pub mesh: MeshConfig,
    pub plugins: PluginsConfig,
    pub server: ServerConfig,
    pub logging: LoggingConfig,
    pub credentials: CredentialsConfig,
    pub services: ServicesConfig,
}

// ── Agent ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Primary model identifier, e.g. "anthropic/claude-opus-4-6".
    pub model: String,
    /// Fallback model for when primary is unavailable.
    pub fallback_model: Option<String>,
    /// Small/fast model for classification, routing, summaries.
    pub fast_model: Option<String>,
    /// Local model for offline / low-latency tasks.
    pub local_model: Option<String>,
    /// System prompt injected at the start of every conversation.
    pub system_prompt: Option<String>,
    /// Path to a file containing the system prompt (overrides `system_prompt`).
    pub system_prompt_file: Option<PathBuf>,
    /// Maximum tokens per response.
    pub max_tokens: u32,
    /// Temperature (0.0 - 2.0).
    pub temperature: f32,
    /// Maximum agent loop iterations before forcing a stop.
    pub max_iterations: u32,
    /// Maximum concurrent tool executions.
    pub max_parallel_tools: u32,
    /// Thinking / reasoning budget ("off", "low", "medium", "high", "xhigh").
    pub thinking_level: String,
    /// Context window size in tokens. If 0, auto-detected from model name.
    pub context_window: usize,
    /// Maximum tokens per tool result. Longer results are truncated with a note.
    /// Default: 12000 (~48KB of text). Set to 0 to disable truncation.
    pub tool_result_max_tokens: usize,
    /// When context usage exceeds this fraction (0.0–1.0), trigger compaction.
    /// Default: 0.75 (compact at 75% full).
    pub compaction_threshold: f64,
    /// Maximum wall-clock seconds per request before the agent loop is terminated.
    /// 0 = no timeout (rely only on max_iterations). Default: 300 (5 minutes).
    pub request_timeout_secs: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: "anthropic/claude-sonnet-4-20250514".into(),
            fallback_model: None,
            fast_model: Some("anthropic/claude-haiku-3-5".into()),
            local_model: None,
            system_prompt: None,
            system_prompt_file: None,
            max_tokens: 16384,
            temperature: 0.7,
            max_iterations: 50,
            max_parallel_tools: 8,
            thinking_level: "medium".into(),
            context_window: 0,
            tool_result_max_tokens: 12_000,
            compaction_threshold: 0.75,
            request_timeout_secs: 300,
        }
    }
}

// ── Autonomy ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutonomyConfig {
    /// Autonomy level: 0 = manual, 1 = assisted, 2 = supervised, 3 = autonomous, 4 = full auto.
    pub level: u8,
    /// Maximum USD spend per day across all LLM providers.
    pub daily_budget_usd: f64,
    /// Maximum number of tool calls per single agent loop.
    pub max_tool_calls_per_loop: u32,
    /// Maximum files that can be deleted in a single action.
    pub max_delete_files: u32,
    /// Tools that are always allowed regardless of autonomy level.
    pub tool_allowlist: Vec<String>,
    /// Tools that are always blocked.
    pub tool_denylist: Vec<String>,
    /// Actions above this risk level require human approval (0-10).
    pub approval_threshold: u8,
    /// Enable proactive heartbeat / background tasks.
    pub proactive: bool,
    /// Cron schedule for heartbeat checks (cron expression).
    pub heartbeat_cron: Option<String>,
    /// Goals the agent should autonomously pursue.
    pub goals: Vec<GoalConfig>,
}

impl Default for AutonomyConfig {
    fn default() -> Self {
        Self {
            level: 1,
            daily_budget_usd: 10.0,
            max_tool_calls_per_loop: 100,
            max_delete_files: 5,
            tool_allowlist: vec![],
            tool_denylist: vec![],
            approval_threshold: 7,
            proactive: false,
            heartbeat_cron: None,
            goals: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalConfig {
    pub description: String,
    pub priority: u8,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

// ── Memory ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Path to the SQLite database.
    pub db_path: PathBuf,
    /// Maximum number of episodic memories to retain.
    pub max_episodes: usize,
    /// Enable vector similarity search for semantic memory.
    pub vector_search: bool,
    /// Embedding dimensions (384 for MiniLM, 1536 for OpenAI, etc.)
    pub embedding_dims: usize,
    /// Auto-summarize conversations after this many messages.
    pub auto_summarize_after: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("memory.db"),
            max_episodes: 10_000,
            vector_search: true,
            embedding_dims: 384,
            auto_summarize_after: 50,
        }
    }
}

// ── Channels ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Channel adapter type: "telegram", "discord", "whatsapp", "slack", "signal", "webchat", etc.
    #[serde(rename = "type")]
    pub channel_type: String,
    /// Whether this channel is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// DM access policy: "pairing" (default), "allowlist", "open", "disabled".
    /// Pairing mode: unknown senders receive a code, approved via `claw channels approve`.
    #[serde(default = "default_dm_policy")]
    pub dm_policy: String,
    /// Allowed sender identifiers (phone numbers, user IDs, etc.)
    #[serde(default)]
    pub allow_from: Vec<String>,
    /// Adapter-specific settings (API keys, tokens, etc.)
    #[serde(flatten)]
    pub settings: HashMap<String, serde_json::Value>,
}

// ── Mesh ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MeshConfig {
    /// Enable mesh networking.
    pub enabled: bool,
    /// Listen address for the mesh transport.
    pub listen: String,
    /// Known bootstrap peers.
    pub bootstrap_peers: Vec<String>,
    /// Enable mDNS for local discovery.
    pub mdns: bool,
    /// Device capabilities to advertise.
    pub capabilities: Vec<String>,
    /// Pre-shared key for mesh encryption (in addition to noise protocol).
    pub psk: Option<String>,
}

impl Default for MeshConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen: "/ip4/0.0.0.0/tcp/0".into(),
            bootstrap_peers: vec![],
            mdns: true,
            capabilities: vec!["shell".into(), "browser".into()],
            psk: None,
        }
    }
}

// ── Plugins ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginsConfig {
    /// Directory containing WASM plugin files.
    pub plugin_dir: PathBuf,
    /// ClawHub registry URL.
    pub registry_url: String,
    /// Plugins to install / load on startup.
    pub install: Vec<PluginRef>,
    /// Global capability grants for plugins.
    pub default_capabilities: Vec<String>,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            plugin_dir: PathBuf::from("plugins"),
            registry_url: "https://registry.clawhub.com".into(),
            install: vec![],
            default_capabilities: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRef {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
}

// ── Server ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// HTTP/WebSocket listen address.
    pub listen: String,
    /// Enable the web UI.
    pub web_ui: bool,
    /// Optional API key for the control API.
    pub api_key: Option<String>,
    /// Enable CORS (for web UI development).
    pub cors: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:3700".into(),
            web_ui: true,
            api_key: None,
            cors: false,
        }
    }
}

// ── Logging ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    /// Log level: "trace", "debug", "info", "warn", "error".
    pub level: String,
    /// Output format: "pretty", "json", "compact".
    pub format: String,
    /// Log file path (None = stdout only).
    pub file: Option<PathBuf>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            format: "pretty".into(),
            file: None,
        }
    }
}

// ── Credentials ────────────────────────────────────────────────

/// Credential provider configuration.
/// Tells the agent how to retrieve passwords, API keys, and secrets
/// across sessions without storing them in plaintext.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CredentialsConfig {
    /// Credential provider: "none" or "1password".
    /// When set to "1password", the agent will use the `op` CLI to retrieve secrets.
    pub provider: String,
    /// 1Password service account token for headless / automated usage.
    /// Can also be set via OP_SERVICE_ACCOUNT_TOKEN environment variable.
    /// Not needed when 1Password desktop app is installed (uses biometric).
    pub service_account_token: Option<String>,
    /// Default 1Password vault to search (e.g. "Personal", "Servers").
    /// When set, `op` commands default to this vault.
    pub default_vault: Option<String>,
}

impl Default for CredentialsConfig {
    fn default() -> Self {
        Self {
            provider: "none".into(),
            service_account_token: None,
            default_vault: None,
        }
    }
}

// ── Services ───────────────────────────────────────────────────

/// External service API keys and configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServicesConfig {
    /// Anthropic API key — used for Claude models.
    /// Can also be set via ANTHROPIC_API_KEY environment variable.
    /// Config file takes priority over environment variable.
    pub anthropic_api_key: Option<String>,
    /// OpenAI API key — used for GPT models.
    /// Can also be set via OPENAI_API_KEY environment variable.
    /// Config file takes priority over environment variable.
    pub openai_api_key: Option<String>,
    /// Brave Search API key — get one free at https://api.search.brave.com/
    pub brave_api_key: Option<String>,
    /// URL of the central Skills Hub (e.g. "https://hub.claw.dev" or "http://192.168.1.50:3800").
    /// When set, `claw skill push/pull/search` and the web UI talk to this remote hub.
    /// Run `claw hub serve` to host your own hub.
    pub hub_url: Option<String>,
}

impl Default for ServicesConfig {
    fn default() -> Self {
        Self {
            anthropic_api_key: None,
            openai_api_key: None,
            brave_api_key: None,
            hub_url: None,
        }
    }
}

// ── Default for root ───────────────────────────────────────────

impl Default for ClawConfig {
    fn default() -> Self {
        Self {
            agent: AgentConfig::default(),
            autonomy: AutonomyConfig::default(),
            memory: MemoryConfig::default(),
            channels: HashMap::new(),
            mesh: MeshConfig::default(),
            plugins: PluginsConfig::default(),
            server: ServerConfig::default(),
            logging: LoggingConfig::default(),
            credentials: CredentialsConfig::default(),
            services: ServicesConfig::default(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_dm_policy() -> String {
    "pairing".into()
}

/// Resolve context window size for a model. If the user configured a specific
/// value, use that. Otherwise, infer from the model name.
pub fn resolve_context_window(config_value: usize, model: &str) -> usize {
    if config_value > 0 {
        return config_value;
    }
    let m = model.to_lowercase();
    // Claude models
    if m.contains("claude") {
        return 200_000;
    }
    // GPT-5 and variants
    if m.contains("gpt-5") || m.contains("gpt5") {
        return 1_000_000;
    }
    // GPT-4o and variants
    if m.contains("gpt-4o") || m.contains("gpt-4-turbo") {
        return 128_000;
    }
    // GPT-4 (original)
    if m.contains("gpt-4") {
        return 8_192;
    }
    // o1 / o3 reasoning models
    if m.contains("o1") || m.contains("o3") {
        return 200_000;
    }
    // GPT-3.5
    if m.contains("gpt-3.5") {
        return 16_385;
    }
    // Llama 3 models
    if m.contains("llama3") || m.contains("llama-3") {
        return 128_000;
    }
    // Mistral / Mixtral
    if m.contains("mistral") || m.contains("mixtral") {
        return 32_768;
    }
    // Gemini
    if m.contains("gemini") {
        return 1_000_000;
    }
    // DeepSeek
    if m.contains("deepseek") {
        return 128_000;
    }
    // Default fallback
    128_000
}

// ── Validation ─────────────────────────────────────────────────

/// A single config validation issue.
#[derive(Debug)]
pub struct ConfigWarning {
    pub field: String,
    pub message: String,
    pub severity: WarningSeverity,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningSeverity {
    Error,
    Warning,
    Info,
}

impl std::fmt::Display for ConfigWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let icon = match self.severity {
            WarningSeverity::Error => "❌",
            WarningSeverity::Warning => "⚠️ ",
            WarningSeverity::Info => "💡",
        };
        write!(f, "{} {}: {}", icon, self.field, self.message)?;
        if let Some(ref h) = self.hint {
            write!(f, "\n   ↳ {}", h)?;
        }
        Ok(())
    }
}

impl ClawConfig {
    /// Validate the config and return a list of warnings/errors.
    /// Returns `Err` with all messages joined if any severity is Error.
    pub fn validate(&self) -> Result<Vec<ConfigWarning>, String> {
        let mut warnings = Vec::new();

        // ── Agent model ───
        let model = &self.agent.model;
        if model.is_empty() {
            warnings.push(ConfigWarning {
                field: "agent.model".into(),
                message: "model is empty".into(),
                severity: WarningSeverity::Error,
                hint: Some("Set to e.g. 'anthropic/claude-sonnet-4-20250514' or 'openai/gpt-4o'".into()),
            });
        } else if !model.contains('/') {
            warnings.push(ConfigWarning {
                field: "agent.model".into(),
                message: format!("model '{}' should be in 'provider/model' format", model),
                severity: WarningSeverity::Warning,
                hint: Some("Use 'anthropic/claude-sonnet-4-20250514', 'openai/gpt-4o', or 'ollama/llama3'".into()),
            });
        }

        // ── Temperature ───
        if self.agent.temperature < 0.0 || self.agent.temperature > 2.0 {
            warnings.push(ConfigWarning {
                field: "agent.temperature".into(),
                message: format!("temperature {} is out of range", self.agent.temperature),
                severity: WarningSeverity::Error,
                hint: Some("Temperature must be between 0.0 and 2.0".into()),
            });
        }

        // ── Max tokens ───
        if self.agent.max_tokens == 0 {
            warnings.push(ConfigWarning {
                field: "agent.max_tokens".into(),
                message: "max_tokens is 0 — agent won't produce output".into(),
                severity: WarningSeverity::Error,
                hint: Some("Set to e.g. 8192".into()),
            });
        }

        // ── Thinking level ───
        let valid_thinking = ["off", "low", "medium", "high", "xhigh"];
        if !valid_thinking.contains(&self.agent.thinking_level.as_str()) {
            warnings.push(ConfigWarning {
                field: "agent.thinking_level".into(),
                message: format!("unknown thinking level '{}'", self.agent.thinking_level),
                severity: WarningSeverity::Warning,
                hint: Some(format!("Valid values: {}", valid_thinking.join(", "))),
            });
        }

        // ── Autonomy level ───
        if self.autonomy.level > 4 {
            warnings.push(ConfigWarning {
                field: "autonomy.level".into(),
                message: format!("level {} is invalid", self.autonomy.level),
                severity: WarningSeverity::Error,
                hint: Some("Valid levels: 0 (manual), 1 (assisted), 2 (supervised), 3 (autonomous), 4 (full auto)".into()),
            });
        } else if self.autonomy.level == 4 {
            warnings.push(ConfigWarning {
                field: "autonomy.level".into(),
                message: "level 4 (full auto) — agent can execute ANY tool without approval".into(),
                severity: WarningSeverity::Warning,
                hint: Some("Consider level 2 or 3 for safer operation".into()),
            });
        }

        // ── Budget ───
        if self.autonomy.daily_budget_usd <= 0.0 {
            warnings.push(ConfigWarning {
                field: "autonomy.daily_budget_usd".into(),
                message: "budget is zero or negative — agent cannot spend".into(),
                severity: WarningSeverity::Warning,
                hint: Some("Set to e.g. 10.0".into()),
            });
        } else if self.autonomy.daily_budget_usd > 500.0 {
            warnings.push(ConfigWarning {
                field: "autonomy.daily_budget_usd".into(),
                message: format!("daily budget is ${:.2} — this is very high", self.autonomy.daily_budget_usd),
                severity: WarningSeverity::Warning,
                hint: Some("Consider a lower limit to prevent runaway costs".into()),
            });
        }

        // ── Approval threshold ───
        if self.autonomy.approval_threshold > 10 {
            warnings.push(ConfigWarning {
                field: "autonomy.approval_threshold".into(),
                message: format!("threshold {} > 10 — all tools would need approval", self.autonomy.approval_threshold),
                severity: WarningSeverity::Warning,
                hint: Some("Risk scores range 0-10. A threshold of 7-8 is typical.".into()),
            });
        }

        // ── Server listen address ───
        if self.server.listen.is_empty() {
            warnings.push(ConfigWarning {
                field: "server.listen".into(),
                message: "listen address is empty".into(),
                severity: WarningSeverity::Error,
                hint: Some("Set to e.g. '127.0.0.1:3700'".into()),
            });
        } else if self.server.listen.starts_with("0.0.0.0") {
            warnings.push(ConfigWarning {
                field: "server.listen".into(),
                message: "binding to 0.0.0.0 — server is accessible from all interfaces".into(),
                severity: WarningSeverity::Warning,
                hint: Some("Use '127.0.0.1:3700' for local-only access, or set an api_key".into()),
            });
        }

        // ── API key ───
        if self.server.api_key.is_none() && self.server.listen.starts_with("0.0.0.0") {
            warnings.push(ConfigWarning {
                field: "server.api_key".into(),
                message: "no API key set while server is network-accessible".into(),
                severity: WarningSeverity::Warning,
                hint: Some("Set server.api_key to protect your agent".into()),
            });
        }

        // ── Logging format ───
        let valid_formats = ["pretty", "json", "compact"];
        if !valid_formats.contains(&self.logging.format.as_str()) {
            warnings.push(ConfigWarning {
                field: "logging.format".into(),
                message: format!("unknown log format '{}'", self.logging.format),
                severity: WarningSeverity::Warning,
                hint: Some(format!("Valid values: {}", valid_formats.join(", "))),
            });
        }

        // ── Logging level ───
        let valid_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_levels.contains(&self.logging.level.as_str()) {
            warnings.push(ConfigWarning {
                field: "logging.level".into(),
                message: format!("unknown log level '{}'", self.logging.level),
                severity: WarningSeverity::Warning,
                hint: Some(format!("Valid values: {}", valid_levels.join(", "))),
            });
        }

        // ── Channel types ───
        let valid_channel_types = ["telegram", "webchat", "discord", "slack", "whatsapp", "signal", "matrix", "imessage"];
        for (id, ch) in &self.channels {
            if !valid_channel_types.contains(&ch.channel_type.as_str()) {
                warnings.push(ConfigWarning {
                    field: format!("channels.{}.type", id),
                    message: format!("unknown channel type '{}'", ch.channel_type),
                    severity: WarningSeverity::Warning,
                    hint: Some(format!("Supported: {}", valid_channel_types.join(", "))),
                });
            }

            // Validate DM policy
            let valid_policies = ["pairing", "allowlist", "open", "disabled"];
            if !valid_policies.contains(&ch.dm_policy.as_str()) {
                warnings.push(ConfigWarning {
                    field: format!("channels.{}.dm_policy", id),
                    message: format!("unknown DM policy '{}'", ch.dm_policy),
                    severity: WarningSeverity::Warning,
                    hint: Some(format!("Valid: {}", valid_policies.join(", "))),
                });
            }
        }

        // Check for hard errors
        let errors: Vec<String> = warnings
            .iter()
            .filter(|w| w.severity == WarningSeverity::Error)
            .map(|w| format!("{}: {}", w.field, w.message))
            .collect();

        if !errors.is_empty() {
            return Err(format!("Configuration errors:\n  • {}", errors.join("\n  • ")));
        }

        Ok(warnings)
    }
}